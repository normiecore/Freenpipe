# Open questions

Spec ambiguities resolved while implementing a work package. Each entry gives the question, the
decision taken and the reason. The owner can overturn any of them; the code points to where.

## W0-09

1. **Where is the "Freenpipe-controlled mirror" for the pyannote models, and what are the asset names?**
   - Decision: release assets of `normiecore/Freenpipe` under the tag `pyannote-models-v1`, named
     `pyannote-segmentation-3.0.onnx` and `wespeaker-en-voxceleb-campp.onnx`
     (`DEFAULT_MODEL_BASE_URL` in `crates/screenpipe-audio/src/speaker/models.rs`, mirrored in
     `apps/screenpipe-app-tauri/scripts/stage_bundled_runtime.js`). Downloads are checked against the
     size and SHA-256 recorded in the repository's Git LFS pointers.
   - Reason: a release asset is immutable and needs no LFS bandwidth; GitHub renames asset files with
     special characters, so `CAM++` cannot be used as an asset name. The local file names stay as
     before, so existing caches keep working.
   - **Owner action:** create that release and upload the two files (the LFS objects of
     `crates/screenpipe-audio/models/pyannote/`, renamed to the asset names). Until it exists, the
     installer gets the models from an LFS checkout only, and a fresh install without bundled models
     cannot download them (the URL answers 404).

2. **How do users override the model sources?**
   - Decision: environment variables only, for now: `SCREENPIPE_PYANNOTE_BASE_URL` (mirror base URL),
     `SCREENPIPE_PYANNOTE_DIR` (directory with pre-installed pyannote files), `HF_ENDPOINT`
     (Hugging Face compatible host for the PII models) and one repository override per PII model
     family: `SCREENPIPE_PII_REDACTOR_REPO`, `SCREENPIPE_PII_TEXT_REDACTOR_REPO`,
     `SCREENPIPE_PII_IMAGE_REDACTOR_REPO` (`owner/name`).
   - Reason: the target has no evidence of such settings, and settings keys belong to W1-02. The
     names follow the `SCREENPIPE_*` convention; `HF_ENDPOINT` is the hub's own variable and is already
     listed in shell.md. SHA-256 pins stay in force for every source, so a mirror cannot change the
     weights.

3. **Do the PII weights keep downloading from the vendor's Hugging Face organisation by default?**
   - Decision: yes, as cloud.md cloud-024 says (opt-in only, when local AI PII removal is enabled;
     regex-only redaction needs no download). The licence of those weights is still unknown and stays
     an open question for the owner.

4. **How is the CLI bundled, given that the base ships no standalone CLI?**
   - Decision: the engine's `screenpipe` binary is staged by `scripts/stage_bundled_runtime.js`
     (`FREENPIPE_CLI_BINARY=<path>` or `FREENPIPE_BUILD_CLI=1`) into `src-tauri/bundled/cli/` and
     mapped as a resource into the install root next to the app, bun and the runtime DLLs. It is not an
     `externalBin`, so development builds without a staged CLI still build. Only the Windows bundle
     maps it (`tauri.windows.conf.json`); macOS and Linux keep working without it (agents use the
     REST API, MCP snippets show placeholders, models download on first use).
   - Reason: the CLI must sit beside the DLLs it loads; an `externalBin` would make every Windows dev
     build depend on a release build of the engine.

5. **Should agents fall back to a `screenpipe` found on `PATH`?**
   - Decision: no. Agents get `SCREENPIPE_CLI`, a launcher directory (`<data dir>/pi-agent/bin`)
     first on `PATH`, and a bash function in the `BASH_ENV` shim, all pointing at the bundled binary.
     Only the MCP server's API-key discovery tries `screenpipe` on `PATH`, as its last CLI option.
   - Reason: a `screenpipe` on `PATH` may belong to another install of a different product; agents
     acting on the user's data should use the binary that came with this app.

6. **How is the MCP server shipped?**
   - Decision: `packages/screenpipe-mcp` is bundled by `bun build --target=node --format=cjs` into one
     file, `mcp/screenpipe-mcp.js`, and every generated MCP config runs `<bundled bun> <that file>`
     by absolute path (`lib/mcp-config.ts`). The config also carries `SCREENPIPE_LOCAL_API_KEY` and a
     `--port` when the API does not use 3030.
   - Reason: a single CommonJS file needs no install step and runs under bun or node, and the server
     always matches the installed app version.

7. **What does the standalone CLI's update notice compare against?**
   - Decision: the latest GitHub release of `normiecore/Freenpipe` (`tag_name`), instead of the npm
     registry entry of the upstream `screenpipe` package.
   - Reason: the upstream package version says nothing about Freenpipe releases, and querying it
     resolves a vendor package (cloud-M02).

8. **Which SKILL.md do the agent cards serve?**
   - Decision: `scripts/gen-skill-content.js` now reads the product copies under
     `crates/screenpipe-core/assets/skills/`, and the Hermes card installs skills from that folder of
     this repository. `.claude/skills/` (the repository's developer mirror, still mentioning the npm
     CLI) is no longer read by the app.
   - Reason: the product copies are what the app installs into its own agent sessions, and the
     mirror is edited by other tooling.
