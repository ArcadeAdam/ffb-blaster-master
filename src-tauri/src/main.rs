#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod devices;
mod ini_edit;
mod legacy;
mod profiles;

use devices::DeviceEntry;
use ini_edit::{apply_changes, has_mixed_or_unsupported_line_endings, parse, Change, IniEntry};
use profiles::{parse_profile, set_ffb_enabled};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

const INI_NAME: &str = "FFBBlaster.ini";
const MAX_DEPTH: usize = 12;

// ---------------------------------------------------------------- portability
// Everything the app remembers lives next to the .exe, so the whole thing can
// run from a USB stick or a tools folder with no install and no AppData.

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn settings_file() -> PathBuf {
    exe_dir().join("ffbblaster-master.settings.json")
}

fn presets_file() -> PathBuf {
    exe_dir().join("ffbblaster-master.presets.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AppSettings {
    root_path: String,
    compatibility_paths: Vec<String>,
    teknoparrot_path: String,
    default_device_guid: String,
    backup_on_save: bool,
    hide_gui_initialized_paths: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            root_path: String::new(),
            compatibility_paths: Vec::new(),
            teknoparrot_path: String::new(),
            default_device_guid: String::new(),
            backup_on_save: true,
            hide_gui_initialized_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
struct GameEntry {
    name: String,
    folder_name: String,
    named_by_profile: bool,
    folder: String,
    ini_path: String,
    read_only: bool,
}

#[derive(Debug, Serialize)]
struct IniFile {
    path: String,
    entries: Vec<IniEntry>,
    read_only: bool,
}

#[derive(Debug, Serialize)]
struct WriteReport {
    path: String,
    ok: bool,
    changed: bool,
    message: String,
    backup: String,
}

#[derive(Debug, Serialize)]
struct PreviewChange {
    key: String,
    old: Option<String>,
    new: String,
}

#[derive(Debug, Serialize)]
struct PreviewReport {
    path: String,
    name: String,
    ok: bool,
    message: String,
    changes: Vec<PreviewChange>,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileEntry {
    name: String,
    exe: String,
    dir: String,
    file: String,
    ffb_supported: bool,
    ffb_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameRootKind {
    Library,
    CompatibilityGame,
}

#[derive(Debug, Clone)]
struct GameRoot {
    path: PathBuf,
    kind: GameRootKind,
}

#[derive(Debug)]
struct ResolvedGameRoots {
    roots: Vec<GameRoot>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanResult {
    games: Vec<GameEntry>,
    warnings: Vec<String>,
}

fn unique_stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        name,
        std::process::id(),
        unique_stamp()
    ))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = sibling_temp_path(path);
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }

    use std::os::windows::ffi::OsStrExt;
    let source: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let e = std::io::Error::last_os_error();
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

fn valid_device_guid(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// `std::fs::canonicalize` uses Windows' verbatim `\\?\` prefix. Keep that
/// internally for scope checks, but do not expose it in paths shown by the UI.
fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.into_owned()
    }
}

fn normalized_path(value: &str) -> String {
    let mut value = value.trim().replace('/', "\\");
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        value = format!(r"\\{rest}");
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        value = rest.to_string();
    }
    while value.ends_with('\\') {
        value.pop();
    }
    value.to_ascii_lowercase()
}

fn path_key(path: &Path) -> String {
    normalized_path(&display_path(path))
}

fn normalized_is_within(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!(r"{root}\"))
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    normalized_is_within(&path, &root)
}

fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none() || (cfg!(windows) && path.components().count() <= 2)
}

fn canonical_game_roots(
    root: &str,
    compatibility_paths: &[String],
) -> Result<ResolvedGameRoots, String> {
    let primary_path = PathBuf::from(root.trim());
    if !primary_path.is_dir() {
        return Err(format!(
            "Games folder not found: {}",
            primary_path.display()
        ));
    }
    let primary = fs::canonicalize(&primary_path).map_err(|error| {
        format!(
            "Could not resolve games folder {}: {error}",
            primary_path.display()
        )
    })?;
    if is_filesystem_root(&primary) {
        return Err(format!(
            "Add your main game library folder, not the whole drive: {}",
            display_path(&primary)
        ));
    }
    let mut seen = HashSet::from([path_key(&primary)]);
    let mut roots = vec![GameRoot {
        path: primary,
        kind: GameRootKind::Library,
    }];
    let mut warnings = Vec::new();

    for configured in compatibility_paths {
        let configured = configured.trim();
        if configured.is_empty() {
            continue;
        }
        let path = PathBuf::from(configured);
        if !path.is_absolute() {
            warnings.push(format!(
                "Additional game folders must use a complete path: {}",
                path.display()
            ));
            continue;
        }
        if !path.is_dir() {
            warnings.push(format!(
                "Additional game folder not found: {}",
                path.display()
            ));
            continue;
        }
        let path = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(format!(
                    "Could not resolve additional game folder {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if is_filesystem_root(&path) {
            warnings.push(format!(
                "Add each individual game folder, not the whole drive: {}",
                display_path(&path)
            ));
            continue;
        }
        if seen.insert(path_key(&path)) {
            roots.push(GameRoot {
                path,
                kind: GameRootKind::CompatibilityGame,
            });
        }
    }
    Ok(ResolvedGameRoots { roots, warnings })
}

fn validate_ini_target(path: &Path) -> Result<(), String> {
    let is_expected_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case(INI_NAME))
        .unwrap_or(false);
    if !is_expected_name {
        return Err(format!("Refusing to write a file not named {INI_NAME}."));
    }
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }
    if is_read_only(path) {
        return Err("File is read-only. No changes were written.".into());
    }
    Ok(())
}

