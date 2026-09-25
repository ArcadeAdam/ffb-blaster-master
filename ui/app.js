// FFB Blaster MASTER — front end.
// No build step, no npm: this is plain ES2020 talking to Rust via the Tauri
// global. Every control is generated from SCHEMA below, so adding a key that a
// future FFBBlaster version introduces means adding one line there.

const invoke = window.__TAURI__?.core?.invoke;
const dialog = window.__TAURI__?.dialog;
const openUrl = window.__TAURI__?.opener?.openUrl;

const SECTION = "SETTINGS";
const DONATE_URL = "https://www.paypal.com/paypalme/acbauer12/9.99";

const SCHEMA = [
  {
    title: "BASICS",
    blurb: "The switches worth reaching for first. Everything else is below.",
    keys: [
      { key: "EnableDamper", type: "bool", label: "Damper on" },
      { key: "EnableRumble", type: "bool", label: "Rumble on" },
      { key: "EnableRumbleTriggers", type: "bool", label: "Trigger rumble" },
      { key: "AlternativeFFB", type: "bool", label: "Use per-direction limits", help: "Only needed when a wheel pulls to one side. Set the left/right values further down." },
    ],
  },
  {
    title: "DEVICE",
    blurb: "The wheel or cabinet board this game drives. Use the field at the top of the window to push one GUID to every game at once.",
    keys: [
      { key: "DeviceGUID", type: "text", label: "Device GUID" },
    ],
  },
  {
    title: "FORCE",
    keys: [
      { key: "MinForce", type: "int", min: 0, max: 100, label: "Minimum force", help: "Raise until the wheel stops going slack around centre." },
      { key: "MaxForce", type: "int", min: 0, max: 100, label: "Maximum force" },
      { key: "FeedbackLength", type: "int", min: 0, max: 1000, label: "Effect length (ms)" },
      { key: "DefaultCentering", type: "int", min: 0, max: 100, label: "Centering spring" },
      { key: "DefaultFriction", type: "int", min: 0, max: 100, label: "Friction" },
      { key: "UseAltConstantEffect", type: "bool", label: "Alternate constant-force effect", help: "Try this if the wheel feels notchy or dead in one direction." },
    ],
  },
  {
    title: "DAMPER",
    keys: [
      { key: "DamperStrength", type: "int", min: 0, max: 100, label: "Damper strength" },
    ],
  },
  {
    title: "RUMBLE",
    keys: [
      { key: "ReverseRumble", type: "bool", label: "Swap rumble motors" },
    ],
  },
  {
    title: "ASYMMETRIC FORCE",
    blurb: "Only used when “Use per-direction limits” is on up top. Left values are usually negative.",
    keys: [
      { key: "AlternativeMinForceLeft", type: "int", min: -100, max: 100, label: "Left minimum" },
      { key: "AlternativeMaxForceLeft", type: "int", min: -100, max: 100, label: "Left maximum" },
      { key: "AlternativeMinForceRight", type: "int", min: -100, max: 100, label: "Right minimum" },
      { key: "AlternativeMaxForceRight", type: "int", min: -100, max: 100, label: "Right maximum" },
    ],
  },
  {
    title: "CABINET OUTPUTS",
    keys: [
      { key: "OutputsSystem", type: "int", min: 0, max: 20, label: "Outputs system" },
      { key: "MaxScaleOutput", type: "int", min: 0, max: 255, label: "Output scale" },
      { key: "NetOutputsWithLF", type: "bool", label: "Line feed on network outputs" },
      { key: "NetOutputsTCPPort", type: "number", min: 0, max: 65535, label: "TCP port" },
      { key: "NetOutputsUDPBroadcastPort", type: "number", min: 0, max: 65535, label: "UDP broadcast port" },
    ],
  },
  {
    title: "DIAGNOSTICS",
    keys: [
      { key: "Logging", type: "bool", label: "Write log file" },
      { key: "BeepWhenHook", type: "bool", label: "Beep when the plugin hooks" },
      { key: "ForceShowDeviceGUIDMessageBox", type: "bool", label: "Show device GUID on launch" },
      {
        key: "disableInGameGui",
        type: "bool",
        label: "Hide in-game overlay",
        default: "1",
        help: "Enabled automatically the first time Blaster Master finds each INI. Later changes are respected.",
      },
    ],
  },
];

const KNOWN = new Set(SCHEMA.flatMap((g) => g.keys.map((k) => k.key.toLowerCase())));
const DEFINITIONS = new Map(
  SCHEMA.flatMap((g) => g.keys).map((definition) => [definition.key.toLowerCase(), definition])
);
const CANONICAL_KEYS = new Map(
  SCHEMA.flatMap((g) => g.keys).map((definition) => [definition.key.toLowerCase(), definition.key])
);

const state = {
  settings: {
    root_path: "",
    compatibility_paths: [],
    teknoparrot_path: "",
    default_device_guid: "",
    backup_on_save: true,
    hide_gui_initialized_paths: [],
  },
  games: [],
  checked: new Set(),
  current: null,   // { name, ini_path }
  original: {},    // key -> value as read from disk
  pending: {},     // key -> edited value
  extras: [],      // keys present in the file but not in SCHEMA
  presets: {},
  devices: [],     // attached wheels/pads, already resolved to a GUID by Rust
  profiles: [],    // TeknoParrot UserProfiles, with their FFB Blaster state
  ffbChecked: new Set(),
  legacyInstalls: [],
  legacyChecked: new Set(),
  hasScanned: false,
  launchMonitorTimer: null,
  launchMonitorToken: 0,
  scanBusy: false,
  startingUp: true,
};

const $ = (id) => document.getElementById(id);

function setStatus(text, kind = "", details = "") {
  const el = $("status");
  el.textContent = text;
  el.title = details || text;
  el.className = "status" + (kind ? " " + kind : "");
}

function paintFrame() {
  return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}

async function startupStep(text, percent) {
  if (!state.startingUp) return;
  const progress = Math.max(0, Math.min(100, Number(percent) || 0));
  $("startupStage").textContent = text;
  $("startupBar").style.width = `${progress}%`;
  $("startupProgress").setAttribute("aria-valuenow", String(progress));
  await paintFrame();
}

async function finishStartup() {
  await startupStep("Ready", 100);
  state.startingUp = false;
  const overlay = $("startupOverlay");
  overlay.classList.add("done");
  setTimeout(() => { overlay.hidden = true; }, 240);
}

async function failStartup(error) {
  await startupStep(`Startup problem: ${error}`, 100);
  state.startingUp = false;
  const overlay = $("startupOverlay");
  overlay.classList.add("done");
  setTimeout(() => { overlay.hidden = true; }, 1400);
}

function dirtyKeys() {
  return Object.keys(state.pending).filter(
    (k) => String(state.pending[k]) !== String(state.original[k] ?? "")
  );
}

function refreshButtons() {
  const n = dirtyKeys().length;
  const hasGame = !!state.current && !state.current.read_only;
  const writableChecked = [...state.checked].filter(
    (path) => !state.games.find((g) => g.ini_path === path)?.read_only
  ).length;
  $("saveBtn").disabled = !hasGame || n === 0;
  $("revertBtn").disabled = !hasGame || n === 0;
  $("applyCheckedBtn").disabled = n === 0 || writableChecked === 0;
  $("copyAllBtn").disabled = !hasGame || writableChecked === 0;
  if (hasGame) {
    setStatus(n === 0 ? "No unsaved changes" : `${n} unsaved change${n === 1 ? "" : "s"}`, n ? "warn" : "");
  }
}

