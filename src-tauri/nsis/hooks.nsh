; hooks.nsh — custom NSIS macros for the oSystems Sync installer (T-6.4, RNF-016).
;
; Referenced from `tauri.conf.json` → `bundle.windows.nsis.installerHooks`.
; Supported macro names are fixed by Tauri's bundler; see:
; https://v2.tauri.app/distribute/windows-installer/#installer-hooks
;
; Only `NSIS_HOOK_PREUNINSTALL` is defined here — see SPEC.md §12 (decision:
; "NSIS currentUser install mode + uninstall hook").

!macro NSIS_HOOK_PREUNINSTALL
  ; 1. Stop a running instance before touching files/registry, otherwise the
  ;    uninstaller cannot remove the locked .exe and DLLs.
  ;    Binary name comes from the Cargo package name (`osystems-sync`,
  ;    `src-tauri/Cargo.toml`), which Cargo uses verbatim as the implicit bin
  ;    target — the shipped executable is `osystems-sync.exe`.
  nsExec::Exec 'taskkill /IM osystems-sync.exe /F'
  Pop $0

  ; 2. Remove the autostart registry value created by `tauri-plugin-autostart`
  ;    (RF-092, `src-tauri/src/lib.rs`). The plugin defaults the value name to
  ;    `app.package_info().name`, which resolves to `tauri.conf.json`'s
  ;    `productName` ("oSystems Sync") — confirmed by reading
  ;    `tauri-plugin-autostart-2.5.1` + its `auto-launch-0.5.0` dependency
  ;    (`src/windows.rs`), which writes/deletes exactly this value name under
  ;    `HKCU\...\Run` on `enable()`/`disable()`.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "oSystems Sync"

  ; 3. `auto-launch` also writes a matching value to the Task Manager
  ;    "startup approved" override key when autostart is enabled, so it is not
  ;    re-flagged as disabled in Task Manager. Clean it up too — safe no-op if
  ;    autostart was never enabled (key/value may not exist).
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "oSystems Sync"
!macroend