fn read_editable_ini(path: &Path) -> Result<String, String> {
    validate_ini_target(path)?;
    let content = fs::read_to_string(path).map_err(|e| {
        format!("Could not read this INI as UTF-8 text; no changes were written: {e}")
    })?;
    if has_mixed_or_unsupported_line_endings(&content) {
        return Err("Mixed or unsupported line endings detected; no changes were written.".into());
    }
    Ok(content)
}

fn validate_changes(changes: &[Change]) -> Result<(), String> {
    const BOOL_KEYS: &[&str] = &[
        "EnableDamper",
        "EnableRumble",
        "EnableRumbleTriggers",
        "AlternativeFFB",
        "UseAltConstantEffect",
        "ReverseRumble",
        "NetOutputsWithLF",
        "Logging",
        "BeepWhenHook",
        "ForceShowDeviceGUIDMessageBox",
        "disableInGameGui",
    ];
    const RANGED_KEYS: &[(&str, i64, i64)] = &[
        ("MinForce", 0, 100),
        ("MaxForce", 0, 100),
        ("FeedbackLength", 0, 1000),
        ("DefaultCentering", 0, 100),
        ("DefaultFriction", 0, 100),
        ("DamperStrength", 0, 100),
        ("AlternativeMinForceLeft", -100, 100),
        ("AlternativeMaxForceLeft", -100, 100),
        ("AlternativeMinForceRight", -100, 100),
        ("AlternativeMaxForceRight", -100, 100),
        ("OutputsSystem", 0, 20),
        ("MaxScaleOutput", 0, 255),
        ("NetOutputsTCPPort", 0, 65535),
        ("NetOutputsUDPBroadcastPort", 0, 65535),
    ];

    for change in changes {
        if !change.section.trim().eq_ignore_ascii_case("SETTINGS") {
            return Err("Only the SETTINGS section can be edited.".into());
        }
        let key = change.key.trim();
        if key.is_empty()
            || key.contains(['\r', '\n', '=', '[', ']'])
            || change.value.contains(['\r', '\n'])
        {
            return Err(format!("{key}: invalid key or value."));
        }
        let value = change.value.trim();
        if key.eq_ignore_ascii_case("DeviceGUID") && !valid_device_guid(value) {
            return Err("DeviceGUID must be exactly 32 hexadecimal characters.".into());
        }
        if BOOL_KEYS
            .iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
            && !matches!(value, "0" | "1")
        {
            return Err(format!("{key} must be 0 or 1."));
        }
        if let Some((_, minimum, maximum)) = RANGED_KEYS
            .iter()
            .find(|(candidate, _, _)| key.eq_ignore_ascii_case(candidate))
        {
            let number = value
                .parse::<i64>()
                .map_err(|_| format!("{key} must be a whole number."))?;
            if number < *minimum || number > *maximum {
                return Err(format!("{key} must be from {minimum} to {maximum}."));
            }
        }
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    atomic_write(path, json.as_bytes())
}

// ------------------------------------------------------------------- commands

#[tauri::command]
fn load_settings() -> AppSettings {
    fs::read_to_string(settings_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn save_settings(settings: AppSettings) -> Result<(), String> {
    write_json(&settings_file(), &settings)
}

#[tauri::command]
fn load_presets() -> serde_json::Value {
    fs::read_to_string(presets_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
fn save_presets(presets: serde_json::Value) -> Result<(), String> {
    write_json(&presets_file(), &presets)
}

fn is_read_only(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn game_entry(path: &Path) -> GameEntry {
    let folder_name = folder_name(path);
    GameEntry {
        name: folder_name.clone(),
        folder_name,
        named_by_profile: false,
        folder: display_path(path),
        ini_path: display_path(&path.join(INI_NAME)),
        read_only: is_read_only(&path.join(INI_NAME)),
    }
}

/// Walk `dir` looking for folders that contain FFBBlaster.ini. Stops descending
/// as soon as it finds one, so a game folder with sub-mods is listed once.
fn walk(dir: &Path, depth: usize, out: &mut Vec<GameEntry>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let ini = path.join(INI_NAME);
        if ini.is_file() {
            out.push(game_entry(&path));
        } else {
            walk(&path, depth + 1, out);
        }
    }
}

fn collect_games(root: &Path, stop_below_root_ini: bool, out: &mut Vec<GameEntry>) {
    if root.join(INI_NAME).is_file() {
        out.push(game_entry(root));
        if stop_below_root_ini {
            return;
        }
    }
    walk(root, 1, out);
}

fn profile_match_score(
    game: &GameEntry,
    profile: &ProfileEntry,
    roots: &[GameRoot],
) -> Option<usize> {
    let game_dir = normalized_path(&game.folder);
    let profile_dir = normalized_path(&profile.dir);
    if game_dir.is_empty() || profile_dir.is_empty() {
        return None;
    }

    // Absolute path relationships are decisive for compatibility folders. Many
    // Raw Thrills titles share a final `rawart` directory name, so a leaf-only
    // comparison can assign the same friendly name to several different games.
    if game_dir == profile_dir {
        return Some(1_000_000 + game_dir.len());
    }
    if normalized_is_within(&profile_dir, &game_dir) {
        return Some(900_000 + game_dir.len());
    }
    if normalized_is_within(&game_dir, &profile_dir) {
        return Some(800_000 + profile_dir.len());
    }

    // Retain the old relative-path fallback for portable libraries whose drive
    // letter or leading folders changed after the TeknoParrot profile was saved.
    roots
        .iter()
        .filter(|root| root.kind == GameRootKind::Library)
        .filter_map(|root| {
            let root = normalized_path(&display_path(&root.path));
            let relative = if game_dir == root {
                ""
            } else {
                game_dir.strip_prefix(&format!(r"{root}\"))?
            };
            if relative.is_empty() {
                return None;
            }
            let needle = format!(r"\{relative}\");
            if profile_dir.ends_with(&format!(r"\{relative}")) {
                Some(10_000 + relative.len())
            } else if format!(r"{profile_dir}\").contains(&needle) {
                Some(1_000 + relative.len())
            } else {
                None
            }
        })
        .max()
}

#[tauri::command(async)]
fn scan_games(
    root: String,
    compatibility_paths: Option<Vec<String>>,
    teknoparrot_path: Option<String>,
) -> Result<ScanResult, String> {
    let compatibility_paths = compatibility_paths.unwrap_or_default();
    let resolved_roots = canonical_game_roots(&root, &compatibility_paths)?;
    let roots = resolved_roots.roots;
    let mut games = Vec::new();
    for game_root in &roots {
        collect_games(
            &game_root.path,
            game_root.kind == GameRootKind::CompatibilityGame,
            &mut games,
        );
    }
    let mut seen = HashSet::new();
    games.retain(|game| seen.insert(normalized_path(&game.ini_path)));

    if let Some(tp_path) = teknoparrot_path.filter(|p| !p.trim().is_empty()) {
        if let Ok(profiles) = list_profiles(tp_path) {
            for game in &mut games {
                let mut candidates = profiles
                    .iter()
                    .filter_map(|profile| {
                        profile_match_score(game, profile, &roots).map(|score| (score, profile))
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
                if let Some((_, profile)) = candidates.first() {
                    game.name = profile.name.clone();
                    game.named_by_profile = true;
                }
            }
        }
    }
    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(ScanResult {
        games,
        warnings: resolved_roots.warnings,
    })
}

fn user_profiles_dir(teknoparrot_path: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(teknoparrot_path.trim());
    let dir = if base
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("UserProfiles"))
        .unwrap_or(false)
    {
        base
    } else {
        base.join("UserProfiles")
    };
    if !dir.is_dir() {
        return Err(format!(
            "UserProfiles folder not found under: {}",
            PathBuf::from(teknoparrot_path).display()
        ));
    }
    Ok(dir)
}

#[tauri::command(async)]
fn list_profiles(teknoparrot_path: String) -> Result<Vec<ProfileEntry>, String> {
    let dir = user_profiles_dir(&teknoparrot_path)?;
    let read = fs::read_dir(&dir).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
    let mut profiles = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let is_xml = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("xml"))
            .unwrap_or(false);
        if !path.is_file() || !is_xml {
            continue;
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Could not read {} as UTF-8 XML: {e}", path.display()))?;
        let fallback = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown profile".to_string());
        let parsed = parse_profile(&content, &fallback);
        profiles.push(ProfileEntry {
            name: parsed.name,
            exe: parsed.exe,
            dir: parsed.dir,
            file: path.to_string_lossy().to_string(),
            ffb_supported: parsed.ffb_supported,
            ffb_enabled: parsed.ffb_enabled,
        });
    }
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(profiles)
}

fn resolve_profile_file(
    teknoparrot_path: &str,
    profile_file: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let profiles_dir = fs::canonicalize(user_profiles_dir(teknoparrot_path)?)
        .map_err(|e| format!("Could not resolve the UserProfiles folder: {e}"))?;
    let profile = fs::canonicalize(profile_file)
        .map_err(|e| format!("Could not resolve profile {}: {e}", profile_file.display()))?;

    let is_xml = profile
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("xml"))
        .unwrap_or(false);
    if !is_xml || profile.parent() != Some(profiles_dir.as_path()) {
        return Err(
            "Refusing to use a profile outside the selected UserProfiles folder.".to_string(),
        );
    }
    Ok((profiles_dir, profile))
}

fn resolve_profile_launch(
    teknoparrot_path: &str,
    profile_file: &Path,
) -> Result<(PathBuf, PathBuf, String), String> {
    let (profiles_dir, profile) = resolve_profile_file(teknoparrot_path, profile_file)?;

    let profile_name = profile
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "The selected profile filename is not valid Unicode.".to_string())?
        .to_string();
    let tp_root = profiles_dir
        .parent()
        .ok_or_else(|| "Could not determine the TeknoParrot emulator folder.".to_string())?
        .to_path_buf();
    let launcher = tp_root.join("TeknoParrotUi.exe");
    if !launcher.is_file() {
        return Err(format!(
            "TeknoParrotUi.exe not found at: {}",
            launcher.display()
        ));
    }
    Ok((launcher, tp_root, profile_name))
}

#[tauri::command]
fn launch_profile(teknoparrot_path: String, profile_file: String) -> Result<u32, String> {
    let (launcher, working_dir, profile_name) =
        resolve_profile_launch(&teknoparrot_path, Path::new(&profile_file))?;
    Command::new(&launcher)
        .arg(format!("--profile={profile_name}"))
        .current_dir(&working_dir)
        .spawn()
        .map(|child| child.id())
        .map_err(|e| format!("Could not launch {}: {e}", launcher.display()))
}

fn resolve_profile_scan_root(
    roots: &[GameRoot],
    profile_name: &str,
    game_path: &str,
) -> Result<PathBuf, String> {
    if game_path.trim().is_empty() {
        return Err(format!("{profile_name} has no GamePath configured."));
    }

    let configured = PathBuf::from(game_path.trim());
    if !configured.is_absolute() {
        return Err(format!(
            "{profile_name} has a relative GamePath: {}",
            configured.display()
        ));
    }
    let resolved_game = fs::canonicalize(&configured).map_err(|error| {
        format!(
            "{profile_name} game path was not found: {} ({error})",
            configured.display()
        )
    })?;
    let executable_dir = if resolved_game.is_dir() {
        resolved_game
    } else if resolved_game.is_file() {
        resolved_game
            .parent()
            .ok_or_else(|| format!("Could not determine {profile_name}'s game folder."))?
            .to_path_buf()
    } else {
        return Err(format!(
            "{profile_name} GamePath is not a regular file or folder: {}",
            configured.display()
        ));
    };
    let executable_dir = fs::canonicalize(&executable_dir).map_err(|error| {
        format!(
            "Could not resolve {profile_name}'s game folder {}: {error}",
            executable_dir.display()
        )
    })?;
    let allowed = roots
        .iter()
        .filter(|root| path_is_within(&executable_dir, &root.path))
        .max_by_key(|root| path_key(&root.path).len())
        .ok_or_else(|| {
            format!(
                "{profile_name} is outside the configured game folders: {}",
                configured.display()
            )
        })?;
    let top_level_dir = match allowed.kind {
        GameRootKind::CompatibilityGame => allowed.path.clone(),
        GameRootKind::Library => {
            let relative = executable_dir.strip_prefix(&allowed.path).map_err(|_| {
                format!(
                    "Could not safely resolve {profile_name}'s folder under the selected games folder."
                )
            })?;
            match relative.components().next() {
                Some(std::path::Component::Normal(component)) => allowed.path.join(component),
                Some(_) => {
                    return Err(format!(
                        "Could not safely resolve {profile_name}'s top-level game folder."
                    ))
                }
                None => {
                    return Err(format!(
                        "{profile_name}'s GamePath must be inside an individual game folder under the selected games folder."
                    ))
                }
            }
        }
    };
    let top_level_dir = fs::canonicalize(&top_level_dir).map_err(|error| {
        format!(
            "Could not resolve {profile_name}'s top-level folder {}: {error}",
            top_level_dir.display()
        )
    })?;
    if !path_is_within(&top_level_dir, &allowed.path)
        || !path_is_within(&executable_dir, &top_level_dir)
    {
        return Err(format!(
            "Refusing to watch outside {profile_name}'s game folder."
        ));
    }

    Ok(top_level_dir)
}

#[tauri::command(async)]
fn find_profile_ini(
    root: String,
    compatibility_paths: Option<Vec<String>>,
    teknoparrot_path: String,
    profile_file: String,
) -> Result<Option<GameEntry>, String> {
    let (_, profile) = resolve_profile_file(&teknoparrot_path, Path::new(&profile_file))?;
    let content = fs::read_to_string(&profile)
        .map_err(|e| format!("Could not read profile {}: {e}", profile.display()))?;
    let fallback = profile
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown profile".to_string());
    let parsed = parse_profile(&content, &fallback);
    if parsed.exe.trim().is_empty() {
        return Err(format!(
            "{} has no game executable configured.",
            parsed.name
        ));
    }

    let roots = canonical_game_roots(&root, &compatibility_paths.unwrap_or_default())?.roots;
    let scan_root = resolve_profile_scan_root(&roots, &parsed.name, &parsed.exe)?;
    let mut games = Vec::new();
    collect_games(&scan_root, true, &mut games);
    let Some(mut game) = games.into_iter().next() else {
        return Ok(None);
    };
    game.name = parsed.name;
    game.named_by_profile = true;
    Ok(Some(game))
}

#[tauri::command(async)]
fn find_legacy_plugins(
    root: String,
    compatibility_paths: Option<Vec<String>>,
) -> Result<Vec<legacy::LegacyInstall>, String> {
    let roots = canonical_game_roots(&root, &compatibility_paths.unwrap_or_default())?.roots;
    let mut installs = Vec::new();
    for game_root in roots {
        installs.extend(legacy::find_legacy_installs(&PathBuf::from(display_path(
            &game_root.path,
        )))?);
    }
    let mut seen = HashSet::new();
    installs.retain(|install| seen.insert(normalized_path(&install.folder)));
    installs.sort_by_key(|install| normalized_path(&install.folder));
    Ok(installs)
}

#[tauri::command(async)]
fn quarantine_legacy_plugin(
    root: String,
    compatibility_paths: Option<Vec<String>>,
    folder: String,
) -> Result<legacy::LegacyRemoval, String> {
    let roots = canonical_game_roots(&root, &compatibility_paths.unwrap_or_default())?.roots;
    let folder = fs::canonicalize(PathBuf::from(&folder))
        .map_err(|error| format!("Could not resolve legacy plugin folder {folder}: {error}"))?;
    let allowed = roots
        .iter()
        .filter(|game_root| path_is_within(&folder, &game_root.path))
        .max_by_key(|game_root| path_key(&game_root.path).len())
        .ok_or_else(|| {
            "Refusing to remove files outside the configured game folders.".to_string()
        })?;
    legacy::quarantine_legacy_install(&allowed.path, &folder)
}

fn validate_profile_target(path: &Path) -> Result<(), String> {
    let is_xml = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("xml"))
        .unwrap_or(false);
    let in_user_profiles = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("UserProfiles"))
        .unwrap_or(false);
    if !is_xml || !in_user_profiles {
        return Err("Refusing to write outside a TeknoParrot UserProfiles XML file.".into());
    }
    if !path.is_file() {
        return Err(format!("Profile not found: {}", path.display()));
    }
    if is_read_only(path) {
        return Err("Profile is read-only. No changes were written.".into());
    }
    Ok(())
}

#[tauri::command(async)]
fn set_ffb_blaster(
    files: Vec<String>,
    enable: bool,
    backup: bool,
) -> Result<Vec<WriteReport>, String> {
    if files.is_empty() {
        return Err("No TeknoParrot profiles were selected.".into());
    }
    let mut reports = Vec::new();
    for file in files {
        let path = PathBuf::from(&file);
        let mut backup_path = String::new();
        let result = (|| -> Result<bool, String> {
            validate_profile_target(&path)?;
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("Could not read this profile as UTF-8 XML: {e}"))?;
            if has_mixed_or_unsupported_line_endings(&content) {
                return Err(
                    "Mixed or unsupported line endings detected; no changes were written.".into(),
                );
            }
            let updated = set_ffb_enabled(&content, enable)
                .ok_or_else(|| "This profile has no FFB Blaster Enable setting.".to_string())?;
            if updated == content {
                return Ok(false);
            }
            if backup {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                let bak = path.with_file_name(format!("{name}.{}.bak", unique_stamp()));
                fs::copy(&path, &bak).map_err(|e| format!("backup failed: {e}"))?;
                backup_path = bak.to_string_lossy().to_string();
            }
            atomic_write(&path, updated.as_bytes())?;
            Ok(true)
        })();
        reports.push(match result {
            Ok(changed) => WriteReport {
                path: file,
                ok: true,
                changed,
                message: if changed {
                    "Enabled".into()
                } else {
                    "Already set".into()
                },
                backup: backup_path,
            },
            Err(message) => WriteReport {
                path: file,
                ok: false,
                changed: false,
                message,
                backup: backup_path,
            },
        });
    }
    Ok(reports)
}

#[tauri::command(async)]
fn list_devices(targets: Vec<String>) -> Result<Vec<DeviceEntry>, String> {
    devices::list_devices(targets)
}

#[tauri::command(async)]
fn read_ini(path: String) -> Result<IniFile, String> {
    let p = PathBuf::from(&path);
    let content = fs::read_to_string(&p).map_err(|e| format!("{}: {}", path, e))?;
    Ok(IniFile {
        entries: parse(&content),
        read_only: is_read_only(&p),
        path,
    })
}

#[tauri::command(async)]
fn preview_changes(
    targets: Vec<String>,
    changes: Vec<Change>,
) -> Result<Vec<PreviewReport>, String> {
    if changes.is_empty() {
        return Err("Nothing to preview.".into());
    }
    validate_changes(&changes)?;

    Ok(targets
        .into_iter()
        .map(|target| {
            let path = PathBuf::from(&target);
            let name = path
                .parent()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| target.clone());

            match read_editable_ini(&path) {
                Ok(content) => {
                    let entries = parse(&content);
                    let diffs = changes
                        .iter()
                        .filter_map(|change| {
                            let old = entries
                                .iter()
                                .find(|entry| {
                                    entry.section.eq_ignore_ascii_case(change.section.trim())
                                        && entry.key.eq_ignore_ascii_case(change.key.trim())
                                })
                                .map(|entry| entry.value.clone());
                            let new = change.value.trim().to_string();
                            (old.as_deref() != Some(new.as_str())).then(|| PreviewChange {
                                key: change.key.clone(),
                                old,
                                new,
                            })
                        })
                        .collect::<Vec<_>>();
                    let message = if diffs.is_empty() {
                        "No changes needed".into()
                    } else {
                        "Ready".into()
                    };
                    PreviewReport {
                        path: target,
                        name,
                        ok: true,
                        message,
                        changes: diffs,
                    }
                }
                Err(message) => PreviewReport {
                    path: target,
                    name,
                    ok: false,
                    message,
                    changes: Vec::new(),
                },
            }
        })
        .collect())
}

#[tauri::command(async)]
fn write_changes(
    targets: Vec<String>,
    changes: Vec<Change>,
    backup: bool,
) -> Result<Vec<WriteReport>, String> {
    if changes.is_empty() {
        return Err("Nothing to save.".into());
    }
    validate_changes(&changes)?;
    let mut reports = Vec::new();
    for target in targets {
        let path = PathBuf::from(&target);
        let mut backup_path = String::new();

        let result = (|| -> Result<bool, String> {
            let content = read_editable_ini(&path)?;
            let updated = apply_changes(&content, &changes);
            if updated == content {
                return Ok(false); // nothing to write; don't churn a backup
            }
            if backup {
                let bak = path.with_file_name(format!("{}.{}.bak", INI_NAME, unique_stamp()));
                fs::copy(&path, &bak).map_err(|e| format!("backup failed: {}", e))?;
                backup_path = bak.to_string_lossy().to_string();
            }
            atomic_write(&path, updated.as_bytes())?;
            Ok(true)
        })();

        reports.push(match result {
            Ok(changed) => WriteReport {
                path: target,
                ok: true,
                changed,
                message: if changed {
                    "Saved".into()
                } else {
                    "No changes needed".into()
                },
                backup: backup_path,
            },
            Err(e) => WriteReport {
                path: target,
                ok: false,
                changed: false,
                message: e,
                backup: backup_path,
            },
        });
    }
    Ok(reports)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            load_presets,
            save_presets,
            scan_games,
            list_profiles,
            set_ffb_blaster,
            launch_profile,
            find_profile_ini,
            find_legacy_plugins,
            quarantine_legacy_plugin,
            list_devices,
            read_ini,
            preview_changes,
            write_changes
        ])
        .run(tauri::generate_context!())
        .expect("failed to start FFBBlaster MASTER");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ffbblaster-master-{name}-{}-{}",
            std::process::id(),
            unique_stamp()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn change(key: &str, value: &str) -> Change {
        Change {
            section: "SETTINGS".into(),
            key: key.into(),
            value: value.into(),
        }
    }

    fn enabled_profile(name: &str, game_path: &Path) -> String {
        format!(
            "<GameProfile><GameNameInternal>{name}</GameNameInternal><GamePath>{}</GamePath><FieldInformation><CategoryName>FFB Blaster</CategoryName><FieldName>Enable</FieldName><FieldValue>1</FieldValue></FieldInformation></GameProfile>",
            game_path.display()
        )
    }

    #[test]
    fn missing_settings_fields_keep_safe_defaults() {
        let settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.backup_on_save);
        assert!(settings.compatibility_paths.is_empty());
        assert!(settings.hide_gui_initialized_paths.is_empty());
    }

    #[test]
    fn refuses_a_whole_drive_as_the_main_game_library() {
        let current = fs::canonicalize(".").unwrap();
        let volume_root = current.ancestors().last().unwrap();
        assert!(is_filesystem_root(volume_root));

        let error = canonical_game_roots(&display_path(volume_root), &[]).unwrap_err();
        assert!(error.contains("main game library folder, not the whole drive"));
    }

    #[test]
    fn scans_previews_and_writes_a_preserved_file() {
        let root = test_root("write");
        let game = root.join("GAME A");
        fs::create_dir_all(&game).unwrap();
        let ini = game.join(INI_NAME);
        let original = "[Settings]\r\n  MinForce = 0   ; tuned value\r\nMaxForce=100\r\n";
        fs::write(&ini, original).unwrap();

        let games = scan_games(root.to_string_lossy().to_string(), None, None)
            .unwrap()
            .games;
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "GAME A");

        let targets = vec![ini.to_string_lossy().to_string()];
        let changes = vec![change("MinForce", "20")];
        let preview = preview_changes(targets.clone(), changes.clone()).unwrap();
        assert!(preview[0].ok);
        assert_eq!(preview[0].changes.len(), 1);
        assert_eq!(preview[0].changes[0].old.as_deref(), Some("0"));

        let reports = write_changes(targets, changes, true).unwrap();
        assert!(reports[0].ok);
        assert!(reports[0].changed);
        assert!(!reports[0].backup.is_empty());
        assert_eq!(
            fs::read_to_string(&ini).unwrap(),
            "[Settings]\r\n  MinForce = 20   ; tuned value\r\nMaxForce=100\r\n"
        );
        assert!(Path::new(&reports[0].backup).is_file());
        assert!(!fs::read_dir(&game).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".tmp")
        }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_deeply_nested_game_executable_folders() {
        let root = test_root("deep-scan");
        let game = root
            .join("GAME A")
            .join("package")
            .join("data")
            .join("bin")
            .join("win64");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join(INI_NAME), "[SETTINGS]\r\nMinForce=0\r\n").unwrap();

        let games = scan_games(root.to_string_lossy().to_string(), None, None)
            .unwrap()
            .games;
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].folder, game.to_string_lossy());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_primary_and_exact_compatibility_roots_without_duplicates_or_name_collisions() {
        let root = test_root("compatibility-scan");
        let games_root = root.join("ARCADE");
        let primary_game = games_root.join("Primary Game");
        let fnf_root = root.join("FNF");
        let fnf_rawart = fnf_root.join("rawart");
        let superbikes_root = root.join("FFSB2");
        let superbikes_rawart = superbikes_root.join("rawart");
        let nicktoons_root = root.join("Nicktoons Nitro Racing");
        let tp_root = root.join("TeknoParrot");
        let profiles_dir = tp_root.join("UserProfiles");

        for folder in [
            &primary_game,
            &fnf_rawart,
            &superbikes_rawart,
            &nicktoons_root,
            &profiles_dir,
        ] {
            fs::create_dir_all(folder).unwrap();
        }
        for folder in [
            &primary_game,
            &fnf_rawart,
            &superbikes_rawart,
            &nicktoons_root,
        ] {
            fs::write(folder.join(INI_NAME), "[SETTINGS]\r\nMinForce=0\r\n").unwrap();
        }

        let fnf_exe = fnf_rawart.join("sdaemon.exe");
        let superbikes_exe = superbikes_rawart.join("sdaemon.exe");
        let nicktoons_exe = nicktoons_root.join("sdaemon.exe");
        for executable in [&fnf_exe, &superbikes_exe, &nicktoons_exe] {
            fs::write(executable, b"test game").unwrap();
        }
        fs::write(
            profiles_dir.join("FNF.xml"),
            enabled_profile("The Fast and the Furious", &fnf_exe),
        )
        .unwrap();
        fs::write(
            profiles_dir.join("FNFSB2.xml"),
            enabled_profile("Super Bikes 2", &superbikes_exe),
        )
        .unwrap();
        fs::write(
            profiles_dir.join("NicktoonsNitro.xml"),
            enabled_profile("Nicktoons Nitro Racing", &nicktoons_exe),
        )
        .unwrap();

        let games = scan_games(
            games_root.to_string_lossy().to_string(),
            Some(vec![
                primary_game.to_string_lossy().to_string(),
                fnf_root.to_string_lossy().to_string(),
                fnf_root.to_string_lossy().to_string(),
                superbikes_root.to_string_lossy().to_string(),
                nicktoons_root.to_string_lossy().to_string(),
            ]),
            Some(tp_root.to_string_lossy().to_string()),
        )
        .unwrap()
        .games;

        assert_eq!(games.len(), 4);
        assert_eq!(
            games
                .iter()
                .filter(|game| normalized_path(&game.ini_path)
                    == normalized_path(&primary_game.join(INI_NAME).to_string_lossy()))
                .count(),
            1
        );

        let fnf = games
            .iter()
            .find(|game| normalized_path(&game.folder) == path_key(&fnf_rawart))
            .unwrap();
        assert_eq!(fnf.name, "The Fast and the Furious");
        assert!(fnf.named_by_profile);

        let superbikes = games
            .iter()
            .find(|game| normalized_path(&game.folder) == path_key(&superbikes_rawart))
            .unwrap();
        assert_eq!(superbikes.name, "Super Bikes 2");
        assert!(superbikes.named_by_profile);

        let nicktoons = games
            .iter()
            .find(|game| normalized_path(&game.folder) == path_key(&nicktoons_root))
            .unwrap();
        assert_eq!(nicktoons.name, "Nicktoons Nitro Racing");
        assert!(nicktoons.named_by_profile);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_stale_compatibility_paths_and_stops_below_a_root_ini() {
        let root = test_root("compatibility-warning");
        let games_root = root.join("ARCADE");
        let primary_game = games_root.join("Primary Game");
        let compatibility_root = root.join("Root Game");
        let nested = compatibility_root.join("rawart");
        let missing = root.join("Moved Game");
        fs::create_dir_all(&primary_game).unwrap();
        fs::create_dir_all(&nested).unwrap();
        for folder in [&primary_game, &compatibility_root, &nested] {
            fs::write(folder.join(INI_NAME), "[SETTINGS]\r\nMinForce=0\r\n").unwrap();
        }

        let result = scan_games(
            games_root.to_string_lossy().to_string(),
            Some(vec![
                compatibility_root.to_string_lossy().to_string(),
                missing.to_string_lossy().to_string(),
            ]),
            None,
        )
        .unwrap();

        assert_eq!(result.games.len(), 2);
        assert!(result
            .games
            .iter()
            .any(|game| normalized_path(&game.folder) == path_key(&compatibility_root)));
        assert!(!result
            .games
            .iter()
            .any(|game| normalized_path(&game.folder) == path_key(&nested)));
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("not found"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_only_and_mixed_ending_files_are_blocked_before_writing() {
        let root = test_root("blocked");
        let read_only_dir = root.join("READ ONLY");
        let mixed_dir = root.join("MIXED");
        fs::create_dir_all(&read_only_dir).unwrap();
        fs::create_dir_all(&mixed_dir).unwrap();
        let read_only_ini = read_only_dir.join(INI_NAME);
        let mixed_ini = mixed_dir.join(INI_NAME);
        fs::write(&read_only_ini, "[SETTINGS]\r\nMinForce=0\r\n").unwrap();
        fs::write(&mixed_ini, "[SETTINGS]\r\nMinForce=0\n").unwrap();

        let mut permissions = fs::metadata(&read_only_ini).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&read_only_ini, permissions).unwrap();

        let preview = preview_changes(
            vec![
                read_only_ini.to_string_lossy().to_string(),
                mixed_ini.to_string_lossy().to_string(),
            ],
            vec![change("MinForce", "20")],
        )
        .unwrap();
        assert!(!preview[0].ok);
        assert!(preview[0].message.contains("read-only"));
        assert!(!preview[1].ok);
        assert!(preview[1].message.contains("line endings"));

        let mut permissions = fs::metadata(&read_only_ini).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&read_only_ini, permissions).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_names_are_matched_and_only_ffb_switch_is_updated() {
        let root = test_root("profiles");
        let games_root = root.join("ARCADE");
        let game = games_root.join("GAME A");
        let profiles_dir = root.join("TeknoParrot").join("UserProfiles");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(game.join(INI_NAME), "[SETTINGS]\r\nMinForce=0\r\n").unwrap();
        let profile_path = profiles_dir.join("game-a.xml");
        let original = "\u{feff}<GameProfile>\r\n  <ProfileName>raw</ProfileName>\r\n  <GameNameInternal>Friendly Game A</GameNameInternal>\r\n  <GamePath>R:\\Arcade\\TeknoParrot\\GAME A\\game.exe</GamePath>\r\n  <FieldInformation>\r\n    <CategoryName>General</CategoryName>\r\n    <FieldName>Windowed</FieldName>\r\n    <FieldValue>0</FieldValue>\r\n  </FieldInformation>\r\n  <FieldInformation>\r\n    <CategoryName>FFB Blaster</CategoryName>\r\n    <FieldName>Enable</FieldName>\r\n    <FieldValue>0</FieldValue>\r\n  </FieldInformation>\r\n</GameProfile>\r\n";
        fs::write(&profile_path, original).unwrap();

        let profiles =
            list_profiles(root.join("TeknoParrot").to_string_lossy().to_string()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Friendly Game A");
        assert!(profiles[0].ffb_supported);
        assert!(!profiles[0].ffb_enabled);

        let games = scan_games(
            games_root.to_string_lossy().to_string(),
            None,
            Some(root.join("TeknoParrot").to_string_lossy().to_string()),
        )
        .unwrap()
        .games;
        assert_eq!(games[0].name, "Friendly Game A");
        assert_eq!(games[0].folder_name, "GAME A");
        assert!(games[0].named_by_profile);

        let reports =
            set_ffb_blaster(vec![profile_path.to_string_lossy().to_string()], true, true).unwrap();
        assert!(reports[0].ok);
        assert!(reports[0].changed);
        assert_eq!(fs::read_to_string(&reports[0].backup).unwrap(), original);
        let updated = fs::read_to_string(&profile_path).unwrap();
        assert_eq!(updated.matches("<FieldValue>0</FieldValue>").count(), 1);
        assert_eq!(updated.matches("<FieldValue>1</FieldValue>").count(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_launch_is_limited_to_the_selected_teknoparrot_folder() {
        let root = test_root("profile-launch");
        let tp_root = root.join("TeknoParrot");
        let profiles_dir = tp_root.join("UserProfiles");
        let profile = profiles_dir.join("ArcticThunder.xml");
        let outside = root.join("Outside.xml");
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(tp_root.join("TeknoParrotUi.exe"), b"test launcher").unwrap();
        fs::write(&profile, b"<GameProfile />").unwrap();
        fs::write(&outside, b"<GameProfile />").unwrap();

        let (launcher, working_dir, profile_name) =
            resolve_profile_launch(&tp_root.to_string_lossy(), &profile).unwrap();
        assert_eq!(
            launcher,
            fs::canonicalize(tp_root.join("TeknoParrotUi.exe")).unwrap()
        );
        assert_eq!(working_dir, fs::canonicalize(&tp_root).unwrap());
        assert_eq!(profile_name, "ArcticThunder.xml");
        assert!(resolve_profile_launch(&profiles_dir.to_string_lossy(), &profile).is_ok());
        assert!(resolve_profile_launch(&tp_root.to_string_lossy(), &outside).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_a_new_ini_only_inside_the_profile_game_folder() {
        let root = test_root("profile-ini-watch");
        let games_root = root.join("ARCADE");
        let game_root = games_root.join("Ballistics");
        let game_bin = game_root.join("bin");
        let tp_root = root.join("TeknoParrot");
        let profiles_dir = tp_root.join("UserProfiles");
        let profile = profiles_dir.join("Ballistics.xml");
        fs::create_dir_all(&game_bin).unwrap();
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(tp_root.join("TeknoParrotUi.exe"), b"test launcher").unwrap();
        fs::write(game_bin.join("game.exe"), b"test game").unwrap();
        fs::write(
            &profile,
            format!(
                "<GameProfile><GameNameInternal>Ballistics</GameNameInternal><GamePath>{}</GamePath></GameProfile>",
                game_bin.join("game.exe").display()
            ),
        )
        .unwrap();

        let missing = find_profile_ini(
            games_root.to_string_lossy().to_string(),
            None,
            tp_root.to_string_lossy().to_string(),
            profile.to_string_lossy().to_string(),
        )
        .unwrap();
        assert!(missing.is_none());

        fs::write(game_root.join(INI_NAME), "[SETTINGS]\r\nMinForce=0\r\n").unwrap();
        let found = find_profile_ini(
            games_root.to_string_lossy().to_string(),
            None,
            tp_root.to_string_lossy().to_string(),
            profile.to_string_lossy().to_string(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.name, "Ballistics");
        assert_eq!(found.folder, game_root.to_string_lossy());
        assert!(found.named_by_profile);

        let outside_game = root.join("OUTSIDE GAME").join("game.exe");
        fs::create_dir_all(outside_game.parent().unwrap()).unwrap();
        fs::write(&outside_game, b"outside game").unwrap();
        fs::write(
            &profile,
            format!(
                "<GameProfile><GameNameInternal>Outside</GameNameInternal><GamePath>{}</GamePath></GameProfile>",
                outside_game.display()
            ),
        )
        .unwrap();
        assert!(find_profile_ini(
            games_root.to_string_lossy().to_string(),
            None,
            tp_root.to_string_lossy().to_string(),
            profile.to_string_lossy().to_string(),
        )
        .is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_watch_honors_exact_compatibility_root_scope() {
        let root = test_root("compatibility-scope");
        let games_root = root.join("ARCADE");
        let compatibility_root = root.join("FNF");
        let compatibility_rawart = compatibility_root.join("rawart");
        let sibling_root = root.join("FFSB2");
        let sibling_rawart = sibling_root.join("rawart");
        let tp_root = root.join("TeknoParrot");
        let profiles_dir = tp_root.join("UserProfiles");
        let compatibility_profile = profiles_dir.join("FNF.xml");
        let sibling_profile = profiles_dir.join("FNFSB2.xml");
        let compatibility_exe = compatibility_rawart.join("sdaemon.exe");
        let sibling_exe = sibling_rawart.join("sdaemon.exe");

        for folder in [
            &games_root,
            &compatibility_rawart,
            &sibling_rawart,
            &profiles_dir,
        ] {
            fs::create_dir_all(folder).unwrap();
        }
        fs::write(&compatibility_exe, b"test game").unwrap();
        fs::write(&sibling_exe, b"test game").unwrap();
        fs::write(
            &compatibility_profile,
            enabled_profile("The Fast and the Furious", &compatibility_exe),
        )
        .unwrap();
        fs::write(
            &sibling_profile,
            enabled_profile("Super Bikes 2", &sibling_exe),
        )
        .unwrap();

        let compatibility_paths = || Some(vec![compatibility_root.to_string_lossy().to_string()]);

        assert!(find_profile_ini(
            games_root.to_string_lossy().to_string(),
            compatibility_paths(),
            tp_root.to_string_lossy().to_string(),
            compatibility_profile.to_string_lossy().to_string(),
        )
        .unwrap()
        .is_none());
        let outside_error = find_profile_ini(
            games_root.to_string_lossy().to_string(),
            compatibility_paths(),
            tp_root.to_string_lossy().to_string(),
            sibling_profile.to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(outside_error.contains("outside"));

        fs::write(
            compatibility_root.join(INI_NAME),
            "[SETTINGS]\r\nMinForce=0\r\n",
        )
        .unwrap();

        let found = find_profile_ini(
            games_root.to_string_lossy().to_string(),
            compatibility_paths(),
            tp_root.to_string_lossy().to_string(),
            compatibility_profile.to_string_lossy().to_string(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.name, "The Fast and the Furious");
        assert_eq!(PathBuf::from(found.folder), compatibility_root);

        assert!(!sibling_rawart.join(INI_NAME).exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_watch_rejects_a_game_path_directly_in_the_selected_games_root() {
        let root = test_root("profile-root-target");
        let games_root = root.join("ARCADE");
        let game_exe = games_root.join("game.exe");
        let tp_root = root.join("TeknoParrot");
        let profiles_dir = tp_root.join("UserProfiles");
        let profile = profiles_dir.join("RootGame.xml");
        fs::create_dir_all(&games_root).unwrap();
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(&game_exe, b"test game").unwrap();
        fs::write(&profile, enabled_profile("Root Game", &game_exe)).unwrap();

        let error = find_profile_ini(
            games_root.to_string_lossy().to_string(),
            None,
            tp_root.to_string_lossy().to_string(),
            profile.to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(error.contains("individual game folder"));
        assert!(!games_root.join(INI_NAME).exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_values_are_rejected_before_any_file_is_opened() {
        let invalid_number = preview_changes(
            vec!["missing\\FFBBlaster.ini".into()],
            vec![change("MinForce", "101")],
        )
        .unwrap_err();
        assert!(invalid_number.contains("0 to 100"));

        let invalid_guid = preview_changes(
            vec!["missing\\FFBBlaster.ini".into()],
            vec![change("DeviceGUID", "not-a-guid")],
        )
        .unwrap_err();
        assert!(invalid_guid.contains("32 hexadecimal"));

        let injection = preview_changes(
            vec!["missing\\FFBBlaster.ini".into()],
            vec![change("UnknownFutureKey", "okay\nAnotherKey=bad")],
        )
        .unwrap_err();
        assert!(injection.contains("invalid key or value"));
    }
}
