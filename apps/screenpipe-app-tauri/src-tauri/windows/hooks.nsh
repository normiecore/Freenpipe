!macro NSIS_HOOK_PREINSTALL
  ; Stop Freenpipe processes that run from this install directory (the app,
  ; the bundled screenpipe CLI/engine and the Bun sidecar) so their files can
  ; be replaced. Processes are matched by path, not by image name: an
  ; installed screenpipe uses the same executable names from its own folder
  ; and must keep running. Use CIM ExecutablePath instead of Get-Process.Path:
  ; reading process module paths can throw "Access to the path is denied".
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "$$root = [System.IO.Path]::GetFullPath(''$INSTDIR'').TrimEnd(''\'') + ''\''; Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -and $$_.ExecutablePath.StartsWith($$root, [System.StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { $$pidToStop = $$_.ProcessId; Stop-Process -Id $$pidToStop -Force -ErrorAction SilentlyContinue; Wait-Process -Id $$pidToStop -Timeout 5 -ErrorAction SilentlyContinue }"'
  ; Wait a moment for processes to fully terminate and release file handles.
  Sleep 1000
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Clean up the runtime-downloaded PortableGit (bash for AI chat). The folder
  ; keeps screenpipe's name on purpose (shared data layout, decision 1); the
  ; chat re-downloads it on demand.
  RMDir /r "$LOCALAPPDATA\screenpipe\git-portable"
  ; Remove the parent dir only if empty (keeps any other data in it).
  RMDir "$LOCALAPPDATA\screenpipe"
!macroend
