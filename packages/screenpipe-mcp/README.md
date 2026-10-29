# Handvel MCP Server

MCP server for Handvel: lets AI assistants search your screen recordings and audio transcriptions, and control your computer.

The package keeps its upstream name, `screenpipe-mcp`, and talks to the Handvel API on `localhost:3030`, so existing MCP client configurations keep working.

## Installation

Handvel does not publish this server to a package registry, and the
`screenpipe-mcp` package on npm belongs to the upstream project. Use the copy that ships
with the desktop app, or build it from this repository.

### Option 1: The copy bundled with the Handvel app (recommended)

The app installs the server as one file, `mcp/screenpipe-mcp.js`, in its
install folder and runs it with the `bun` binary it also ships. Settings >
Connections writes the config for Claude Desktop, Cursor, Codex and other
clients for you, with absolute paths and your local API key. To do it by
hand, edit your Claude Desktop config:

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%AppData%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "C:\\Users\\you\\AppData\\Local\\Handvel\\bun.exe",
      "args": ["C:\\Users\\you\\AppData\\Local\\Handvel\\mcp\\screenpipe-mcp.js"],
      "env": { "SCREENPIPE_LOCAL_API_KEY": "<your local API key>" }
    }
  }
}
```

Replace both paths with your install folder. Add `"--port", "<port>"` to
`args` when the local API does not listen on 3030. The standalone CLI can
write the same config: `screenpipe mcp setup`.

### Option 2: Build from source

```bash
git clone https://github.com/normiecore/Handvel
cd Handvel/packages/screenpipe-mcp
bun install
bun run build
```

Then point your client at the build with an absolute path:

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "node",
      "args": ["/absolute/path/to/packages/screenpipe-mcp/dist/cli.js"]
    }
  }
}
```

The single-file build the app ships is made with
`bun build ./src/cli.ts --target=node --format=cjs --outfile=screenpipe-mcp.js`
(see `apps/screenpipe-app-tauri/scripts/stage_bundled_runtime.js`).

**Note:** Restart Claude Desktop after making changes.

### HTTP transport (remote / network access)

