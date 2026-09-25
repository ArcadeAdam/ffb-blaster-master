//! Detection and recoverable removal of the superseded FFB Arcade Plugin.
//!
//! Several wrapper DLL names are generic and may belong to unrelated game
//! patches. We only touch a wrapper when its embedded version strings identify
//! it as FFB Arcade Plugin, and only offer a folder when other legacy package
//! evidence is present.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PACKAGE_FILES: &[&str] = &[
    "ffbplugin.ini",
    "ffbplugingui.exe",
    "ffbpluginreadme.txt",
    "metroframework.dll",
    "sdl2.dll",
];

const WRAPPER_FILES: &[&str] = &[
    "dinput8.dll",
    "d3d9.dll",
    "d3d11.dll",
    "opengl32.dll",
    "winmm.dll",
    "xinput1_3.dll",
];

const BACKUP_PREFIX: &str = "_FFBPlugin_legacy_backup_";
const MAX_DIRECTORIES: usize = 100_000;

#[derive(Debug, Clone, Serialize)]
pub struct LegacyFile {
    pub name: String,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyInstall {
    pub folder: String,
    pub files: Vec<LegacyFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyRemoval {
    pub folder: String,
    pub backup_folder: String,
    pub moved_files: Vec<String>,
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn utf16le(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

fn is_verified_wrapper(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    contains_bytes(&bytes, &utf16le("FFB Arcade Plugin"))
        && contains_bytes(&bytes, &utf16le("Force Feedback Plugin"))
}

fn files_by_lower_name(dir: &Path) -> HashMap<String, PathBuf> {
    let mut files = HashMap::new();
    let Ok(read) = fs::read_dir(dir) else {
        return files;
    };
    for entry in read.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            files.insert(
                entry.file_name().to_string_lossy().to_ascii_lowercase(),
                entry.path(),
            );
        }
    }
    files
}

fn inspect_legacy_dir(dir: &Path) -> Option<LegacyInstall> {
    let files = files_by_lower_name(dir);
    if !files.contains_key("ffbplugin.ini") {
        return None;
    }

    let verified_wrappers = WRAPPER_FILES
        .iter()
        .filter_map(|name| files.get(*name))
        .filter(|path| is_verified_wrapper(path))
        .cloned()
        .collect::<Vec<_>>();

    let has_package_marker = files.contains_key("ffbplugingui.exe")
        || files.contains_key("ffbpluginreadme.txt")
        || !verified_wrappers.is_empty();
    if !has_package_marker {
        return None;
    }

    let mut found = Vec::new();
    for name in PACKAGE_FILES {
        if let Some(path) = files.get(*name) {
            found.push(LegacyFile {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                path: path.to_string_lossy().to_string(),
                reason: if *name == "ffbplugin.ini" {
                    "Legacy FFB Arcade Plugin settings".to_string()
                } else {
                    "Legacy FFB Arcade Plugin support file".to_string()
                },
            });
        }
    }
    for path in verified_wrappers {
        found.push(LegacyFile {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            path: path.to_string_lossy().to_string(),
            reason: "Verified FFB Arcade Plugin wrapper".to_string(),
        });
    }
    found.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });

    Some(LegacyInstall {
        folder: dir.to_string_lossy().to_string(),
        files: found,
    })
}

pub fn find_legacy_installs(root: &Path) -> Result<Vec<LegacyInstall>, String> {
    if !root.is_dir() {
        return Err(format!("Games folder not found: {}", root.display()));
    }

    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0_usize;
    let mut found = Vec::new();
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > MAX_DIRECTORIES {
            return Err("Stopped legacy-plugin scan after 100,000 folders.".to_string());
        }

        if let Some(install) = inspect_legacy_dir(&dir) {
            found.push(install);
        }

        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir()
                && !kind.is_symlink()
                && !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(BACKUP_PREFIX)
            {
                stack.push(entry.path());
            }
        }
    }
    found.sort_by(|a, b| {
        a.folder
            .to_ascii_lowercase()
            .cmp(&b.folder.to_ascii_lowercase())
    });
    Ok(found)
}

fn backup_folder(parent: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    parent.join(format!("{BACKUP_PREFIX}{stamp}_{}", std::process::id()))
}

