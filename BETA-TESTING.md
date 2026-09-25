# FFB Blaster MASTER 0.2.0 Beta 16

This is a portable beta. It does not need an installer.

## What Beta 16 includes

- Individual fixed-location games such as `C:\FNF` participate in scanning,
  friendly profile naming, first-INI watching, and old-plugin checks without
  searching the whole drive.
- **TeknoParrot game folders** combines the main library and fixed-path games
  into one ordered box. The first line is the main library searched recursively;
  every later line is one exact standalone game folder such as `C:\FNF`.
- Existing saved paths are migrated into the combined box in the same order,
  equivalent paths are deduplicated, and whole-drive entries such as `C:\` are
  rejected so the app never searches an entire drive.
- A **Donate / Buy me a coffee** button is available in the main app. It opens
  the exact PayPal link in the system browser; support remains optional and does
  not unlock features or priority assistance.
- Selecting **Test launch game** without a configured game-folder path or before
  scanning now gives direct guidance to set the folder and run **Scan**.
- **INI ready** games have a darker, quieter appearance in the launch picker so
  **needs first launch** games stand out. Ready games remain selectable and can
  still be launched to test their existing settings.
- The obsolete **Create missing starter INIs** action has been removed. FFB
  Blaster creates each game-specific INI after the game reaches actual gameplay,
  and Blaster Master continues watching for that file after a test launch.

Beta 16 also includes the still-current Beta 12 and earlier changes:

- Renamed **TeknoParrotUI folder** to **TeknoParrot emulator folder** throughout
  the interface so it is clearer that this is the main emulator installation
  containing `TeknoParrotUi.exe` and `UserProfiles`.

Beta 12 includes the Beta 11 changes:

- A startup overlay now shows progress while loading settings, scanning game
  folders, checking for old plugin files, and finding connected devices.
- Slow folder, profile, device, and INI work now runs away from the main window
  thread, so the progress bar remains animated during a long startup scan.
- Startup no longer performs the connected-device scan twice when a saved games
  folder triggers an automatic library scan.

Beta 11 also includes the Beta 10 changes:

- Selecting a detected wheel now remembers that device immediately; using
  **Set on every game** is no longer required to persist the selection.
- Opening another game automatically fills its **Device GUID** form field from
  the remembered device when needed.
- Loading a preset keeps/reapplies the remembered device GUID, since presets do
  not contain a device GUID.

Beta 10 also includes the Beta 9 changes:

- The first time this copy of Blaster Master discovers each writable
  `FFBBlaster.ini`, it sets `disableInGameGui=1` so the in-game overlay starts
  hidden.
- Initialized INI paths are remembered in `ffbblaster-master.settings.json`.
  Later user changes, including turning the overlay back on for one game, are
  respected on every following scan.
- Existing backup behavior applies to this one-time initialization. Read-only or
  failed files are left uninitialized and retried after the problem is fixed.

Beta 9 also includes the Beta 8 changes:

- **Test launch game** now lists every FFB-enabled profile.
- **INI ready** games can be launched to test their saved configuration.
- **Needs first launch** games retain automatic INI detection after reaching
  gameplay.
- The app warns instead of launching the currently open game when its settings
  are still unsaved.
- Resolved games show only their friendly XML name; the full path remains
  available by hovering over the name.

Beta 8 also includes the Beta 7 changes:

- Watches the test-launched game's folder automatically every three seconds.
- Adds the game to the editable list as soon as its first `FFBBlaster.ini`
  appears; no manual rescan is required.
- Targets only the launched game's folder and preserves another game's open form
  and unsaved edits.

Beta 7 included the Beta 6 changes:

- Moves the TeknoParrot enable/disable button into the title area.
- Adds **Test launch game**, which lists enabled profiles still missing an INI
  and launches the selected profile through TeknoParrot.
- Includes a gameplay tooltip and first-launch instructions in the picker.

Beta 6 included the pending Beta 5 changes:

- Selecting a detected wheel now fills the open game's **Device GUID** field as
  an unsaved change. Review it and choose **Save this game** for a one-game test.
- Corrects the setup instructions to require reaching actual gameplay before
  expecting a new `FFBBlaster.ini`; the Press Start screen may be too early.

Beta 5 included the Beta 4 changes:

- Keeps the result of a large old-plugin removal compact instead of printing
  every backup path across the bottom of the window.
- Prevents unusually long status and error messages from widening the interface.
- Shows the full list of backup paths as hover text on the status message when
  those details are needed.

Beta 4 included the Beta 3 changes:

- Finds `FFBBlaster.ini` in more deeply nested game folders.
- Detects verified remnants of the superseded **FFB Arcade Plugin**, which can
  prevent FFB Blaster from loading and creating its own INI.
- Lets you review the exact old files before removing them.
- Makes removal recoverable by moving those files into a timestamped backup
  folder inside the game folder.
- Leaves generic wrapper DLLs alone unless their embedded metadata identifies
  them as the old FFB Arcade Plugin.

Beta 3 also contains the Beta 2 GUID fix, stacked folder selectors, SET / NOT SET
indicators, and clearer TeknoParrot profile-versus-INI counts.

## Arctic Thunder test

1. Extract this ZIP to its own folder on the arcade PC. Do not overwrite an
   earlier beta.
2. Close both the game and `TeknoParrotUi.exe`.
3. Run `ffbblaster-master.exe` and confirm the title bar says **Beta 16**. Watch
   the startup overlay report each loading phase until the main interface is ready.
4. Leave **Back up before saving** checked.
5. Before entering a game-folder path or scanning, click **Test launch game**.
   Confirm the app explains that a game folder must be configured and scanned,
   and directs you to the correct controls instead of showing a generic failure.
6. In **TeknoParrot game folders**, put the main library on the first line. Add
   each fixed-path game, such as `C:\FNF`, on a later line, then type the
   **TeknoParrot emulator folder** path. Before scanning, click **Test launch game**
   once more and confirm it directs you to run **Scan**. Then scan and confirm
   games under the first line are found recursively and each later line is
   treated as one exact game location. On this first scan, note any status saying
   **Hide in-game overlay** was initialized for newly discovered games.
7. Add the same fixed game path again using different letter case or slash style,
   move focus out of the box, and confirm the duplicate is removed without
   changing the order. Restart the app and confirm the ordered folder list is
   preserved and scanned automatically.
8. Temporarily enter `C:\` as the first line and confirm **Scan** refuses to search
   it. Restore the main library, add `C:\` on a later line, and confirm it is
   skipped with a warning rather than searched. Remove the test entry afterward.
9. Click **Donate / Buy me a coffee** and confirm the system browser opens
   `https://www.paypal.com/paypalme/acbauer12/9.99`. Do not complete a payment;
   return to the app and confirm it remains responsive.