The server can also speak the [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#streamable-http),
so an MCP client on another machine can connect over the network instead of
stdio. Start it with `--http`, using either entry point from above:

```bash
# loopback only (default)
node dist/cli.js --http --port 3031

# reachable from your LAN, bearer token required
node dist/cli.js --http --listen-on-lan --api-key $(openssl rand -hex 16)

# or, from this package directory
bun run start:http -- --port 3031
```

The server exposes:
- **MCP endpoint**: `http://localhost:3031/mcp` — Streamable HTTP transport (POST for requests, GET for SSE stream)
- **Health check**: `http://localhost:3031/health` — always unauthenticated, for monitors

**Options:**
| Flag | Description | Default |
|------|-------------|---------|
| `--port` | Port for the MCP HTTP server | `3031` |
| `--screenpipe-port` | Port where the Handvel API is running | `3030` |
| `--listen-on-lan` | Bind `0.0.0.0` so other devices on the LAN can connect. Requires `--api-key`. | off (binds `127.0.0.1`) |
| `--api-key <secret>` | Bearer token required for non-loopback requests (`Authorization: Bearer <secret>`). Loopback always allowed. | none |

**Connecting a remote MCP client:**

Point any MCP client that supports HTTP transport at the `/mcp` endpoint:

```json
{
  "mcpServers": {
    "screenpipe": {
      "url": "http://<your-ip>:3031/mcp",
      "headers": {
        "Authorization": "Bearer <your-secret>"
      }
    }
  }
}
```

If the machines are on different networks, reach port 3031 through
Tailscale, an SSH tunnel or similar. The
[OpenClaw integration guide](../../docs/mintlify/docs-mintlify-mig-tmp/openclaw.mdx)
has examples.

> **Note:** The HTTP server currently exposes `search_content` only. The stdio server has the full tool set (export-video, list-meetings, activity-summary, search-elements, frame-context).

## Testing

Run the MCP Inspector against a local build:

```bash
bun run build
npx @modelcontextprotocol/inspector node dist/cli.js
```

## Transport Modes

| Mode | Command | Use Case |
|------|---------|----------|
| **stdio** (default) | `<bun or node> screenpipe-mcp.js` | Claude Desktop, local MCP clients |
| **HTTP** | `<bun or node> screenpipe-mcp.js --http` | Remote clients, network access |

## Available Tools

### search-content
Search through recorded content with content type filtering:
- `all` — OCR + Audio + Accessibility (default)
- `ocr` — Screen text from screenshots
- `audio` — Audio transcriptions
- `input` — User actions (clicks, keystrokes, clipboard, app switches)
- `accessibility` — Accessibility tree text
- Time range, app/window, and speaker filtering
- Pagination support

### export-video
Export screen recordings as video files:
- Specify time range with start/end times
- Configurable FPS for output video

### activity-summary
Get a lightweight compressed activity overview for a time range:
- App usage with active minutes and frame counts
- Recent accessibility texts
- Audio speaker summary

### list-meetings
List detected meetings with duration, app, and attendees. Pass `q` to filter by substring (title, attendees, notes).

### search-elements
Search structured UI elements (accessibility tree nodes and OCR text blocks):
- Filter by source, role, app, time range
- Much lighter than search-content for targeted UI lookups

### frame-context
Get accessibility text, parsed tree nodes, and extracted URLs for a specific frame.

## Example Queries in Claude

- "Search for any mentions of 'rust' in my screen recordings"
- "Find audio transcriptions from the last hour"
- "Show me what was on my screen in VSCode yesterday"
- "Export a video of my screen from 2-3pm today"
- "Find what John said in our meeting about the database"
- "What did I type in Slack today?" (uses content_type=input)
- "What did I copy to clipboard recently?" (uses content_type=input)
- "Show me accessibility text from Chrome" (uses content_type=accessibility)

## Requirements

- Handvel must be running on localhost:3030 (or pass `--port`)
- bun, or Node.js >= 18.0.0 (the app ships bun)

## Notes

- All timestamps are handled in UTC
- Results are formatted for readability in Claude's interface
- macOS automation features require accessibility permissions

## Privacy Policy

The Handvel MCP server is a local-only bridge between Claude and your
local Handvel instance. It does not collect, transmit, or store any
data on its own.

### What this MCP server does
When Claude invokes a tool (`search-content`, `activity-summary`, etc.)
the MCP server forwards the request to `http://localhost:3030` (the
Handvel engine running on your machine) and returns the response.
That's the entire data path.

### Data collection
**None by this MCP server.** No analytics, no telemetry, no usage tracking.

### Data usage
Tool calls are passed straight through to your local Handvel engine
and the results stream back to Claude. The MCP server doesn't keep
anything.

### Data storage
Nothing is stored by the MCP server itself. Recordings, OCR text,
audio transcripts, and UI events are stored by Handvel in a
SQLite database under `~/.screenpipe/` on your device. Retention is
whatever you configure in Handvel, usually in the storage settings.

### Third-party sharing
None: the server only talks to `localhost:3030` and does not contact
Anthropic, the upstream vendor's servers, or any other external service.
If you configure a cloud AI provider or another remote service in
Handvel itself, that data flow is governed by the provider you chose,
not by this MCP server.

### Retention
The MCP server has no persistent state. The data Handvel captures is
retained according to your Handvel storage configuration and is
deletable at any time (`rm -rf ~/.screenpipe` removes everything).

### Source code
The Handvel MCP server is MIT-licensed and the entire source is
public at <https://github.com/normiecore/Handvel/tree/main/packages/screenpipe-mcp>.

### Contact
Questions or concerns: open an issue at
<https://github.com/normiecore/Handvel/issues>.
