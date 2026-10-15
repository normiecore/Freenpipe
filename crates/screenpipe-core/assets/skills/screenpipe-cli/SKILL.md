---
name: screenpipe-cli
description: Manage screenpipe pipes (scheduled AI automations) and connections (Telegram, Slack, Discord, etc.) via the CLI. Use when the user asks to create, list, enable, disable, run, or debug pipes, or manage service connections from the command line.
---

# Screenpipe CLI

Run commands as `screenpipe <command>`. The CLI is installed with the desktop app, so nothing has to be downloaded first:

- In agent sessions started by the app, `screenpipe` already points at that binary, and its absolute path is in `$SCREENPIPE_CLI`. Use `"$SCREENPIPE_CLI" <command>` if a tool runs without a shell.
- In your own terminal, call the `screenpipe` executable in the app's install folder (on Windows, next to the app's `.exe`).
- Do not fetch the CLI from npm (`npx`, `bun x`); those packages are not this app.

## Shell

- Every platform uses `bash`. On Windows the app provides a portable Git bash for agents, so nothing needs installing.

> **Note:** tool output is cut off at roughly 50 KB. Listings such as `pipe list` and `connection list` put enabled or connected entries first; to find one row, filter the output instead of reading all of it, e.g. `screenpipe connection list | grep -E 'browser|connected'`.

---

## Pipe Management

Pipes are markdown-based AI automations that run on schedule. Each pipe lives at `~/.screenpipe/pipes/<name>/pipe.md`.

### Commands

```bash
screenpipe pipe list                   # List all pipes (compact table)
screenpipe pipe enable <name>          # Enable a pipe
screenpipe pipe disable <name>         # Disable a pipe
screenpipe pipe run <name>             # Run once immediately (for testing)
screenpipe pipe logs <name>            # View execution logs
screenpipe pipe install <url-or-path>  # Install from GitHub or local path
screenpipe pipe delete <name>          # Delete a pipe
screenpipe pipe models list            # View AI model presets
```

### Creating a Pipe

Create `~/.screenpipe/pipes/<name>/pipe.md` with YAML frontmatter + prompt:

```markdown
---
schedule: every 30m
enabled: true
preset: ["Primary", "Fallback"]
---

Your prompt instructions here. The AI agent executes this on schedule.

## What to do

1. Query screenpipe search API for recent activity
2. Process results
3. Output summary / send notification
```

**Schedule syntax**:
- Recurring: `every 30m`, `every 1h`, `every day at 9am`, `every monday at 9am`, or cron `*/30 * * * *`, `0 9 * * *`
- One-off (fires once, then auto-disables): `at <RFC3339 timestamp>` — e.g. `at 2026-04-29T17:00:00-07:00`
- Manual only: `manual` (run via `pipe run` or API trigger)

**One-off scheduled tasks** (use this when the user says "in 2 days", "tomorrow at 5pm", "next Monday", "remind me to check X later", or any other future-time deferred action):

```yaml
---
schedule: at 2026-04-29T17:00:00-07:00
enabled: true
preset: auto
---

Check Gmail for a reply from Mark about the HIPAA evidence pack.
If found, summarize and send a notification. If not, note it.
```

Resolve "in 2 days" / "tomorrow 5pm" / "next Monday" against the user's local timezone (which is in the context header), format as RFC3339 with offset, and put it in the `at <iso>` schedule.

When fired, the pipe auto-disables itself — `enabled: false` is set in the local-overrides file. The pipe.md stays on disk as history. Users see upcoming one-offs in the chat sidebar's "upcoming" section with a countdown ("in 2d 4h"). To cancel before fire time: `pipe disable <name>`. To re-run after firing: `pipe enable <name>` then `pipe run <name>` (or set a new `at <iso>`).

**Config fields**: `schedule`, `enabled` (bool), `preset` (string or array — e.g. `"Oai"` or `["Primary", "Fallback"]`), `history` (bool — include previous output as context)

Screenpipe prepends a context header with time range, timezone, OS, and API URL before each execution. No template variables needed.

After creating:
```bash
screenpipe pipe install ~/.screenpipe/pipes/my-pipe
screenpipe pipe enable my-pipe
screenpipe pipe run my-pipe            # test immediately
```

### Editing Config

Edit frontmatter in `~/.screenpipe/pipes/<name>/pipe.md` directly, or use the API:

```bash
curl -X POST http://localhost:3030/pipes/<name>/config \
  -H "Content-Type: application/json" \
  -d '{"config": {"schedule": "every 1h", "enabled": true}}'
```

### Rules

1. Use `pipe list` (not `--json`) — table output is compact
2. Never dump full pipe JSON — can be 15MB+
3. Check logs first when debugging: `pipe logs <name>`
4. Use `pipe run <name>` to test before waiting for schedule

---

## Connection Management

Manage integrations (Telegram, Slack, Discord, Email, Todoist, Teams) from the CLI.

### Commands

```bash
screenpipe connection list             # List all connections + status
screenpipe connection list --json      # JSON output
screenpipe connection get <id>         # Show saved credentials
screenpipe connection get <id> --json  # JSON output
screenpipe connection set <id> key=val # Save credentials
screenpipe connection test <id>        # Test a connection
screenpipe connection remove <id>      # Remove credentials
```

### Examples

```bash
# Set up Telegram
screenpipe connection set telegram bot_token=123456:ABC-DEF chat_id=5776185278

# Set up Slack webhook
screenpipe connection set slack webhook_url=https://hooks.slack.com/services/...

# Verify it works
screenpipe connection test telegram

# Check what's connected
screenpipe connection list
```

Connection IDs: `telegram`, `slack`, `discord`, `email`, `todoist`, `teams`, `google-calendar`, `apple-intelligence`, `openclaw`

Credentials are stored locally at `~/.screenpipe/connections.json`.

**Per-integration details**: don't guess API shapes from this skill. Run `connection list` or `connection get <id>` — each entry includes a self-describing `description` with credential fields, endpoints, and example bodies. Only fetch the integration you need.

## Sharing pipes

There is no hosted pipe store to upload to, so `screenpipe pipe publish <pipe-name>` only prints how to share the pipe instead.

A pipe travels as one file: its `~/.screenpipe/pipes/<pipe-name>/pipe.md`, saved as `<pipe-name>.md` (the desktop app's "export pipe file" menu item does this and masks values that look like API keys or tokens). Whoever receives it runs:

```bash
screenpipe pipe install <pipe-name>.md   # the file name becomes the pipe name
```

If `SCREENPIPE_API_BASE_URL` names a pipe registry (a registry server, or a static `.json` index), these read from it:

```bash
screenpipe pipe search <query>
screenpipe pipe info <slug>
screenpipe pipe install <slug>
```

Without a registry they explain that none is configured; local and URL installs keep working.