// ----------------------------------------------------------------- devices
// The GUID FFBBlaster wants is an SDL2 joystick GUID. We derive one from what
// Windows reports about each attached device, but a GUID that is *already*
// working somewhere in the user's library is better evidence than anything we
// derive, so when an attached device matches one of those by vendor/product id
// we offer the on-disk value instead and say so.

function renderDevices() {
  const sel = $("deviceSelect");
  const keep = sel.value || state.settings.default_device_guid || "";
  sel.innerHTML = "";

  const first = document.createElement("option");
  first.value = "";
  first.textContent = state.devices.length
    ? `${state.devices.length} device${state.devices.length === 1 ? "" : "s"} detected — pick one`
    : "No wheels or pads detected";
  sel.append(first);

  state.devices.forEach((dev) => {
    const o = document.createElement("option");
    o.value = dev.guid;
    o.textContent = dev.kind === "confirmed" ? `${dev.name}  ✓` : dev.name;
    o.title = `${dev.product_string || dev.name}\nvendor ${hex4(dev.vendor_id)}  product ${hex4(dev.product_id)}\n${dev.guid}`;
    sel.append(o);
  });

  const selected = state.devices.find(
    (device) => device.guid.toLowerCase() === String(keep).toLowerCase()
  );
  sel.value = selected?.guid || "";
  if (selected) $("globalGuid").value = selected.guid;
  sel.disabled = state.devices.length === 0;
}

function preferredDevice() {
  const selectedGuid = $("deviceSelect").value || state.settings.default_device_guid || "";
  return state.devices.find(
    (device) => device.guid.toLowerCase() === String(selectedGuid).toLowerCase()
  ) || null;
}

function fillPreferredGuidIntoCurrent() {
  if (!state.current || state.current.read_only) return null;
  const device = preferredDevice();
  if (!device) return null;
  const original = String(state.original.DeviceGUID ?? "");
  const previous = String(state.pending.DeviceGUID ?? "");
  state.pending.DeviceGUID = device.guid;
  return {
    device,
    formChanged: previous.toLowerCase() !== device.guid.toLowerCase(),
    diskChanged: original.toLowerCase() !== device.guid.toLowerCase(),
  };
}

function hex4(n) {
  return "0x" + Number(n).toString(16).padStart(4, "0");
}

function setDeviceNote(text, kind = "") {
  const el = $("deviceNote");
  el.textContent = text;
  el.className = "device-note" + (kind ? " " + kind : "");
}

function dedupeGamePaths(values) {
  const paths = Array.isArray(values) ? values : String(values || "").split(/\r?\n/);
  const seen = new Set();
  const unique = [];
  paths.forEach((value) => {
    const path = String(value || "").trim();
    const key = normalizePath(path);
    if (!key || seen.has(key)) return;
    seen.add(key);
    unique.push(path);
  });
  return unique;
}

function gamePaths() {
  return dedupeGamePaths($("gamePaths").value);
}

function setGamePaths(paths) {
  const unique = dedupeGamePaths(paths);
  $("gamePaths").value = unique.join("\n");
  return unique;
}

function configuredGameLocations() {
  const paths = gamePaths();
  return {
    root: paths[0] || "",
    compatibilityPaths: paths.slice(1),
  };
}

function primaryGamePath() {
  return configuredGameLocations().root;
}

function compatibilityPaths() {
  return configuredGameLocations().compatibilityPaths;
}

function updatePathStates() {
  const locationCount = gamePaths().length;
  const gameBadge = $("gamePathState");
  gameBadge.textContent = locationCount
    ? `${locationCount} LOCATION${locationCount === 1 ? "" : "S"}`
    : "NONE";
  gameBadge.className = `path-state ${locationCount ? "set" : "unset"}`;

  const tpIsSet = Boolean($("tpPath").value.trim());
  const tpBadge = $("tpPathState");
  tpBadge.textContent = tpIsSet ? "SET" : "NOT SET";
  tpBadge.className = `path-state ${tpIsSet ? "set" : "unset"}`;
}

async function refreshDevices() {
  try {
    state.devices = await invoke("list_devices", {
      targets: state.games.map((g) => g.ini_path),
    });
    renderDevices();
    if (!state.devices.length) {
      setDeviceNote("Nothing detected. Plug the wheel in and power it on, then hit Refresh — or paste a GUID below.", "warn");
    } else {
      const device = preferredDevice();
      setDeviceNote(
        device
          ? `${device.name}: ${device.guid} — selected for newly opened games`
          : `Found ${state.devices.length} device${state.devices.length === 1 ? "" : "s"}. Pick one to fill the GUID below.`,
        device?.kind === "confirmed" ? "ok" : ""
      );
      const filled = fillPreferredGuidIntoCurrent();
      if (filled?.formChanged) {
        renderFields();
        refreshButtons();
      }
    }
  } catch (e) {
    state.devices = [];
    renderDevices();
    setDeviceNote(String(e), "warn");
  }
}

// --------------------------------------------- superseded FFB Arcade Plugin

function renderLegacyAlert() {
  const count = state.legacyInstalls.length;
  $("legacyAlert").hidden = count === 0;
  $("legacyAlertText").textContent = count
    ? `${count} old FFB Arcade Plugin install${count === 1 ? "" : "s"} may block FFB Blaster.`
    : "";
}

async function refreshLegacyPlugins(root, configuredCompatibilityPaths = compatibilityPaths()) {
  state.legacyInstalls = await invoke("find_legacy_plugins", {
    root,
    compatibilityPaths: configuredCompatibilityPaths,
  });
  renderLegacyAlert();
}

function openLegacyDialog() {
  state.legacyChecked = new Set(state.legacyInstalls.map((install) => install.folder));
  renderLegacyList();
  $("legacyDlg").showModal();
}

function renderLegacyList() {
  const host = $("legacyList");
  host.innerHTML = "";
  state.legacyInstalls.forEach((install) => {
    const row = document.createElement("label");
    row.className = "legacy-row";

    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = state.legacyChecked.has(install.folder);
    box.addEventListener("change", () => {
      box.checked
        ? state.legacyChecked.add(install.folder)
        : state.legacyChecked.delete(install.folder);
      updateLegacyCount();
    });

    const folder = document.createElement("span");
    folder.className = "legacy-folder";
    folder.textContent = install.folder;

    const files = document.createElement("span");
    files.className = "legacy-files";
    files.textContent = install.files
      .map((file) => `${file.name} — ${file.reason}`)
      .join(" · ");

    row.append(box, folder, files);
    host.append(row);
  });
  updateLegacyCount();
}

function updateLegacyCount() {
  const packages = state.legacyChecked.size;
  const files = state.legacyInstalls
    .filter((install) => state.legacyChecked.has(install.folder))
    .reduce((total, install) => total + install.files.length, 0);
  $("legacyCount").textContent = `${state.legacyInstalls.length} old install${state.legacyInstalls.length === 1 ? "" : "s"} found · ${packages} selected · ${files} files`;
  $("legacyRemoveBtn").disabled = packages === 0;
}

