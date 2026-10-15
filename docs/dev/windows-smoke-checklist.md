# Windows smoke checklist

Cloud sessions cannot run Windows. The CI `windows` job compiles and unit-tests every PR, but
runtime and hardware behaviour (screen capture, audio devices, UIA, DirectML, Windows OCR, tray,
windows, installer, deep links, OAuth in a browser) needs a real Windows desktop. Those checks
collect here, and the owner runs them when the PC is on.

- **Workers add entries** (never tick them) in the same PR as the code, under their WP's heading in
  plan order. See [cloud-runbook.md](cloud-runbook.md), section 1.3.
- **The owner runs them** and records the result in place.
- **The merger** counts the open entries of each wave in its integration record (runbook 4.7).

## How to run a smoke session

1. Update a checkout of `main` (or the commit named in the entry) and build it in the isolated dev
   setup from [windows-build.md](windows-build.md):

   ```bash
   source scripts/dev/msvc-env.sh          # MSVC, LLVM, CMake and OpenBLAS environment for Git Bash
   source scripts/dev/dev-env.sh smoke     # FP_DD, port 3035, focus port 11436, SCREENPIPE_API_KEY
   cargo build -p screenpipe-engine --profile release-dev
   ```

2. Engine checks: `scripts/dev/engine-smoke.sh` starts the engine on `$FP_PORT` with a throwaway data
   dir. For longer runs, start it yourself:
   `target/release-dev/screenpipe.exe record --port $FP_PORT --data-dir "$FP_DD" > "$FP_DD/engine.log" 2>&1 &`.
   Call the API with
   `curl -s -H "Authorization: Bearer $SCREENPIPE_API_KEY" http://127.0.0.1:$FP_PORT/<path>`.
3. App checks (UI smoke): quit the installed screenpipe first, then run `scripts/dev/app-dev.sh`
   (guards, then `bun run tauri dev`). Read its caveats in windows-build.md: the webview still uses
   `~/.screenpipe` until the owning WPs fix that.
4. Never point a dev build at `~/.screenpipe` or ports 3030/11435 unless an entry says so explicitly.

## Recording results

Change `- [ ]` to `- [x]` for a pass or `- [!]` for a failure, and fill in the `Result:` line with the
date, the commit you tested and a note. Commit the file to `main` yourself (the GitHub web editor is
fine); agents never tick these boxes. For a failure, describe it under **Failures to fix** at the
end of this file, then start a cloud session to fix it on a `fix/smoke-<entry-id>` branch, or leave
it for the WP that owns the area.

## Entry format

```markdown
### W3-12 Windows OCR worker

- [ ] **SMK-W3-12-1** (UI smoke | Manual | Runtime | Hardware) <one-line check>
  - Setup: <build, settings, devices, data needed>
  - Steps: 1. ... 2. ...
  - Expected: <observable result>
  - Result:
```

Ids are `SMK-<WP-ID>-<n>`. Keep each entry self-contained: someone who did not write the code must
be able to run it in about ten minutes.

## Wave 0: Foundation

Wave 0 ran on the PC before the cloud phase. Tick anything you already checked there.

### W0-01 Build bootstrap, ee/ stubs, endpoint switchboard and dev isolation

- [ ] **SMK-W0-01-1** (Runtime) Engine harness smoke on the dev port leaves `~/.screenpipe` alone
  - Setup: release-dev engine build; `source scripts/dev/dev-env.sh`.
  - Steps: 1. Note the mtime of `~/.screenpipe/auth.json` (if it exists). 2. Run `scripts/dev/engine-smoke.sh`.
  - Expected: `/health` answers JSON on port 3035; the script passes; `~/.screenpipe` and its `auth.json` are unchanged.
  - Result:
- [ ] **SMK-W0-01-2** (Runtime) No blind kill of an installed screenpipe
  - Setup: installed screenpipe running (ports 3030 and 11435 in use).
  - Steps: 1. `FP_ALLOW_SCREENPIPE_RUNNING=1 scripts/dev/app-dev.sh`. 2. Let the dev app start, then quit it.
  - Expected: the Freenpipe log reports the port holders and does not kill them; the installed screenpipe engine and app are still running afterwards.
  - Result:

### W0-02 Remove telemetry and crash reporting (PostHog, Sentry)

- [ ] **SMK-W0-02-1** (Runtime) No telemetry traffic during a recording run
  - Setup: release-dev engine, dev env.
  - Steps: 1. Run the engine with `record` on `$FP_PORT` for 2 minutes. 2. `grep -iE 'posthog|sentry' "$FP_DD/engine.log"`. 3. Optional: watch connections with Resource Monitor or `netstat -b`.
  - Expected: no matching log lines; no connections to PostHog or Sentry hosts.
  - Result:

### W0-03 Updater on GitHub releases, release pipeline de-vendoring, restart_for_update

- [ ] **SMK-W0-03-1** (UI smoke) The updater is off by default, and the manual check opens the releases page
  - Setup: `scripts/dev/app-dev.sh` with a fresh `FP_DD`.
  - Steps: 1. Start the app and wait one minute. 2. Use the manual update-check action (Settings or tray).
  - Expected: no automatic update request at start; the manual action opens https://github.com/normiecore/Freenpipe/releases in the browser (DECISIONS.md 3).
  - Result:

### W0-04 Remove entitlement, login and plan gates

- [ ] **SMK-W0-04-1** (UI smoke) A fresh install records without signing in
  - Setup: installed screenpipe quit; fresh `FP_DD` (no `store.bin`).
  - Steps: 1. `scripts/dev/app-dev.sh`. 2. Complete onboarding if shown.
  - Expected: recording starts automatically; no sign-in prompt, plan gate or entitlement screen appears.
  - Result:

### W0-05 Remove commerce, team and vendor UI surfaces

- [ ] **SMK-W0-05-1** (UI smoke) Settings has no account, usage, team or referral sections
  - Setup: app dev run.
  - Steps: open every Settings section.
  - Expected: no Account, Usage, Team or Referral section, and no upgrade or checkout button anywhere.
  - Result:
- [ ] **SMK-W0-05-2** (UI smoke) The tray menu has no plan or upgrade row
  - Setup: app dev run.
  - Steps: open the tray menu.
  - Expected: no plan, upgrade or subscription entry.
  - Result:

### W0-06 AI gateway, transcription, PII backend and media analysis to bring-your-own

- [ ] **SMK-W0-06-1** (Runtime) Chat completions without a gateway return 503
  - Setup: engine on `$FP_PORT`, no `aiGatewayUrl` configured.
  - Steps: `curl -s -X POST -H "Authorization: Bearer $SCREENPIPE_API_KEY" -H 'Content-Type: application/json' -d '{"model":"x","messages":[]}' http://127.0.0.1:$FP_PORT/v1/chat/completions -w ' %{http_code}'`
  - Expected: HTTP 503 with error `cloud_token_missing`.
  - Result:
- [ ] **SMK-W0-06-2** (Runtime) A configured gateway receives the request
  - Setup: a local mock OpenAI-compatible server (any small HTTP server that logs requests); `aiGatewayUrl` pointing at it.
  - Steps: repeat the request from SMK-W0-06-1.
  - Expected: the mock logs the forwarded request; the engine returns the mock's answer.
  - Result:
- [ ] **SMK-W0-06-3** (UI smoke) A fresh install starts on a local Ollama preset with nothing remote configured
  - Setup: installed screenpipe quit; fresh `FP_DD` (no `store.bin`); `scripts/dev/app-dev.sh`.
  - Steps: 1. Finish onboarding. 2. Open Settings > AI. 3. Open Settings > Privacy and Settings > Recording. 4. Optional: `netstat -b` while doing this.
  - Expected: one preset named `default` on Ollama (`http://localhost:11434/v1`) with no model picked; the AI gateway card is not selected and says to set a gateway URL; transcription is Whisper Turbo (fast) or Parakeet; the AI gateway engine is not offered; "AI audio & video analysis" is off and disabled; no connection to any screenpipe or Tinfoil host.
  - Result:
- [ ] **SMK-W0-06-4** (UI smoke) A user gateway serves chat, models and the agent
  - Setup: a local OpenAI-compatible server (for example LiteLLM in front of Ollama) at `http://127.0.0.1:4000/v1`; app dev run.
  - Steps: 1. Settings > AI > new preset > **AI gateway**; enter the URL and key; leave the fields. 2. Pick a model from the list. 3. Make it the default and send a chat message. 4. Shift+drag a region in the timeline (region OCR) with a vision model on the gateway.
  - Expected: the model list comes from the gateway; the chat answers; region OCR copies text; `%USERPROFILE%\.pi\agent\models.json` has a `screenpipe` provider whose `baseUrl` is your URL.
  - Result:
- [ ] **SMK-W0-06-5** (UI smoke) A screenpipe store on hosted settings is migrated once
  - Setup: a synthetic `store.bin` (never a real one) whose settings hold presets with `"provider": "screenpipe-cloud"`, `"audioTranscriptionEngine": "screenpipe-cloud"`, `"piiBackend": "tinfoil"`, `"cloudMediaAnalysisEnabled": true`; no gateway configured.
  - Steps: 1. Start the app on it. 2. Open Settings > AI, Recording and Privacy.
  - Expected: the hosted presets became Ollama presets (same names, no model); the transcription alert says the AI gateway is not active and Whisper is in use, and a notification "AI gateway transcription unavailable" appears; the PII backend shows Local and no enclave option; media analysis is off.
  - Result:
- [ ] **SMK-W0-06-6** (Runtime) Live notes fall back to the local engine without a gateway relay
  - Setup: engine/app with `meetingLiveTranscriptionProvider` set to `screenpipe-cloud`, no gateway; a microphone.
  - Steps: start a meeting from the tray and talk for a minute.
  - Expected: live captions arrive from the selected local engine; the log says live notes use the selected transcription engine; no websocket to any remote host.
  - Result:

### W0-07 OAuth broker replaced by BYO client credentials

- [ ] **SMK-W0-07-1** (Manual) Google Calendar connects with your own OAuth client
  - Setup: a Google Cloud OAuth client (desktop type) with the loopback redirect; app dev run.
  - Steps: 1. Enter the client id and secret in the connection settings. 2. Connect Google Calendar and finish the browser consent.
  - Expected: the loopback callback completes; tokens are stored in the secret store (not in plain files); calendar events load.
  - Result:
- [ ] **SMK-W0-07-2** (UI smoke) Without an OAuth app the connect button stays disabled and explains why
  - Setup: app dev run with a fresh `FP_DD`; nothing entered under any connection's OAuth app.
  - Steps: 1. Settings > Connections > Notion (any generic OAuth tile). 2. Open Zoom. 3. In Zoom, click "set up OAuth app", enter any client id and no redirect URI, save. 4. Open Google Calendar, then Gmail. 5. In Google Calendar click "set up OAuth app" and read the redirect URI to register. 6. Pipe onboarding or a pipe's post-install connections dialog: click connect on an OAuth tile (Notion).
  - Expected: 1. A "Connecting needs your own Notion OAuth app" line with "set up OAuth app"; "connect with Notion" is disabled. 2. Same for Zoom. 3. Saving is refused with a message asking for an HTTPS relay URL; nothing is stored. 4. The same OAuth app line sits above each card, and the card's "Connect ..." button is disabled. 5. `http://localhost:3035/connections/oauth/callback` (the dev engine's port, not 3030). 6. The tile shows the "no OAuth app is configured ... Settings > Connections" message instead of silently going back to idle.
  - Result:
- [ ] **SMK-W0-07-3** (Manual) Token refresh goes straight to the provider with your client
  - Setup: Google Calendar connected as in SMK-W0-07-1; engine log at `$FP_DD/engine.log`.
  - Steps: 1. Wait for the access token to expire (about an hour) or leave the app running past the next refresh-scheduler tick after expiry. 2. Load upcoming events in the Google Calendar card. 3. `grep -iE 'oauth refresh' "$FP_DD/engine.log"`.
  - Expected: events load without reconnecting; no refresh failure is logged; the only token host contacted is `oauth2.googleapis.com` (check with Resource Monitor or a proxy if in doubt).
  - Result:
- [ ] **SMK-W0-07-4** (Manual) MCP OAuth finishes on the local callback
  - Setup: app dev run on port 3035; a remote MCP server that signs in with OAuth (Krisp, Plaud or any server added through Custom MCP).
  - Steps: 1. Start its sign-in from Settings > Connections. 2. Look at the `redirect_uri` in the provider's browser URL. 3. Finish the consent.
  - Expected: the redirect is `http://localhost:3035/mcp-servers/<id>/oauth/callback`; the tab shows the connected page and the server lists its tools.
  - Result:
- [ ] **SMK-W0-07-5** (Manual) Cal.com connects and refreshes with your own OAuth client
  - Setup: a Cal.com OAuth client you registered, with redirect `http://localhost:3035/connections/oauth/callback`; app dev run; engine log at `$FP_DD/engine.log`.
  - Steps: 1. Settings > Connections > Cal.com > "set up OAuth app": enter the client id and secret. 2. Connect and finish the browser consent. 3. After the access token expires (or the next refresh-scheduler tick after expiry), use the connection again. 4. `grep -iE 'oauth refresh|token exchange' "$FP_DD/engine.log"`.
  - Expected: the code exchange at `https://app.cal.com/api/auth/oauth/token` succeeds and Cal.com shows as connected; the later refresh succeeds without reconnecting; no exchange or refresh failure is logged. If either fails, record the HTTP status and error body so the token endpoint in `crates/screenpipe-connect/src/connections/calcom.rs` can be corrected.
  - Result:

### W0-08 Cloud sync, archive and pipe store off by default

- [ ] **SMK-W0-08-1** (Runtime) The pipe store is empty without a registry, and lists and installs from a local static index
  - Setup: engine on `$FP_PORT` with `SCREENPIPE_API_BASE_URL` unset. A static registry in a temp dir:
    `mkdir -p "$FP_DD/registry/pipes/hello"`,
    `printf -- '---\nschedule: manual\nenabled: false\n---\n\nSay hello.\n' > "$FP_DD/registry/pipes/hello/pipe.md"`,
    `echo '{"data":[{"slug":"hello","title":"Hello","version":1,"source_path":"pipes/hello/pipe.md"}]}' > "$FP_DD/registry/index.json"`,
    then `python -m http.server 8765 --directory "$FP_DD/registry"` in a second shell.
  - Steps: 1. `GET /pipes/store` and `GET /pipes/store/hello`. 2. Restart the engine with `SCREENPIPE_API_BASE_URL=http://127.0.0.1:8765/index.json`, then `GET /pipes/store` and `POST /pipes/store/install` with body `{"slug":"hello"}` (`-H "Content-Type: application/json" -d ...`). 3. `POST /pipes/store/publish` with `{}`.
  - Expected: 1. HTTP 200 `{"data":[],"registry_configured":false}`; the detail call answers 503 with `"error_code":"registry_not_configured"`. 2. The list holds `hello`; install answers `{"success":true,"name":"hello",...}` and `$FP_DD/pipes/hello/pipe.md` exists with `source_slug: hello`. 3. HTTP 501 with `"error_code":"registry_read_only"`.
  - Result:
- [ ] **SMK-W0-08-2** (UI smoke) Storage settings offer no cloud archive or cloud sync
  - Setup: app dev run against the isolated data dir.
  - Steps: 1. Settings > Storage. 2. Search settings for "archive" and for "sync".
  - Expected: only the Local tab is shown (no Archive or Sync tab), the section subtitle reads "Local disk", and the settings search finds no Archive or Sync entry.
  - Result:
- [ ] **SMK-W0-08-3** (Runtime) Cloud sync and archive never start, even for a store that had them enabled
  - Setup: app dev run against the isolated data dir; engine reachable on `$FP_PORT`.
  - Steps: 1. Start the app and read its log for the first minute. 2. `POST /archive/init` with `{"token":"x"}` and `POST /sync/init` with `{"token":"x","password":"y"}` (`-H "Content-Type: application/json"`).
  - Expected: 1. The log shows "cloud sync: no sync server configured, skipping auto-start" and the same for cloud archive; these checks run before any stored setting is read, so an old `store.bin` with sync or archive enabled behaves the same. 2. Both answer HTTP 501 with an error saying cloud sync is not configured.
  - Result:
- [ ] **SMK-W0-08-4** (UI smoke) Pipe store tab explains the missing registry and has no publish or review controls
  - Setup: app dev run with `SCREENPIPE_API_BASE_URL` unset; then again with the static index from SMK-W0-08-1.
  - Steps: 1. Pipes > Discover. 2. Restart with the registry set, open Discover, open the `hello` pipe, install it.
  - Expected: 1. A "no pipe registry configured" card, no PUBLISH button. 2. `hello` is listed; its detail page shows FORK and GET only (no EDIT, PUBLISH UPDATE or UNPUBLISH); GET installs it and switches to My Pipes.
  - Result:
- [ ] **SMK-W0-08-5** (UI smoke) "export pipe file" replaces share and publish
  - Setup: app dev run with at least one pipe in My Pipes (the `hello` pipe from SMK-W0-08-4 works).
  - Steps: 1. Open the pipe's overflow menu. 2. Choose "export pipe file" and save to the desktop. 3. Delete the pipe, then run `screenpipe pipe install <saved file>` (or use the install dialog).
  - Expected: 1. The menu has "export pipe file" and no "copy share link" or "publish to store"; there is no upload icon on the pipe row. 2. The save dialog proposes `hello.md`; the file holds the pipe.md text and a toast confirms the export. 3. The pipe comes back under the same name.
  - Result:

### W0-10 Logs and feedback upload replaced by a local diagnostics bundle

- [ ] **SMK-W0-10-1** (UI smoke) "Share logs" writes a local diagnostics bundle
  - Setup: app dev run.
  - Steps: Help > share logs.
  - Expected: `<FP_DD>/feedback/<timestamp>.zip` appears, the UI shows the completion state, and nothing is uploaded.
  - Result:

### W0-12 Rebrand user-visible identity to Freenpipe

- [ ] **SMK-W0-12-1** (UI smoke) The installer and the running app say Freenpipe
  - Setup: a Windows installer built from `main` (the NSIS bundle; see windows-build.md), installed screenpipe left installed.
  - Steps: 1. Run the installer. 2. Start Freenpipe and check the tray tooltip and every window title.
  - Expected: the installer is named Freenpipe; tray tooltip and window titles say Freenpipe; the installed screenpipe still starts and works.
  - Result:
- [ ] **SMK-W0-12-2** (Manual) Deep-link registration leaves screenpipe's scheme alone
  - Setup: `reg export HKCU\Software\Classes\screenpipe before.reg` before installing.
  - Steps: 1. Install Freenpipe. 2. `reg query HKCU\Software\Classes\freenpipe`. 3. Export the screenpipe key again and compare.
  - Expected: the `freenpipe` key exists; the `screenpipe` key is byte-identical to before.
  - Result:

## Wave 1: Cross-cutting foundations

### W1-01 Schema v2.7.104: 28 migrations and ocr_text retirement

- [ ] **SMK-W1-01-1** (Runtime) Real capture writes text into frames, not ocr_text
  - Setup: engine on `$FP_PORT` with a fresh `FP_DD`, a normal desktop session.
  - Steps: 1. Record for 2 minutes while using a few apps. 2. `GET /search?content_type=ocr&limit=5`. 3. `python -I -c "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(c.execute('select count(*) from frames where text_json is not null').fetchone(), c.execute(\"select name from sqlite_master where name like 'ocr_text%'\").fetchall())" "$FP_DD/db.sqlite"`.
  - Expected: search returns items whose `ocr_engine` is empty; frames carry `text_json`; no `ocr_text` table exists.
  - Result:

### W1-03 Integration seams, stub modules and shared constants

- [ ] **SMK-W1-03-1** (Runtime) The OpenAPI route count did not change
  - Setup: engine builds from the commit before W1-03 and from main.
  - Steps: for each build, `GET /openapi.json` and count `paths`.
  - Expected: the same count for both builds.
  - Result:

## Failures to fix

None yet.