pub fn quarantine_legacy_install(root: &Path, folder: &Path) -> Result<LegacyRemoval, String> {
    let root = fs::canonicalize(root)
        .map_err(|e| format!("Could not resolve games folder {}: {e}", root.display()))?;
    let folder = fs::canonicalize(folder).map_err(|e| {
        format!(
            "Could not resolve legacy plugin folder {}: {e}",
            folder.display()
        )
    })?;
    if !folder.starts_with(&root) {
        return Err("Refusing to remove files outside the selected games folder.".to_string());
    }

    let install = inspect_legacy_dir(&folder).ok_or_else(|| {
        "The folder no longer matches a verified legacy FFB Plugin install.".to_string()
    })?;
    let backup = backup_folder(&folder);
    fs::create_dir(&backup)
        .map_err(|e| format!("Could not create backup folder {}: {e}", backup.display()))?;

    let mut moved = Vec::<(PathBuf, PathBuf)>::new();
    for legacy_file in &install.files {
        let source = PathBuf::from(&legacy_file.path);
        let destination = backup.join(&legacy_file.name);
        if let Err(error) = fs::rename(&source, &destination) {
            for (old_source, old_destination) in moved.iter().rev() {
                let _ = fs::rename(old_destination, old_source);
            }
            let _ = fs::remove_dir(&backup);
            return Err(format!(
                "Could not move {}: {error}. Any earlier moves were rolled back.",
                source.display()
            ));
        }
        moved.push((source, destination));
    }

    Ok(LegacyRemoval {
        folder: folder.to_string_lossy().to_string(),
        backup_folder: backup.to_string_lossy().to_string(),
        moved_files: moved
            .iter()
            .map(|(_, destination)| destination.to_string_lossy().to_string())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ffbblaster-master-legacy-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn fake_wrapper() -> Vec<u8> {
        let mut bytes = b"wrapper-prefix".to_vec();
        bytes.extend(utf16le("FFB Arcade Plugin"));
        bytes.extend(b"middle");
        bytes.extend(utf16le("Force Feedback Plugin"));
        bytes
    }

    #[test]
    fn finds_verified_legacy_package_but_ignores_generic_dll() {
        let root = test_root("detect");
        let legacy = root.join("Arctic Thunder").join("game");
        let unrelated = root.join("Other Game");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(legacy.join("FFBPlugin.ini"), b"[Settings]\r\nGameId=75\r\n").unwrap();
        fs::write(legacy.join("FFBPluginGUI.exe"), b"old gui").unwrap();
        fs::write(legacy.join("SDL2.dll"), b"old support").unwrap();
        fs::write(legacy.join("d3d9.dll"), fake_wrapper()).unwrap();
        fs::write(legacy.join("d3d11.dll"), b"unrelated graphics patch").unwrap();
        fs::write(unrelated.join("FFBPlugin.ini"), b"unverified").unwrap();
        fs::write(unrelated.join("d3d9.dll"), b"unrelated graphics patch").unwrap();

        let found = find_legacy_installs(&root).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].files.len(), 4);
        assert!(found[0].files.iter().any(|file| file.name == "d3d9.dll"));
        assert!(!found[0].files.iter().any(|file| file.name == "d3d11.dll"));
        assert!(found[0]
            .files
            .iter()
            .find(|file| file.name == "d3d9.dll")
            .unwrap()
            .reason
            .contains("Verified"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quarantine_is_recoverable_and_refuses_outside_folder() {
        let root = test_root("quarantine");
        let legacy = root.join("Game A");
        let outside = test_root("outside");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("FFBPlugin.ini"), b"settings").unwrap();
        fs::write(legacy.join("FFBPluginReadme.txt"), b"readme").unwrap();
        fs::write(legacy.join("dinput8.dll"), fake_wrapper()).unwrap();

        assert!(quarantine_legacy_install(&root, &outside).is_err());
        let removal = quarantine_legacy_install(&root, &legacy).unwrap();
        assert_eq!(removal.moved_files.len(), 3);
        assert!(!legacy.join("FFBPlugin.ini").exists());
        assert_eq!(
            fs::read(Path::new(&removal.backup_folder).join("FFBPlugin.ini")).unwrap(),
            b"settings"
        );
        assert!(find_legacy_installs(&root).unwrap().is_empty());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