async function removeLegacyPlugins() {
  const selected = state.legacyInstalls.filter((install) =>
    state.legacyChecked.has(install.folder)
  );
  if (!selected.length) return;

  $("legacyDlg").close();
  const confirmed = await showConfirm({
    title: "Remove old FFB Arcade Plugin files?",
    summary: `Moving ${selected.reduce((total, install) => total + install.files.length, 0)} files from ${selected.length} game folder${selected.length === 1 ? "" : "s"}. A timestamped backup folder will remain in each location.`,
    rows: selected.map(
      (install) => `<span class="target">${escapeHtml(install.folder)}</span>${install.files.map((file) => `<span class="k">${escapeHtml(file.name)}</span>`).join(" · ")}`
    ),
    okLabel: "Move files to backups",
  });
  if (!confirmed) {
    $("legacyDlg").showModal();
    return setStatus("Legacy plugin removal cancelled.");
  }

  const root = primaryGamePath();
  const configuredCompatibilityPaths = compatibilityPaths();
  const completed = [];
  const failed = [];
  for (const install of selected) {
    try {
      completed.push(await invoke("quarantine_legacy_plugin", {
        root,
        compatibilityPaths: configuredCompatibilityPaths,
        folder: install.folder,
      }));
    } catch (error) {
      failed.push(`${install.folder}: ${error}`);
    }
  }
  let rescanError = "";
  try {
    await refreshLegacyPlugins(root);
  } catch (error) {
    rescanError = `Rescan: ${error}`;
    state.legacyInstalls = [];
    renderLegacyAlert();
  }

  const movedFiles = completed.reduce(
    (total, result) => total + result.moved_files.length,
    0
  );
  const completedText = `${completed.length} old FFB Plugin install${completed.length === 1 ? "" : "s"} (${movedFiles} file${movedFiles === 1 ? "" : "s"}) moved safely`;
  const problemCount = failed.length + (rescanError ? 1 : 0);
  const details = [
    ...completed.map((result) => `Backup: ${result.backup_folder}`),
    ...failed,
    rescanError,
  ].filter(Boolean).join("\n");
  setStatus(
    problemCount
      ? `${completedText}; ${problemCount} problem${problemCount === 1 ? "" : "s"}. Hover here for details.`
      : `${completedText}. Backups remain inside their game folders.`,
    problemCount ? "err" : "ok",
    details
  );
  if (!rescanError && state.legacyInstalls.length) {
    state.legacyChecked = new Set();
    renderLegacyList();
    $("legacyDlg").showModal();
  }
}

// ------------------------------------------------- FFB Blaster in TeknoParrot
// TeknoParrot only writes an FFBBlaster.ini for a game once its profile has the
// FFB Blaster switch turned on, which is why a fresh library scans up almost
// empty. This reads every profile and offers to flip that switch.

async function openFfbDialog() {
  const tp = $("tpPath").value.trim();
  if (!tp) return setStatus("Set the TeknoParrot emulator folder first.", "warn");
  try {
    state.profiles = await invoke("list_profiles", { teknoparrotPath: tp });
  } catch (e) {
    return setStatus(String(e), "err");
  }
  if (!state.profiles.length) {
    return setStatus("No TeknoParrot profiles found. Check the TeknoParrot emulator folder.", "warn");
  }
  state.ffbChecked = new Set();
  renderFfbList();
  $("ffbDlg").showModal();
}

function renderFfbList() {
  const host = $("ffbList");
  host.innerHTML = "";

  state.profiles.forEach((p) => {
    const row = document.createElement("div");
    row.className = "ffb-row" + (p.ffb_supported ? "" : " unsupported");

    const box = document.createElement("input");
    box.type = "checkbox";
    box.disabled = !p.ffb_supported || p.ffb_enabled;
    box.checked = state.ffbChecked.has(p.file);
    box.addEventListener("change", () => {
      box.checked ? state.ffbChecked.add(p.file) : state.ffbChecked.delete(p.file);
      updateFfbCount();
    });

    const name = document.createElement("span");
    name.className = "n";
    name.textContent = p.name;
    name.title = p.file;

    const tag = document.createElement("span");
    tag.className = "tag" + (p.ffb_enabled ? " on" : "");
    const iniReady = profileHasIni(p);
    tag.textContent = !p.ffb_supported
      ? "no FFB Blaster support"
      : p.ffb_enabled
      ? iniReady
        ? "on · INI ready"
        : "on · no INI found"
      : "off";

    row.append(box, name, tag);
    host.append(row);
  });
  updateFfbCount();
}

function normalizePath(value) {
  let path = String(value || "").trim().replaceAll("/", "\\");
  // Rust canonical paths on Windows can carry an extended-length prefix even
  // when the same path from TeknoParrot does not. Remove it before comparing so
  // a newly discovered INI immediately counts as that profile's INI.
  if (path.toLowerCase().startsWith("\\\\?\\unc\\")) {
    path = "\\\\" + path.slice(8);
  } else if (path.startsWith("\\\\?\\")) {
    path = path.slice(4);
  }
  return path.replace(/\\+$/, "").toLowerCase();
}

async function initializeHideGuiDefaults(games) {
  const remembered = Array.isArray(state.settings.hide_gui_initialized_paths)
    ? [...state.settings.hide_gui_initialized_paths]
    : [];
  const known = new Set(remembered.map(normalizePath).filter(Boolean));
  const unseen = games.filter(
    (game) => !known.has(normalizePath(game.ini_path))
  );
  const writable = unseen.filter((game) => !game.read_only);
  const readOnly = unseen.filter((game) => game.read_only);
  if (!writable.length) {
    return { changed: 0, unchanged: 0, failed: [], readOnly: readOnly.length };
  }

  const reports = await invoke("write_changes", {
    targets: writable.map((game) => game.ini_path),
    changes: [{ section: SECTION, key: "disableInGameGui", value: "1" }],
    backup: $("backupChk").checked,
  });
  reports.filter((report) => report.ok).forEach((report) => {
    const path = normalizePath(report.path);
    if (path && !known.has(path)) {
      remembered.push(report.path);
      known.add(path);
    }
  });
  state.settings.hide_gui_initialized_paths = remembered;
  await invoke("save_settings", { settings: state.settings });
  return {
    changed: reports.filter((report) => report.ok && report.changed).length,
    unchanged: reports.filter((report) => report.ok && !report.changed).length,
    failed: reports.filter((report) => !report.ok),
    readOnly: readOnly.length,
  };
}

function profileHasIni(profile) {
  const profileDir = normalizePath(profile.dir);
  if (!profileDir) return false;
  return state.games.some((game) => {
    const iniFolder = normalizePath(game.folder);
    return iniFolder && (profileDir === iniFolder || profileDir.startsWith(iniFolder + "\\"));
  });
}

function launchCandidates() {
  return state.profiles.filter(
    (profile) => profile.ffb_supported && profile.ffb_enabled
  );
}

function updateLaunchProfilePath() {
  const selected = launchCandidates().find(
    (profile) => profile.file === $("launchProfileSelect").value
  );
  $("launchProfileSelect").classList.toggle("ini-ready", !!selected && profileHasIni(selected));
  const readiness = selected
    ? profileHasIni(selected)
      ? "INI ready — launch to test saved settings."
      : "No INI — reach gameplay to create it automatically."
    : "";
  $("launchProfilePath").textContent = selected?.exe
    ? `${readiness} Game executable: ${selected.exe}`
    : selected
    ? `${readiness} No game executable is set in this profile.`
    : "";
  $("launchGoBtn").disabled = !selected;
}

function showLaunchSetup(message, targetId) {
  setStatus(message, "warn");
  $("launchSetupMessage").textContent = message;
  $("launchSetupDlg").addEventListener("close", () => $(targetId).focus(), { once: true });
  $("launchSetupDlg").showModal();
}

