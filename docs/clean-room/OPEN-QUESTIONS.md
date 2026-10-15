# Open questions

Spec ambiguities that a work package had to settle. Each entry gives the question, the decision taken
and the reason. Later work packages follow these unless the owner overrules them.

## W0-10

1. **Where does the bundle go?** cloud-036 says `<data_dir>/feedback/`, shell-033 says Downloads.
   **Decision:** `<data dir>/feedback/<YYYYMMDD-HHMMSS>.zip` (local time, `-2`, `-3` suffixes on a
   clash), where the data dir is the default one (`SCREENPIPE_DATA_DIR` or `~/.screenpipe`) that holds
   the logs, not a custom recording `dataDir`. Logs from the custom dir are still collected when it
   differs. **Reason:** the WP acceptance check names `<FP_DD>/feedback/`, and keeping bundles beside
   the logs needs no extra permission or folder picker.
2. **`read_log_tail` arguments and limits.** **Decision:** `read_log_tail(path, maxBytes)` as the
   shell-033 correction says; default 100 KB, clamped to 1 KB..8 MB; the file must resolve (symlinks
   followed first) inside the default or configured data dir and have `.log` in its name; a cut tail
   starts on a whole line and carries a one-line marker with the shown and total KB. **Reason:** a
   renderer-callable file reader must not become a general file read.
3. **Sensitive-key rule for settings.** **Decision:** the target's fragments plus `passphrase`,
   `cookie`, `privatekey`, `devicename` and any key ending in `key`; exceptions are the target's list
   plus `maxTokens`. The rule lives in `apps/screenpipe-app-tauri/lib/feedback/sensitive-keys.json`,
   read by both the TypeScript helper and the Rust writer, with shared test vectors. **Reason:**
   Freenpipe settings hold keys such as `deepgramApiKey`, `openaiKey` and `licenseKey` the target list
   misses, and `maxTokens` matches `token` while being harmless and useful for debugging.
4. **What does a masked value look like?** **Decision:** strings and numbers become `[redacted]`,
   non-empty arrays and objects become `[redacted: N items]` / `[redacted: N keys]`; booleans, nulls,
   empty strings and empty containers are kept. **Reason:** "is it set / is it on" stays answerable
   without exposing the value.
5. **How is free text scrubbed (`redact_pii_for_feedback`)?** **Decision:** the local deterministic
   detector from `screenpipe-redact` with labels secret, email, phone, url and id always on, plus the
   user's `piiRedactionLabels`; then user names in home-folder paths, IPv4 addresses other than
   127.x and 0.0.0.0, bearer tokens and `password=`-style values, and the signed-in user's name,
   e-mail, the computer's host name and the OS login name. Bare IPv4 addresses, which the shared
   detector labels `url`, are left to the IPv4 rule so loopback stays readable; URLs with
   credentials are still removed whole. Host and login names are scrubbed only when they have four
   or more characters and are not generic (`admin`, `user`, `owner`, `desktop`, ...). `settingsJson`
   is optional and bad JSON is ignored. No remote or model-based redaction. **Reason:** decloud (no
   Tinfoil enclave), and a bundle may end up in a public issue; generic names would blank out
   ordinary words in the logs.
6. **Does the app open the GitHub issue by itself?** **Decision:** no. The completion event carries
   `issueUrl` and the UI offers "open github issue". The body holds a scrubbed excerpt of the note
   (shortened so the URL stays under 7,000 characters), the version, OS, report source and the bundle
   file name, never the full local path. **Reason:** the WP says "offer"; opening a browser
   unasked is surprising, and the full path contains the user name.
7. **`feedback-upload-completed` payload.** **Decision:** `{ jobId, sent: false, path, fileName,
   sizeBytes, issueUrl, error }`; failures send the same event with `error` set and `path` null.
   **Reason:** superset of the spec's `{jobId, sent:false, path}` so the UI needs no second call.
8. **Native notification on completion?** **Decision:** none. The dialog shows the completion
   state, and a toast appears when the dialog was closed while the bundle was being written.
   **Reason:** the bundle is user-initiated and usually finishes in seconds.
9. **Retention.** **Decision:** keep the newest 10 bundles in `feedback/` and delete leftover
   `.zip.partial` files; only files with the bundle naming pattern are touched. **Reason:** a bundle
   can carry a 50 MB recording; unbounded growth in the data dir is a disk-pressure risk.
10. **Defaults for optional content.** **Decision:** chat history is off by default (base had it on);
    screenshot and recording (mp4/mov, 50 MB at most) only when the user adds them; up to three
    newest crash dumps (64 MB each at most) from `<data dir>/crash-dumps/` are always included. The
    README states that screenshots, recordings and dumps are not scrubbed. **Reason:** privacy by
    default for content the user may post publicly; crash dumps are what shell-033 asks for.
11. **More than one bundle at once?** **Decision:** one job at a time; a second
    `start_feedback_upload` while one runs returns an error. **Reason:** avoids two jobs pruning or
    naming bundles in the same second.
