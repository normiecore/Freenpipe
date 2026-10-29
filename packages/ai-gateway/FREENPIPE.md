# packages/ai-gateway in Handvel

This directory holds screenpipe's hosted AI gateway (a Cloudflare worker) as it was in the MIT base
commit. Handvel does not build, test, ship or deploy it: no workspace, CI workflow or release job
references it, and nothing in the app or engine calls it.

Handvel talks to an AI gateway only when you configure one:

- In the app: Settings > AI, choose the **AI gateway** provider and enter its URL (the OpenAI-style
  base including its version path, for example `http://127.0.0.1:4000/v1`) and, if it needs one, a
  key. These are stored as the `aiGatewayUrl` and `aiGatewayApiKey` settings.
- Headless: set `SCREENPIPE_AI_GATEWAY_URL` and `SCREENPIPE_AI_GATEWAY_KEY` for the engine.

Any OpenAI-compatible proxy works for chat (for example LiteLLM, or a local server in front of
Ollama). Optional features call further paths under the same base: `models` (model list),
`listen` (batch transcription in Deepgram's format), `realtime` (live-notes relay, websocket) and
`web-search` (agent web search). Features whose path your gateway does not serve simply fail and
fall back to local behaviour.

The worker in this directory could serve those paths, but as committed it authenticates users and
checks plans against the upstream vendor's account service, so it is not usable as-is. Treat it as
reference material; a self-hosted fork would have to replace that account logic first.