async function openLaunchDialog() {
  if (!primaryGamePath()) {
    return showLaunchSetup(
      "Add your main game library to TeknoParrot game folders, then click Scan before test launching a game.",
      "gamePaths"
    );
  }
  const tp = $("tpPath").value.trim();
  if (!tp) {
    return showLaunchSetup(
      "Set the TeknoParrot emulator folder, then click Scan before test launching a game.",
      "tpPath"
    );
  }
  if (!state.hasScanned) {
    return showLaunchSetup(
      "Click Scan first so the app knows which games have an INI and which need first gameplay.",
      "scanBtn"
    );
  }
  try {
    state.profiles = await invoke("list_profiles", { teknoparrotPath: tp });
  } catch (error) {
    return setStatus(String(error), "err");
  }

  const candidates = launchCandidates();
  const select = $("launchProfileSelect");
  select.innerHTML = "";
  candidates.forEach((profile) => {
    const option = document.createElement("option");
    option.value = profile.file;
    const iniReady = profileHasIni(profile);
    option.className = iniReady ? "ini-ready" : "";
    option.textContent = `${profile.name} — ${iniReady ? "INI ready" : "needs first launch"}`;
    select.append(option);
  });
  if (!candidates.length) {
    const option = document.createElement("option");
    option.textContent = "No FFB-enabled game profiles found";
    option.value = "";
    select.append(option);
  }
  select.disabled = candidates.length === 0;
  const ready = candidates.filter(profileHasIni).length;
  const missing = candidates.length - ready;
  $("launchCount").textContent = candidates.length
    ? `${candidates.length} FFB-enabled game${candidates.length === 1 ? "" : "s"} · ${ready} INI ready · ${missing} need first gameplay`
    : "Enable FFB Blaster for a supported profile before test launching it.";
  updateLaunchProfilePath();
  $("launchDlg").showModal();
}

async function launchSelectedProfile() {
  const profile = launchCandidates().find(
    (candidate) => candidate.file === $("launchProfileSelect").value
  );
  if (!profile) return;
  const iniReady = profileHasIni(profile);
  if (iniReady && state.current?.name === profile.name && dirtyKeys().length) {
    return setStatus(
      `Save or revert ${state.current.name}'s unsaved changes before test launching it.`,
      "warn"
    );
  }
  try {
    await invoke("launch_profile", {
      teknoparrotPath: $("tpPath").value.trim(),
      profileFile: profile.file,
    });
    $("launchDlg").close();
    if (iniReady) {
      stopLaunchMonitor();
      setStatus(
        `${profile.name} launched through TeknoParrot to test its saved FFB settings.`,
        "ok"
      );
    } else {
      startLaunchMonitor(profile);
    }
  } catch (error) {
    setStatus(String(error), "err");
  }
}

function stopLaunchMonitor() {
  state.launchMonitorToken += 1;
  if (state.launchMonitorTimer !== null) {
    clearTimeout(state.launchMonitorTimer);
    state.launchMonitorTimer = null;
  }
}

function mergeDiscoveredGames(games) {
  games.forEach((game) => {
    const path = normalizePath(game.ini_path);
    if (!path) return;
    const existing = state.games.findIndex(
      (candidate) => normalizePath(candidate.ini_path) === path
    );
    if (existing >= 0) {
      state.games[existing] = game;
    } else {
      state.games.push(game);
    }
  });
  state.games.sort((a, b) => a.name.localeCompare(b.name));
  renderGames();
}

function addOrUpdateDiscoveredGame(game) {
  mergeDiscoveredGames([game]);
}

function startLaunchMonitor(profile) {
  stopLaunchMonitor();
  const token = state.launchMonitorToken;
  setStatus(
    `${profile.name} launched. Watching automatically for its FFBBlaster.ini — reach gameplay briefly.`,
    "ok"
  );

  const check = async () => {
    if (token !== state.launchMonitorToken) return;
    try {
      const game = await invoke("find_profile_ini", {
        root: primaryGamePath(),
        compatibilityPaths: compatibilityPaths(),
        teknoparrotPath: $("tpPath").value.trim(),
        profileFile: profile.file,
      });
      if (token !== state.launchMonitorToken) return;
      if (game) {
        stopLaunchMonitor();
        addOrUpdateDiscoveredGame(game);
        setStatus(
          `${profile.name} created FFBBlaster.ini and was added automatically. Exit the game before editing it.`,
          "ok"
        );
        return;
      }
      state.launchMonitorTimer = setTimeout(check, 3000);
    } catch (error) {
      if (token !== state.launchMonitorToken) return;
      stopLaunchMonitor();
      setStatus(`Automatic INI watch stopped: ${error}`, "err");
    }
  };
  check();
}

function updateFfbCount() {
  const off = state.profiles.filter((p) => p.ffb_supported && !p.ffb_enabled).length;
  const on = state.profiles.filter((p) => p.ffb_enabled).length;
  const unsupported = state.profiles.filter((p) => !p.ffb_supported).length;
  const ready = state.profiles.filter((p) => p.ffb_enabled && profileHasIni(p)).length;
  const n = state.ffbChecked.size;
  $("ffbCount").textContent = `${state.profiles.length} profiles · ${on} on · ${off} off · ${unsupported} unsupported · ${ready} INIs found here · ${n} checked`;
  $("ffbApply").disabled = n === 0;
}

async function applyFfb() {
  const files = [...state.ffbChecked];
  if (!files.length) return;
  const names = state.profiles.filter((p) => files.includes(p.file)).map((p) => p.name);

  // Both are modal <dialog>s, so step out of this one while confirming.
  $("ffbDlg").close();
  const ok = await showConfirm({
    title: "Enable FFB Blaster in TeknoParrot",
    summary: `Editing ${files.length} TeknoParrot profile${files.length === 1 ? "" : "s"}. Only the FFB Blaster switch changes.`,
    rows: names.map(
      (n) => `<span class="k">${escapeHtml(n)}</span> &rarr; <span class="new">FFB Blaster on</span>`
    ),
    okLabel: `Write ${files.length} profile${files.length === 1 ? "" : "s"}`,
  });
  if (!ok) {
    $("ffbDlg").showModal();
    return setStatus("Cancelled.");
  }

  try {
    const reports = await invoke("set_ffb_blaster", {
      files,
      enable: true,
      backup: $("backupChk").checked,
    });
    const failed = reports.filter((r) => !r.ok);
    const changed = reports.filter((r) => r.ok && r.changed);
    const unchanged = reports.filter((r) => r.ok && !r.changed);
    const details = failed.map((r) => `${r.path}: ${r.message}`).join(" | ");
    setStatus(
      failed.length
        ? `${changed.length} enabled, ${unchanged.length} already on, ${failed.length} failed — ${details}`
        : `FFB Blaster enabled for ${changed.length} game${changed.length === 1 ? "" : "s"}${unchanged.length ? `; ${unchanged.length} already on` : ""}. Run each newly enabled game and reach gameplay so it writes FFBBlaster.ini, then Scan again.`,
      failed.length ? "err" : "ok"
    );
    state.profiles = await invoke("list_profiles", { teknoparrotPath: $("tpPath").value.trim() });
    state.ffbChecked = new Set();
    renderFfbList();
    $("ffbDlg").showModal();
  } catch (e) {
    setStatus(String(e), "err");
  }
}