10. Scan again and confirm **Hide in-game overlay** is not initialized a second
    time or forced back on for a game whose checkbox was later changed manually.
11. Select the MOZA wheel once, open a different game, and load a preset. The
   MOZA GUID should remain populated as an unsaved game change without toggling
   to another device and back.
12. If the old-plugin warning appears, click **Review old plugin files**. Confirm
   that the folder and filenames belong to Arctic Thunder before selecting
   **Remove selected safely**.
13. Read the confirmation carefully. On approval, the files are moved into a
   folder named `_FFBPlugin_legacy_backup_...`; they are not erased.
14. Confirm Arctic Thunder's TeknoParrot profile has FFB Blaster enabled.
15. Open **Test launch game** and confirm each **INI ready** row is visibly darker
    than a **needs first launch** row but remains enabled and selectable. Launch
    one ready game and confirm its existing settings can still be tested.
16. Reopen **Test launch game**, select an enabled **needs first launch** game, and
    launch it. Confirm there is no manual starter-INI creation action. Let the game
    reach actual gameplay so FFB Blaster creates `FFBBlaster.ini`, then exit the
    game and close TeknoParrotUI.
17. Keep FFB Blaster MASTER open during that first launch. The game should be added
    automatically when its new `FFBBlaster.ini` appears, without a manual rescan.
    Exit the game before editing the new entry.

## If the game stops launching

Close TeknoParrotUI, open the affected game's `_FFBPlugin_legacy_backup_...`
folder, and move its contents back to the parent game folder. Do not keep two
copies of the same wrapper DLL in that parent folder.

Please record the exact game/profile, the old files listed, the backup-folder
path shown in the status bar, and any full error message if the result differs.

## Support development

If this beta helps your cabinet and you would like to support continued development,
you can [buy me a coffee through PayPal ($9.99)](https://www.paypal.com/paypalme/acbauer12/9.99).
Support is entirely optional and does not unlock features or priority assistance.
