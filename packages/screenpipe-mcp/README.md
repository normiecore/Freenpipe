# Freenpipe MCP Server

MCP server for Freenpipe: lets AI assistants search your screen recordings and audio transcriptions, and control your computer.

The package keeps its upstream name, `screenpipe-mcp`, and talks to the Freenpipe API on `localhost:3030`, so existing MCP client configurations keep working.

## Installation

### Option 1: NPX

The `screenpipe-mcp` package on npm is published by the upstream screenpipe project, not by Freenpipe. It queries the same local API, so you can use it with Freenpipe. To run the code in this repository instead, use Option 3. Edit your Claude Desktop config:

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%AppData%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "npx",
      "args": ["-y", "screenpipe-mcp"]
    }
  }
}
```

### Option 2: HTTP Server (Remote / Network Access)

The MCP server can run over HTTP using the [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#streamable-http), allowing remote MCP clients to connect over the network instead of stdio. Use it when your AI assistant (e.g., OpenClaw) runs on a different machine than Freenpipe.

```bash
# loopback only (default)
npx -y screenpipe-mcp --http --port 3031

# expose to your LAN with bearer auth
npx -y screenpipe-mcp --http --listen-on-lan --api-key $(openssl rand -hex 16)

# or from source — must build first so dist/ exists
bun install && bun run build
bun run start:http -- --port 3031
```

> Tip: `npx screenpipe-mcp-http` (without `--http`) does **not** work —
> npm resolves by package name, and there is no `screenpipe-mcp-http`
> package. The HTTP server ships as a transport inside the
> `screenpipe-mcp` package; use `--http` as shown above, or invoke the
> bin directly with `npx -p screenpipe-mcp screenpipe-mcp-http`.

The server exposes:
- **MCP endpoint**: `http://localhost:3031/mcp` — Streamable HTTP transport (POST for requests, GET for SSE stream)
- **Health check**: `http://localhost:3031/health` — always unauthenticated, for monitors

**Options:**
| Flag | Description | Default |
|------|-------------|---------|
| `--port` | Port for the MCP HTTP server | `3031` |
| `--screenpipe-port` | Port where the Freenpipe API is running | `3030` |
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

If your machines are on different networks, expose port 3031 via Tailscale, SSH tunnel, or similar. See the [OpenClaw integration guide](../../docs/mintlify/docs-mintlify-mig-tmp/openclaw.mdx) for detailed examples.

> **Note:** The HTTP server currently exposes `search_content` only. The stdio server has the full tool set (export-video, list-meetings, activity-summary, search-elements, frame-context).

### Option 3: From Source

Clone and build from source:

```bash
git clone https://github.com/normiecore/Freenpipe
cd Freenpipe/packages/screenpipe-mcp
bun install
bun run build
```

Then configure Claude Desktop:

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "node",
      "args": ["/absolute/path/to/screenpipe-mcp/dist/index.js"]
    }
  }
}
```

**Note:** Restart Claude Desktop after making changes.

## Testing

Test with MCP Inspector:

```bash
npx @modelcontextprotocol/inspector npx screenpipe-mcp
```

## Transport Modes

| Mode | Command | Use Case |
|------|---------|----------|
| **stdio** (default) | `npx screenpipe-mcp` | Claude Desktop, local MCP clients |
| **HTTP** | `npx screenpipe-mcp --http` | Remote clients, network access, OpenClaw on VPS |

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

- Freenpipe must be running on localhost:3030
- Node.js >= 18.0.0

## Notes

- All timestamps are handled in UTC
- Results are formatted for readability in Claude's interface
- macOS automation features require accessibility permissions

## Privacy Policy

The Freenpipe MCP server is a local-only bridge between Claude and your
local Freenpipe instance. It does not collect, transmit, or store any
data on its own.

### What this MCP server does
When Claude invokes a tool (`search-content`, `activity-summary`, etc.)
the MCP server forwards the request to `http://localhost:3030` (the
Freenpipe engine running on your machine) and returns the response.
That's the entire data path.

### Data collection
**None by this MCP server.** No analytics, no telemetry, no usage tracking.

### Data usage
Tool calls are passed straight through to your local Freenpipe engine
and the results stream back to Claude. The MCP server doesn't keep
anything.

### Data storage
Nothing is stored by the MCP server itself. Recordings, OCR text,
audio transcripts, and UI events are stored by Freenpipe in a
SQLite database under `~/.screenpipe/` on your device. Retention is
whatever you configure in Freenpipe, usually in the storage settings.

### Third-party sharing
None. The MCP server only talks to `localhost:3030`. It does not
contact Anthropic, the upstream vendor's servers, or any other external
service. If you configure a cloud AI provider or another remote service
in Freenpipe itself, that data flow is governed by the provider you chose,
not by this MCP server.

### Retention
The MCP server has no persistent state. The data Freenpipe captures is
retained according to your Freenpipe storage configuration and is
deletable at any time (`rm -rf ~/.screenpipe` removes everything).

### Source code
The Freenpipe MCP server is MIT-licensed and the entire source is
public at <https://github.com/normiecore/Freenpipe/tree/main/packages/screenpipe-mcp>.

### Contact
Questions or concerns: open an issue at
<https://github.com/normiecore/Freenpipe/issues>.