// --------------------------------------------------------------- game list

function renderGames() {
  const filter = $("gameSearch").value.trim().toLowerCase();
  const list = $("gameList");
  list.innerHTML = "";

  state.games
    .filter((g) => !filter || g.name.toLowerCase().includes(filter))
    .forEach((g) => {
      const li = document.createElement("li");
      if (state.current && state.current.ini_path === g.ini_path) li.classList.add("active");

      const box = document.createElement("input");
      box.type = "checkbox";
      box.disabled = g.read_only;
      box.checked = !g.read_only && state.checked.has(g.ini_path);
      box.title = "Include in bulk actions";
      box.addEventListener("click", (e) => {
        e.stopPropagation();
        box.checked ? state.checked.add(g.ini_path) : state.checked.delete(g.ini_path);
        refreshButtons();
      });

      const name = document.createElement("span");
      name.className = "name";
      name.title = g.folder;
      const title = document.createElement("span");
      title.textContent = g.name;
      name.append(title);

      li.append(box, name);
      if (g.read_only) {
        const ro = document.createElement("span");
        ro.className = "ro";
        ro.textContent = "read-only";
        li.append(ro);
      }
      li.addEventListener("click", () => selectGame(g));
      list.append(li);
    });

  $("gameCount").textContent = state.games.length
    ? `${state.games.length} editable INI${state.games.length === 1 ? "" : "s"}`
    : "No editable INIs found";
}

function setScanPathControlsDisabled(disabled) {
  ["gamePaths", "gameBrowseBtn", "scanBtn", "tpPath", "tpBrowseBtn"]
    .forEach((id) => { $(id).disabled = disabled; });
}

async function scan() {
  if (state.scanBusy) return;
  const configuredPaths = setGamePaths(gamePaths());
  const root = configuredPaths[0] || "";
  if (!root) return setStatus("Add your main TeknoParrot game library on the first line.", "warn");
  const configuredCompatibilityPaths = configuredPaths.slice(1);
  const teknoparrotPath = $("tpPath").value.trim();
  const scanLocationCount = 1 + configuredCompatibilityPaths.length;
  const scanScope = scanLocationCount === 1
    ? "the configured game location"
    : `${scanLocationCount} configured game locations`;
  updatePathStates();
  const dirty = dirtyKeys();
  if (dirty.length) {
    const discard = await showConfirm({
      title: "Discard unsaved changes?",
      summary: `Scanning again will close ${state.current.name} without saving ${dirty.length} change${dirty.length === 1 ? "" : "s"}.`,
      rows: dirty.map(
        (key) => `<span class="k">${escapeHtml(key)}</span> ` +
          `<span class="old">${escapeHtml(state.original[key] ?? "(absent)")}</span>` +
          ` &rarr; <span class="new">${escapeHtml(state.pending[key])}</span>`
      ),
      okLabel: "Discard and scan",
    });
    if (!discard) return setStatus("Scan cancelled.");
  }
  state.scanBusy = true;
  setScanPathControlsDisabled(true);
  try {
    state.hasScanned = false;
    await startupStep(
      `Scanning ${scanLocationCount} configured game location${scanLocationCount === 1 ? "" : "s"} for FFBBlaster.ini…`,
      45
    );
    setStatus(
      `Scanning ${scanLocationCount} configured game location${scanLocationCount === 1 ? "" : "s"} for FFBBlaster.ini…`
    );
    const scanResult = await invoke("scan_games", {
      root,
      compatibilityPaths: configuredCompatibilityPaths,
      teknoparrotPath,
    });
    state.games = Array.isArray(scanResult?.games) ? scanResult.games : [];
    const scanWarnings = Array.isArray(scanResult?.warnings) ? scanResult.warnings : [];
    await startupStep(`Found ${state.games.length} editable game INI${state.games.length === 1 ? "" : "s"}…`, 66);
    let hideGuiDefaults = { changed: 0, unchanged: 0, failed: [], readOnly: 0 };
    let hideGuiError = "";
    try {
      await startupStep("Applying first-run game defaults…", 73);
      hideGuiDefaults = await initializeHideGuiDefaults(state.games);
    } catch (error) {
      hideGuiError = String(error);
    }
    state.hasScanned = true;
    state.checked.clear();
    state.current = null;
    $("currentGame").textContent = "Pick a game";
    $("currentPath").textContent = "";
    $("fields").innerHTML = '<p class="empty">Choose a game from the list to edit its force feedback settings.</p>';
    renderGames();
    let legacyScanError = "";
    try {
      await startupStep("Checking for old FFB Plugin files…", 82);
      await refreshLegacyPlugins(root, configuredCompatibilityPaths);
    } catch (error) {
      state.legacyInstalls = [];
      renderLegacyAlert();
      legacyScanError = String(error);
    }
    const named = state.games.filter((g) => g.named_by_profile).length;
    const namedNote = teknoparrotPath
      ? named
        ? `, ${named} named from TeknoParrot`
        : ", but no TeknoParrot profiles matched — check the emulator folder and game paths"
      : "";
    const legacyNote = state.legacyInstalls.length
      ? ` ${state.legacyInstalls.length} old FFB Arcade Plugin install${state.legacyInstalls.length === 1 ? "" : "s"} found — review before launching those games.`
      : legacyScanError
      ? ` Old-plugin scan failed: ${legacyScanError}`
      : "";
    const initialized = hideGuiDefaults.changed + hideGuiDefaults.unchanged;
    const hideGuiProblems = hideGuiDefaults.failed.length + hideGuiDefaults.readOnly;
    const hideGuiNote = hideGuiError
      ? ` Hide-overlay initialization failed: ${hideGuiError}`
      : initialized || hideGuiProblems
      ? ` Hide in-game overlay initialized once for ${initialized} game${initialized === 1 ? "" : "s"}${hideGuiProblems ? `; ${hideGuiProblems} could not be initialized` : ""}.`
      : "";
    const pathWarningNote = scanWarnings.length
      ? ` ${scanWarnings.length} additional game folder${scanWarnings.length === 1 ? " was" : "s were"} skipped; hover for details.`
      : "";
    setStatus(
      (state.games.length
        ? `Found ${state.games.length} game folder${state.games.length === 1 ? "" : "s"} with FFBBlaster.ini in ${scanScope}${namedNote}`
        : `No FFBBlaster.ini found in ${scanScope}. Check the paths, then review any old-plugin warning before launching the game again.`) + legacyNote + hideGuiNote + pathWarningNote,
      state.legacyInstalls.length || legacyScanError || hideGuiError || hideGuiProblems || scanWarnings.length || !state.games.length || namedNote.startsWith(", but")
        ? "warn"
        : "ok",
      scanWarnings.join("\n")
    );
    state.settings.root_path = root;
    state.settings.compatibility_paths = configuredCompatibilityPaths;
    state.settings.teknoparrot_path = teknoparrotPath;
    await invoke("save_settings", { settings: state.settings });
    // Re-resolve now that we know which GUIDs the library already uses.
    await startupStep("Finding connected wheels and controllers…", 91);
    await refreshDevices();
    await startupStep("Preparing the game list…", 97);
  } catch (e) {
    setStatus(String(e), "err");
  } finally {
    state.scanBusy = false;
    setScanPathControlsDisabled(false);
  }
}

// ------------------------------------------------------------------ editor

async function selectGame(game, skipDirtyCheck = false) {
  const dirty = dirtyKeys();
  if (!skipDirtyCheck && dirty.length) {
    const ok = await showConfirm({
      title: "Discard unsaved changes?",
      summary: `${dirty.length} change${dirty.length === 1 ? "" : "s"} in ${state.current.name} have not been written to disk.`,
      rows: dirty.map(
        (k) =>
          `<span class="k">${escapeHtml(k)}</span>  ` +
          `<span class="old">${escapeHtml(state.original[k] ?? "(absent)")}</span>` +
          ` &rarr; <span class="new">${escapeHtml(state.pending[k])}</span>`
      ),
      okLabel: "Discard and switch",
    });
    if (!ok) return;
  }
  try {
    if (!game.read_only) {
      await initializeHideGuiDefaults([game]);
    }
    const file = await invoke("read_ini", { path: game.ini_path });
    state.current = game;
    state.original = {};
    file.entries
      .filter((e) => e.section.toLowerCase() === SECTION.toLowerCase())
      .forEach((e) => {
        const key = CANONICAL_KEYS.get(e.key.toLowerCase()) || e.key;
        state.original[key] = e.value;
      });
    state.pending = { ...state.original };
    state.extras = Object.keys(state.original).filter((k) => !KNOWN.has(k.toLowerCase()));
    const filledGuid = fillPreferredGuidIntoCurrent();

    $("currentGame").textContent = game.name;
    $("currentPath").textContent = game.ini_path + (file.read_only ? "  (read-only — view only)" : "");
    renderFields();
    renderGames();
    refreshButtons();
    if (filledGuid?.diskChanged) {
      setStatus(
        `${filledGuid.device.name} GUID filled into ${game.name}. Review it, then Save this game.`,
        "warn"
      );
    }
  } catch (e) {
    setStatus(String(e), "err");
  }
}

function settingRow(def) {
  const wrap = document.createElement("div");
  wrap.className = "setting";
  wrap.dataset.key = def.key;

  const label = document.createElement("label");
  label.textContent = def.label || def.key;
  const key = document.createElement("span");
  key.className = "key";
  key.textContent = def.key;

  const ctrl = document.createElement("div");
  ctrl.className = "ctrl";

  const value = state.pending[def.key] ?? def.default ?? "";
  const present = Object.prototype.hasOwnProperty.call(state.original, def.key);

  const onChange = (v) => {
    state.pending[def.key] = String(v);
    const error = validationError(def.key, String(v));
    wrap.classList.toggle("invalid", !!error);
    wrap.title = error || "";
    wrap.classList.toggle(
      "changed",
      String(v) !== String(state.original[def.key] ?? "")
    );
    refreshButtons();
  };

  if (def.type === "bool") {
    const t = document.createElement("label");
    t.className = "toggle";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.disabled = !!state.current?.read_only;
    cb.checked = String(value) === "1";
    cb.addEventListener("change", () => onChange(cb.checked ? "1" : "0"));
    const txt = document.createElement("span");
    txt.textContent = def.label || def.key;
    t.append(cb, txt);
    ctrl.append(t);
    wrap.append(ctrl, key);
  } else if (def.type === "number") {
    // Typed outright, no slider — dragging to a port number is nobody's idea
    // of a good time.
    const num = document.createElement("input");
    num.type = "number";
    num.className = "num";
    num.min = def.min;
    num.max = def.max;
    num.disabled = !!state.current?.read_only;
    num.value = value;
    num.addEventListener("input", () => onChange(num.value));
    ctrl.append(num);
    wrap.append(label, ctrl, key);
  } else if (def.type === "int") {
    const range = document.createElement("input");
    range.type = "range";
    range.min = def.min;
    range.max = def.max;
    range.value = Number(value) || 0;
    range.disabled = !!state.current?.read_only;
    const num = document.createElement("input");
    num.type = "number";
    num.className = "num";
    num.min = def.min;
    num.max = def.max;
    num.value = value;
    num.disabled = !!state.current?.read_only;
    range.addEventListener("input", () => { num.value = range.value; onChange(range.value); });
    num.addEventListener("input", () => { range.value = num.value; onChange(num.value); });
    ctrl.append(range, num);
    wrap.append(label, ctrl, key);
  } else {
    const input = document.createElement("input");
    input.type = "text";
    input.spellcheck = false;
    input.value = value;
    input.disabled = !!state.current?.read_only;
    input.style.flex = "1";
    input.addEventListener("input", () => onChange(input.value));
    ctrl.append(input);
    wrap.append(label, ctrl, key);
  }

  if (def.help) {
    const help = document.createElement("span");
    help.className = "help";
    help.textContent = def.help;
    wrap.append(help);
  }
  if (!present) {
    const help = document.createElement("span");
    help.className = "help";
    help.textContent = "Not in this file yet — saving adds it.";
    wrap.append(help);
  }
  return wrap;
}

function validationError(key, value) {
  if (/\r|\n/.test(value)) return `${key} cannot contain a new line.`;
  const definition = DEFINITIONS.get(key.toLowerCase());
  if (!definition) return "";
  if (definition.key === "DeviceGUID") {
    return /^[0-9a-fA-F]{32}$/.test(value.trim())
      ? ""
      : "DeviceGUID must be exactly 32 hexadecimal characters.";
  }
  if (definition.type === "bool") {
    return value === "0" || value === "1" ? "" : `${key} must be 0 or 1.`;
  }
  if (definition.type === "int" || definition.type === "number") {
    if (!/^-?\d+$/.test(value.trim())) return `${key} must be a whole number.`;
    const number = Number(value);
    if (!Number.isSafeInteger(number) || number < definition.min || number > definition.max) {
      return `${key} must be from ${definition.min} to ${definition.max}.`;
    }
  }
  return "";
}

function validateChanges(changes) {
  for (const change of changes) {
    const error = validationError(change.key, change.value);
    if (error) return error;
  }
  return "";
}

function renderFields() {
  const host = $("fields");
  host.innerHTML = "";

  const groups = SCHEMA.slice();
  if (state.extras.length) {
    groups.push({
      title: "OTHER KEYS IN THIS FILE",
      blurb: "Keys this app has no dedicated control for, including lamp and output names. Edited as raw text and written back exactly as typed.",
      keys: state.extras.map((k) => ({ key: k, type: "text", label: k })),
    });
  }

  groups.forEach((g) => {
    const sec = document.createElement("section");
    sec.className = "group";
    const h = document.createElement("h3");
    h.textContent = g.title;
    sec.append(h);
    if (g.blurb) {
      const p = document.createElement("p");
      p.className = "blurb";
      p.textContent = g.blurb;
      sec.append(p);
    }
    const grid = document.createElement("div");
    grid.className = "grid";
    g.keys.forEach((def) => grid.append(settingRow(def)));
    sec.append(grid);
    host.append(sec);
  });
}

// -------------------------------------------------------------------- saving

function changesFrom(keys) {
  return keys.map((key) => ({ section: SECTION, key, value: String(state.pending[key]) }));
}

// Every confirmation goes through this dialog rather than the webview's native
// confirm(): a bulk write was once observed going ahead with no dialog shown,
// and this way the prompt is styled like the rest of the app, can list what is
// about to change, and resolves true only on an actual click.
function showConfirm({ title, summary, rows, okLabel }) {
  return new Promise((resolve) => {
    const dlg = $("confirmDlg");
    $("confirmTitle").textContent = title;
    $("confirmSummary").textContent = summary;
    const diff = $("confirmDiff");
    diff.innerHTML = "";
    rows.forEach((html) => {
      const row = document.createElement("div");
      row.innerHTML = html;
      diff.append(row);
    });
    let settled = false;
    const done = (ok) => {
      if (settled) return;
      settled = true;
      dlg.onclose = null;
      $("confirmOk").onclick = null;
      $("confirmCancel").onclick = null;
      if (dlg.open) dlg.close();
      resolve(ok);
    };
    $("confirmOk").onclick = () => done(true);
    $("confirmCancel").onclick = () => done(false);
    // Esc closes a <dialog> without either button; treat that as a cancel.
    dlg.onclose = () => done(false);
    $("confirmOk").textContent = okLabel;
    dlg.showModal();
  });
}

async function confirmWrite(title, summary, changes, targets) {
  const previews = await invoke("preview_changes", { targets, changes });
  const ready = previews.filter((report) => report.ok && report.changes.length);
  const unchanged = previews.filter((report) => report.ok && !report.changes.length);
  const blocked = previews.filter((report) => !report.ok);
  if (!ready.length) {
    const reason = blocked.length
      ? blocked.map((report) => `${report.name}: ${report.message}`).join(" | ")
      : "Every selected file already has those values.";
    setStatus(reason, blocked.length ? "err" : "ok");
    return false;
  }
  const rows = [];
  previews.forEach((report) => {
    rows.push(
      `<span class="target ${report.ok ? "" : "blocked"}">${escapeHtml(report.name)}</span>` +
      (report.ok ? "" : ` — <span class="old">${escapeHtml(report.message)}</span>`)
    );
    report.changes.forEach((change) => {
      rows.push(
        `<span class="k">${escapeHtml(change.key)}</span> ` +
        `<span class="old">${escapeHtml(change.old ?? "(absent)")}</span>` +
        ` &rarr; <span class="new">${escapeHtml(change.new)}</span>`
      );
    });
  });
  return showConfirm({
    title,
    summary: `${summary} ${ready.length} will change, ${unchanged.length} already match, ${blocked.length} blocked.`,
    rows,
    okLabel: `Write ${ready.length} file${ready.length === 1 ? "" : "s"}`,
  });
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

async function write(changes, targets, title, summary) {
  if (!changes.length) return setStatus("Nothing to write.", "warn");
  const validation = validateChanges(changes);
  if (validation) return setStatus(validation, "err");
  let confirmed;
  try {
    confirmed = await confirmWrite(title, summary, changes, targets);
  } catch (e) {
    return setStatus(String(e), "err");
  }
  if (!confirmed) return;
  try {
    const reports = await invoke("write_changes", {
      targets,
      changes,
      backup: $("backupChk").checked,
    });
    const failed = reports.filter((r) => !r.ok);
    const changed = reports.filter((r) => r.ok && r.changed);
    const unchanged = reports.filter((r) => r.ok && !r.changed);
    const currentSaved = state.current && reports.some(
      (report) => report.ok && report.path === state.current.ini_path
    );
    if (currentSaved) await selectGame(state.current, true);
    const details = failed.map((report) => `${report.path}: ${report.message}`).join(" | ");
    setStatus(
      failed.length
        ? `${changed.length} changed, ${unchanged.length} already matched, ${failed.length} failed — ${details}`
        : `${changed.length} file${changed.length === 1 ? "" : "s"} changed${unchanged.length ? `; ${unchanged.length} already matched` : ""}.`,
      failed.length ? "err" : "ok"
    );
  } catch (e) {
    setStatus(String(e), "err");
  }
}

// ---------------------------------------------------------------- presets

function renderPresets() {
  const sel = $("presetSelect");
  const keep = sel.value;
  sel.innerHTML = '<option value="">Presets</option>';
  Object.keys(state.presets).sort().forEach((name) => {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = name;
    sel.append(o);
  });
  sel.value = keep in state.presets ? keep : "";
}

// -------------------------------------------------------------------- wiring

async function openDonationPage() {
  if (!openUrl) {
    return setStatus("The system-browser connection is unavailable.", "err");
  }
  const button = $("donateBtn");
  button.disabled = true;
  try {
    await openUrl(DONATE_URL);
    setStatus("Opened the optional PayPal support page in your default browser.", "ok");
  } catch (error) {
    setStatus(`Could not open the PayPal support page: ${error}`, "err");
  } finally {
    button.disabled = false;
  }
}

function wire() {
  $("gamePaths").addEventListener("input", () => {
    stopLaunchMonitor();
    updatePathStates();
    state.hasScanned = false;
    state.legacyInstalls = [];
    renderLegacyAlert();
  });
  $("gamePaths").addEventListener("blur", () => {
    setGamePaths(gamePaths());
    updatePathStates();
  });
  $("tpPath").addEventListener("input", () => {
    stopLaunchMonitor();
    updatePathStates();
    state.hasScanned = false;
  });

  $("gameBrowseBtn").addEventListener("click", async () => {
    if (!dialog) return setStatus("Folder picker unavailable — type one game folder per line.", "warn");
    const picked = await dialog.open({
      directory: true,
      multiple: false,
      title: "Add TeknoParrot game folder",
    });
    if (!picked) return;

    const folder = Array.isArray(picked) ? picked[0] : picked;
    const before = gamePaths();
    const after = dedupeGamePaths([...before, folder]);
    if (after.length === before.length) {
      setGamePaths(before);
      updatePathStates();
      return setStatus("That game folder is already listed.", "warn");
    }

    setGamePaths(after);
    updatePathStates();
    stopLaunchMonitor();
    state.hasScanned = false;
    state.legacyInstalls = [];
    renderLegacyAlert();
    setStatus(
      before.length
        ? `Added standalone game folder: ${folder}. Click Scan to include it.`
        : `Added main game library: ${folder}. Add more folders or click Scan.`
    );
  });

  $("tpBrowseBtn").addEventListener("click", async () => {
    if (!dialog) return setStatus("Folder picker unavailable — type the path instead.", "warn");
    const picked = await dialog.open({ directory: true, multiple: false, title: "TeknoParrot emulator folder" });
    if (picked) {
      $("tpPath").value = picked;
      updatePathStates();
      if (primaryGamePath()) await scan();
    }
  });

  $("donateBtn").addEventListener("click", openDonationPage);
  $("ffbBtn").addEventListener("click", openFfbDialog);
  $("ffbCancel").addEventListener("click", () => $("ffbDlg").close());
  $("ffbApply").addEventListener("click", applyFfb);
  $("launchBtn").addEventListener("click", openLaunchDialog);
  $("launchCancelBtn").addEventListener("click", () => $("launchDlg").close());
  $("launchProfileSelect").addEventListener("change", updateLaunchProfilePath);
  $("launchGoBtn").addEventListener("click", launchSelectedProfile);
  $("launchSetupOkBtn").addEventListener("click", () => $("launchSetupDlg").close());

  $("legacyReviewBtn").addEventListener("click", openLegacyDialog);
  $("legacyCancelBtn").addEventListener("click", () => $("legacyDlg").close());
  $("legacyNoneBtn").addEventListener("click", () => {
    state.legacyChecked.clear();
    renderLegacyList();
  });
  $("legacyRemoveBtn").addEventListener("click", removeLegacyPlugins);

  $("ffbAll").addEventListener("click", () => {
    state.profiles
      .filter((p) => p.ffb_supported && !p.ffb_enabled)
      .forEach((p) => state.ffbChecked.add(p.file));
    renderFfbList();
  });

  $("ffbNone").addEventListener("click", () => {
    state.ffbChecked.clear();
    renderFfbList();
  });

  $("scanBtn").addEventListener("click", scan);
  $("gameSearch").addEventListener("input", renderGames);

  $("refreshDevicesBtn").addEventListener("click", refreshDevices);

  $("deviceSelect").addEventListener("change", async () => {
    const dev = state.devices.find((d) => d.guid === $("deviceSelect").value);
    if (!dev) return;
    $("globalGuid").value = dev.guid;
    state.settings.default_device_guid = dev.guid;
    let rememberError = "";
    try {
      await invoke("save_settings", { settings: state.settings });
    } catch (error) {
      rememberError = String(error);
    }
    setDeviceNote(
      `${dev.name}: ${dev.guid}${dev.note ? ` — ${dev.note}` : ""}${rememberError ? " — could not remember selection" : " — remembered for newly opened games"}`,
      rememberError ? "warn" : dev.kind === "confirmed" ? "ok" : dev.kind === "on-disk" ? "warn" : ""
    );

    if (!state.current) {
      return setStatus(
        rememberError
          ? `${dev.name} selected, but the selection could not be remembered: ${rememberError}`
          : `${dev.name} selected and remembered. Open a game to fill its Device GUID, or use Set on every game.`,
        rememberError ? "err" : "ok"
      );
    }
    if (state.current.read_only) {
      return setStatus(
        rememberError
          ? `${dev.name} selected, but ${state.current.name} is read-only and the selection could not be remembered: ${rememberError}`
          : `${dev.name} selected and remembered, but ${state.current.name} is read-only.`,
        rememberError ? "err" : "warn"
      );
    }

    const filled = fillPreferredGuidIntoCurrent();
    if (filled) {
      renderFields();
      refreshButtons();
    }
    setStatus(
      rememberError
        ? `${dev.name} filled into the form, but the selection could not be remembered: ${rememberError}`
        : !filled?.diskChanged
        ? `${state.current.name} already uses the selected ${dev.name} GUID.`
        : `${dev.name} GUID filled into ${state.current.name}. Review it, then Save this game.`,
      rememberError ? "err" : !filled?.diskChanged ? "ok" : "warn"
    );
  });

  $("checkAllBtn").addEventListener("click", () => {
    state.games.filter((g) => !g.read_only).forEach((g) => state.checked.add(g.ini_path));
    renderGames();
    refreshButtons();
  });

  $("checkNoneBtn").addEventListener("click", () => {
    state.checked.clear();
    renderGames();
    refreshButtons();
  });

  $("saveBtn").addEventListener("click", () => {
    const keys = dirtyKeys();
    write(changesFrom(keys), [state.current.ini_path], "Save this game", `${keys.length} key(s) in ${state.current.name}.`);
  });

  $("applyCheckedBtn").addEventListener("click", () => {
    const keys = dirtyKeys();
    const targets = [...state.checked].filter(
      (path) => !state.games.find((g) => g.ini_path === path)?.read_only
    );
    write(changesFrom(keys), targets, "Apply changes to checked games", `${keys.length} changed key(s) written to ${targets.length} game(s).`);
  });

  $("copyAllBtn").addEventListener("click", () => {
    const keys = SCHEMA.flatMap((g) => g.keys.map((k) => k.key)).filter(
      (k) => state.pending[k] !== undefined && state.pending[k] !== ""
    );
    const targets = [...state.checked].filter(
      (path) => !state.games.find((g) => g.ini_path === path)?.read_only
    );
    write(changesFrom(keys), targets, "Copy supported settings", `${keys.length} supported settings from ${state.current.name} to ${targets.length} game(s).`);
  });

  $("revertBtn").addEventListener("click", () => {
    state.pending = { ...state.original };
    renderFields();
    refreshButtons();
    setStatus("Reverted to the values on disk.");
  });

  $("applyGuidBtn").addEventListener("click", async () => {
    const guid = $("globalGuid").value.trim();
    if (!/^[0-9a-fA-F]{32}$/.test(guid)) {
      return setStatus("A device GUID is 32 hex characters. Copy it from FFBBlaster's own GUI.", "warn");
    }
    if (!state.games.length) return setStatus("Scan for games first.", "warn");
    state.settings.default_device_guid = guid;
    await invoke("save_settings", { settings: state.settings });
    const changes = [{ section: SECTION, key: "DeviceGUID", value: guid }];
    const targets = state.games.filter((g) => !g.read_only).map((g) => g.ini_path);
    write(changes, targets, "Set device GUID everywhere", `DeviceGUID written to all ${targets.length} game(s).`);
  });

  $("presetSaveBtn").addEventListener("click", async () => {
    if (!state.current) return setStatus("Open a game first.", "warn");
    const name = prompt("Preset name", state.current.name);
    if (!name) return;
    const values = {};
    SCHEMA.flatMap((g) => g.keys.map((k) => k.key)).forEach((k) => {
      if (k !== "DeviceGUID" && state.pending[k] !== undefined) values[k] = state.pending[k];
    });
    state.presets[name] = values;
    await invoke("save_presets", { presets: state.presets });
    renderPresets();
    setStatus(`Preset "${name}" saved.`, "ok");
  });

  $("presetApplyBtn").addEventListener("click", () => {
    const name = $("presetSelect").value;
    if (!name || !state.current) return setStatus("Pick a preset and a game.", "warn");
    Object.entries(state.presets[name]).forEach(([k, v]) => {
      const key = CANONICAL_KEYS.get(k.toLowerCase()) || k;
      state.pending[key] = v;
    });
    fillPreferredGuidIntoCurrent();
    renderFields();
    refreshButtons();
    setStatus(`Loaded "${name}" into the form. Nothing is written until you save.`, "warn");
  });

  $("presetDeleteBtn").addEventListener("click", async () => {
    const name = $("presetSelect").value;
    if (!name) return;
    delete state.presets[name];
    await invoke("save_presets", { presets: state.presets });
    renderPresets();
    setStatus(`Preset "${name}" deleted.`);
  });

  $("backupChk").addEventListener("change", async () => {
    state.settings.backup_on_save = $("backupChk").checked;
    await invoke("save_settings", { settings: state.settings });
  });
}

async function init() {
  if (!invoke) {
    setStatus("Tauri bridge missing — open this through the app, not a browser.", "err");
    await failStartup("The desktop connection is unavailable.");
    return;
  }
  try {
    wire();
    await startupStep("Loading saved settings and presets…", 18);
    [state.settings, state.presets] = await Promise.all([
      invoke("load_settings"),
      invoke("load_presets"),
    ]);
    setGamePaths([
      state.settings.root_path || "",
      ...(Array.isArray(state.settings.compatibility_paths) ? state.settings.compatibility_paths : []),
    ]);
    $("tpPath").value = state.settings.teknoparrot_path || "";
    updatePathStates();
    $("globalGuid").value = state.settings.default_device_guid || "";
    $("backupChk").checked = state.settings.backup_on_save !== false;
    renderPresets();
    refreshButtons();
    if (primaryGamePath()) {
      await scan();
    } else {
      await startupStep("Finding connected wheels and controllers…", 65);
      await refreshDevices();
      await startupStep("Waiting for TeknoParrot game folders…", 95);
    }
    await finishStartup();
  } catch (error) {
    setStatus(String(error), "err");
    await failStartup(error);
  }
}

init();
