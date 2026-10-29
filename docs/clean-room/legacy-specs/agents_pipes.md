# Spec: Pipes, AI agents, chat, MCP and LLM providers (`agents_pipes`)

Clean-room delta specification, screenpipe base `892199f` (~v2.5.27, MIT) to installed screenpipe v2.7.104 (Windows).
Every behaviour below is paraphrased from static evidence. Interop identifiers (routes, JSON keys, SQL, env vars, file
names, event names, settings keys) are quoted exactly so Freenpipe can read data written by screenpipe. Nothing here is
copied code, prompt text or UI prose.

## 0. Evidence conventions

- `S:0xNNNNNN` is a hex file offset into `<re-workspace>/app_strings.tsv` (column 1). Line numbers given as
  `L123456` are 1-based lines in that file.
- `F:<path>` is a file under `<re-workspace>/frontend/` (minified Next.js chunks; positions are character
  offsets inside the chunk where useful).
- REA: the JS application analysis is parent evidence `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`.
  Literal-seed traces for `/agent/learning/chats` (`ev_6d828e0fe2536b0fd79f0d90e52929bdddba6859d1e1a79955d3c945fc34f395`),
  `/workflows/workspace` (`ev_a781cc861d4a5db721451592571eddb11e339028aec2c1ced0ecfae68c0f4838`) and route `/dev/workflows-ux`
  (`ev_495868eee9bfd5a2ef792d59c430b0ee5e5d4ccde7204b1912b35061dc642a7b`) returned no graph match (the graph is partial);
  the `/pipes` string trace completed but exceeded the transport budget and is retained as
  `ev_6b11a7659a08867ee09e3891935823490eb2259440bc414115282cd4091fd6e6`. Frontend facts below were therefore taken directly
  from the chunks with `tools/ap_ctx.py`.
- Helper scripts written for this dimension: `tools/ap_srcpaths.py`, `ap_relpaths.py`, `ap_routes.py`, `ap_jsblocks.py`,
  `ap_ctx.py`, `ap_frontmatter.py` (all in `<re-workspace>/tools/`).
- Confidence: **observed** = an identifier/shape is literally present; **inferred** = behaviour reconstructed from
  surrounding log strings, error messages or call sites.

Target source-module inventory relevant to this dimension (from embedded panic/tracing paths, `tools/ap_srcpaths.py`):

| Crate | Target modules (new vs base in bold) |
|---|---|
| screenpipe-core `agents/` | pi.rs, **acp/{mod,executor,runtime,extensions}.rs**, **chat_control.rs**, **chat_destination.rs**, **cli_runtime.rs**, **cloud.rs**, **cloud_context.rs**, **pi_compaction.rs**, **worktree.rs** (base: bash_env.rs, pi.rs, mod.rs) |
| screenpipe-core `pipes/` | mod.rs, favorites.rs, permissions.rs, preset_fallback.rs, sync.rs, **builtin_migrations.rs**, **connection_triggers.rs (+ connection_triggers/voice.rs)**, **mcp_access.rs**, **trajectory.rs** |
| screenpipe-core `workflows/` | **mod.rs, evidence.rs, model_choice.rs, pipeline.rs, timing.rs, workspace.rs**; plus **workflow_video.rs** |
| screenpipe-engine | pipes_api.rs, pipe_store.rs, routes/pipe_store.rs, mcp_servers_api.rs, routes/cloud_proxy.rs, **pipe_stream.rs, agent_skills.rs, agent_profile.rs, local_chat.rs, mcp_call.rs, cli/agent.rs, routes/{cloud_agents, web_search, artifacts, structured_outputs, ai_feedback, connect_broker, workflows, workflow_catalog, workflow_edits, workflow_pipeline, workflow_workspace, internal_telemetry}.rs** |
| app (Tauri, relative `src\`) | pi.rs, pi_command_queue.rs, agent_event_emitter.rs, skills.rs, suggestions.rs, store.rs, **chat_control.rs, coding_workspace.rs, provider_automations.rs, workflows_runtime.rs, workflows_media.rs, workflow_video(_cli).rs** |

## 1. Base inventory (Freenpipe @ 892199f)

| Area | Base implementation | Files |
|---|---|---|
| Pipe format | `~/.screenpipe/pipes/<name>/pipe.md`, YAML frontmatter `PipeConfig` {name, schedule, enabled, agent (default `pi`), model (default `auto`), provider, preset (string or list), connections, permissions (preset string or allow/deny/time/days), timeout, trigger {events, custom}, source_slug, installed_version, source_hash, subagent, privacy_filter} + flattened extras (e.g. `history`, `title`, `icon`). Tombstones `.tombstones.json`, per-device enable overrides `.local-overrides.json`, `run.pid`, `logs/`. | crates/screenpipe-core/src/pipes/mod.rs |
| Scheduler | Interval/`daily`/`manual`/cron/one-off `at <iso>` schedules; event triggers on `meeting_started`, `meeting_ended`, `workflow_event` (inner `event_type`), `pipe_completed:<name>` with circular-chain guard (60 s cooldown); one global execution semaphore; setup-mode gate for missing connections; PID-file guard; preset fallback circuit breaker (`ai_preset_fallback.json`). | pipes/mod.rs, pipes/preset_fallback.rs |
| Execution store | Tables `pipe_executions` (+`session_path`), `pipe_scheduler_state`; `PipeStore` trait. | screenpipe-db migrations `20260213000000_create_pipe_executions.sql`, `20260326000000_add_session_path_to_pipe_executions.sql` |
| Pipes HTTP API | `/pipes` GET (opt. `include_executions`), `/pipes/install`, `/pipes/favorites`, `/pipes/:id` GET/DELETE, `/:id/enable`, `/:id/favorite`, `/:id/run` (body `notification_context`), `/:id/logs`, `/:id/config`, `/:id/stop`, `/:id/executions?limit`, `/:id/history` DELETE, `/:id/session/:exec_id`; store `/pipes/store{,/publish,/install,/update,/check-updates,/auto-update,/:slug,/:slug/review}` proxying `https://screenpi.pe/api/pipes/store`. | crates/screenpipe-engine/src/pipes_api.rs, routes/pipe_store.rs, server.rs |
| Pipe permissions | Typed rules Api()/App()/Window()/Content(), presets reader/writer/admin/none, default reader allowlist, `.screenpipe-permissions.json` + `sp_pipe_` token, middleware + backpressure layer. | pipes/permissions.rs, engine pipe_permissions_middleware.rs |
| Agent runtime | `AgentExecutor` trait; single `PiExecutor` driving `@earendil-works/pi-coding-agent@0.75.4` via bun; extensions written per pipe: screenpipe-permissions, orphan-guard, context-pruning, mcp-bridge, sub-agent, web-search; skills screenpipe-api/cli/team; PortableGit bootstrap. | agents/{mod,pi,bash_env}.rs, assets/extensions/*.ts, assets/skills/* |
| Chat (app) | Pi RPC sessions pool (max 20, idle eviction), queue/steer, abort, model switch; chats persisted to `~/.screenpipe/chats/*.json`; app extensions mcp-bridge + web-search. | apps/.../src-tauri/src/pi.rs, pi_command_queue.rs, lib/chat-storage.ts |
| AI presets | settings `aiPresets[]` with providers `native-ollama`, `openai`, `openai-chatgpt`, `anthropic`, `custom`, `embedded`, `screenpipe-cloud`, `pi` (legacy). | lib/hooks/use-settings.tsx, enterprise-ai-preset-policy.ts |
| Skills | Import skills from device (`~/.claude/skills`), store in `<data>/skills/<name>/`, mirrored into `.pi/skills/`. | src-tauri/src/skills.rs |
| MCP | npm package `screenpipe-mcp` 0.18.10 (stdio + HTTP `/mcp`), 27 tools; BYO MCP servers store (`mcp_servers.json`, http+stdio, OAuth) under `/mcp-servers`. | packages/screenpipe-mcp, crates/screenpipe-connect/src/mcp_servers.rs, engine mcp_servers_api.rs |
| LLM proxy | `/v1/chat/completions` proxy to `https://api.screenpipe.com/v1` with the cloud JWT. | engine routes/cloud_proxy.rs |

## 2. Target inventory summary (v2.7.104)

The UI labels pipes as scheduled tasks (`S:0x6209e39`). The runtime keeps the base pipe.md model but adds
typed scheduling, run destinations, artifacts, event idempotency, connection-driven triggers, a large built-in task set
(workflow discovery/enrichment, skill learning, commitments, speaker reconciliation, accounting), and three new agent
back-ends: ACP coding agents, cloud coding agents (Cursor/Codex/Claude cloud), and the existing Pi runtime (upgraded).
Agents get self-improvement (user profile + agent-owned skills), chat control across local AI tools, worktrees, a local
LLM proxy, and a portable/bridged MCP surface.

## 3. Base vs target inventory table

| # | Feature | Base | Target | Delta id |
|---|---|---|---|---|
| 1 | `pipe_executions` columns | id..duration_ms, session_path | + `trigger_event`, `trigger_key`, `conversation_id` | -001 |
| 2 | Event idempotency | in-memory per tick | table `pipe_event_runs` claims | -002 |
| 3 | pipe.md fields | see §1 | + `effort`, `history` (typed), `run_in`, `artifacts`, `schedule_config`, `cloud_agent`, `trigger.sources`, `event_read_only` | -003 |
| 4 | Structured schedule | string only | `schedule_config` + `POST /pipes/schedule/preview` | -004 |
| 5 | Pipes API | §1 | pagination, `/pipes/activity`, `/pipes/:id/executions/:exec_id`, `/pipes/bundled/:id/install`, richer run body | -005 |
| 6 | Pipe output stream | none | `/pipes/:id/stream` (SSE) | -006 |
| 7 | Scheduler | §1 | dedup/defer/coalesce, `.trigger-context.json`, `connection_trigger`, `workflow_ready:*`, `meeting_summary_refresh_requested`, preset `*` | -007 |
| 8 | Connection triggers | none | watcher over Slack/Notion/IMAP/GCal/Outlook/GitHub/Linear/Todoist/Obsidian/voice | -008 |
| 9 | Run in chat / continuity | `history` extra key | `run_in {mode:existing_chat, chat_id}`, conversation_id, `.screenpipe-acp-sessions.json` | -009 |
| 10 | Pipe artifacts | none | `artifacts:` declarations, `output/`, `/artifacts/register` | -010 |
| 11 | Structured output targets | none | `structured_output` tool, `/outputs/targets*` | -011 |
| 12 | Built-in pipes | 7 | 21 + builtin migrations | -012 |
| 13 | Permission defaults | reader allowlist v1 | + outputs/starred/workflows/feedback; MCP access gate | -013 |
| 14 | Pipe/chat Pi extensions | 6 core + 2 app | + self-improvement, skill-learning, register-artifact, structured-output, workflow workspace, chat-control, connection-gate, save-artifact, live-view, video-sop, fill_work_context, refine_workflow, local web-search | -014 |
| 15 | Pi runtime | pi 0.75.4 | pi 0.84.1 + managed deps, isolated config seed, baseline bun, compaction patches | -015 |
| 16 | ACP agents | none | catalog of 7 + custom, Rust ACP runtime, login/install | -016 |
| 17 | AI presets | 8 providers | + `acp` provider with `acpAgent`; thinking level | -017 |
| 18 | Chat runtime surface | §1 | new Tauri commands, agent_event types | -018 |
| 19 | External chats | none | Claude Code/Codex session import; chat-control broker | -019 |
| 20 | Agent worktrees | none | `start_worktree`, managed git worktrees | -020 |
| 21 | Agent skills API | device import only | `/agent/skills/manage` + provenance manifest | -021 |
| 22 | User profile | none | `/agent/profile/manage`, `/agent/self-improvement/context` | -022 |
| 23 | Skill learning | none | `skill-learning` pipe + `/agent/learning/chats` | -023 |
| 24 | Bundled skills | api/cli/team | + chats, workflow-maintenance, video-sop, 20 starter skills | -024 |
| 25 | Skills registry/export | none | curated registry install; export to 6 external agents | -025 |
| 26 | Workflows | `workflow_event` only | `/workflows*` catalog/workspace/pipeline/context/edits/corrections | -026 |
| 27 | Workflow cloud gating | n/a | rollout/Business/allowance gates | -027 |
| 28 | Cloud coding agents | none | `/cloud-agents/*`, `cloud_agent` pipes | -028 |
| 29 | Provider automations | none | list/manage Claude & Codex scheduled jobs | -029 |
| 30 | Built-in MCP server | 0.18.10, 27 tools | 37 tool names, portable MCP launcher, screenpipe-tools | -030 |
| 31 | MCP client setup | onboarding tiles | `screenpipe agent setup` targets x10 | -031 |
| 32 | BYO MCP | §1 | `/mcp-servers/registry`, session gate, `sp_mcp_read`, Composio | -032 |
| 33 | Local LLM proxy | none | `/v1/local/chat/completions` | -033 |
| 34-40 | Cloud deps | cloud proxy, store, model gateway | + web search, tinfoil GLM, telemetry, feature flags, npm CLI | -034..-040, -045 |
| 41 | Enterprise | ee/ removed | managed pipes/skills/presets | -041 |
| 42 | Trajectories | none | JSONL run trajectories | -042 |
| 43 | Connect broker | none | `/v1/connect-request`/`-response` | -043 |
| 44 | Feedback | none | `feedback` table, `/feedback` | -044 |
| 46 | Workflow skill drafting | none | `generate_workflow_skill`/`save_workflow_skill` | -046 |

---

## 4. Delta items

### agents_pipes-001 - `pipe_executions` new columns and query surface (changed)

**Interop.** Two new sqlx migrations (descriptions as embedded): `create pipe event runs` (also adds two columns) and
`add pipe execution conversation id`. Record verbatim:

```sql
ALTER TABLE pipe_executions ADD COLUMN trigger_event TEXT;
ALTER TABLE pipe_executions ADD COLUMN trigger_key TEXT;
ALTER TABLE pipe_executions ADD COLUMN conversation_id TEXT;
```

**Behaviour.**
- Insert now records the event that caused the run: columns `(pipe_name, status, trigger_type, model, provider, started_at, trigger_event, trigger_key)` with status `'queued'`, `RETURNING id` (S:0xa2b6bb8).
- `trigger_type` values seen: `manual`, `scheduled`, `event`, `connection` (S:0x499d600, S:0x499d460); `trigger_event` holds the event name (e.g. `meeting_ended`, `connection_trigger`), `trigger_key` the instance key (e.g. meeting id).
- `conversation_id` is a stable chat identity for executions that continue the same agent session; set after the run via `UPDATE pipe_executions SET conversation_id = ? WHERE id = ?` (S:0x6a02058).
- Running transition is guarded: only rows still `queued`/`running` with no `finished_at` move to `running`; pid coalesced; `started_at` preserved if set (S:0x6a01220).
- Finish: status/finished_at/stdout/stderr/exit_code/error_type/error_message/session_path written and `duration_ms` computed in SQL from julianday difference (S:0x6a00d18).
- Startup orphan recovery marks `running` **and** `queued` rows `failed` with `error_type='interrupted'` (S:0x6a010e0).
- Pagination: list per pipe newest-first, optionally `id < before_id`, with a variant that blanks stdout/stderr (`'' AS stdout`) for cheap listing (S:0x6a008a0, S:0x6a016f0).
- Activity summary across pipes: per pipe `execution_count`, `latest_execution_id`, `last_run_at` (=COALESCE(finished_at, started_at) of latest finished row), `status`, excluding queued/running, keyset-paged on latest id (S:0x6a01368).
- Retention: keep newest N per pipe using `ROW_NUMBER() OVER (PARTITION BY pipe_name ORDER BY id DESC)` (S:0xa2b6c69). N inferred unchanged from base cleanup.
- Meeting-summary finaliser reads/updates rows by `trigger_event='meeting_ended'` and `trigger_key` (S:0x6ab3051..0x6ab38db): flips a `completed` run with no saved summary to `failed`/`summary_not_saved`, requeues runs whose `error_type='interrupted'` (limit 20, newest without a newer sibling).
- error_type values added: `interrupted`, `summary_not_saved`, `nothing_to_summarize`, `ai_preset_unavailable`, `timed_out`/`timeout` (S:0x499de88, S:0x499d946).

**Base files.** new migration files in `crates/screenpipe-db/src/migrations/` (e.g. `2026xxxx_pipe_event_runs.sql`, `2026xxxx_pipe_exec_conversation_id.sql` - Freenpipe timestamps, identical DDL); `PipeExecution` struct + `PipeStore` trait in `crates/screenpipe-core/src/pipes/mod.rs`; SQLite impl in `crates/screenpipe-engine` (pipe store impl, look for `create_execution`).

**Notes.** Keep column order compatible; add `trigger_event`, `trigger_key`, `conversation_id` to `PipeExecution` serialization (fields observed in serde list S:0x6187698). Add `create_execution(..., trigger_event, trigger_key)` and `set_conversation_id`. Confidence: observed.

### agents_pipes-002 - Event-run idempotency table `pipe_event_runs` (new)

**DDL (verbatim, migration `create pipe event runs`, S:0xa2aa964):**

```sql
CREATE TABLE IF NOT EXISTS pipe_event_runs (
    pipe_name  TEXT NOT NULL,
    event_name TEXT NOT NULL,
    event_key  TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    PRIMARY KEY (pipe_name, event_name, event_key)
);
CREATE INDEX IF NOT EXISTS idx_pipe_event_runs_claimed_at ON pipe_event_runs(claimed_at);
```

**Behaviour.** Before an event-triggered run the scheduler claims `(pipe, event, key)` with `INSERT OR IGNORE`; the
primary key makes the claim atomic. If no row was inserted the run is suppressed and a log line records that the
duplicate was skipped (S:0x499d7be). If the run is cancelled before it starts, or cannot start, the claim is released with
`DELETE ... WHERE pipe_name=? AND event_name=? AND event_key=?` (S:0x499d970, S:0x499dd80). Claims are pruned by age with
`DELETE FROM pipe_event_runs WHERE claimed_at < ?` (S:0xa2b6ec5; a warning is logged if pruning fails, S:0x499d4c0). Motivation
in the migration comment: `meeting_ended` can fire twice for the same meeting id because of a rejoin window. Meeting
routes also query claims for `event_name='meeting_ended'` with `event_key = ? OR event_key LIKE ?` (prefix match for
suffixed keys like `<id>:summary-recovery-...`, S:0x620a358, S:0x6ab3644).

**Implementation.** Add `claim_event_run`, `release_event_run`, `prune_event_runs(older_than)` to `PipeStore`. Prune window
not observed: use 30 days (inferred). Key derivation: for `meeting_ended` the meeting id; for connection triggers the
`source_delivery_id`; for workflow events the event payload id. Confidence: DDL observed, key derivation inferred.

### agents_pipes-003 - PipeConfig frontmatter expansion (changed)

**Target serde field list** for `PipeConfig` (S:0x61874e0, S:0xa11740c): `name, schedule, enabled, agent, model, provider,
cloud_agent, effort, preset, connections, permissions, timeout, trigger, source_slug, installed_version, source_hash,
subagent, history, run_in, privacy_filter, artifacts, schedule_config` (+ flattened extras).

| Field | Type | Behaviour |
|---|---|---|
| `effort` | `low`\|`medium`\|`high` (`PipeEffort`) | Reasoning/thinking effort passed to the agent (Pi thinking level, ACP `reasoning_effort`). Invalid value rejected on config update with an error naming the three allowed values (S:0x517b052). |
| `history` | bool | When true, the pipe continues its previous agent session (Pi `--continue` / ACP resume); when false a fresh session. `has_saved_context` in status reports whether a session exists. Was an untyped extra in base. |
| `run_in` | object `{mode, chat_id}` (`PipeRunIn`), mode `existing_chat` | Deliver the run into an existing chat conversation (see -009). Validation: invalid chat id rejected (S:0x517afbc). |
| `artifacts` | list of `ArtifactDeclaration {path, kind?}` | Declared output files under the pipe folder, normally `output/...`; kinds `html`, `json`, `markdown` (S:0x6187330). Paths escaping the pipe directory are skipped with a warning (S:0x61872f9). |
| `schedule_config` | `ScheduleConfig` (see -004) | Structured schedule, compiled to the effective schedule. |
| `cloud_agent` | `CloudAgentConfig` or null | Used when `agent: cloud-agent` (see -028). |
| `trigger.sources` | list of `SourceTrigger {app, instance, filter, kind?, path?}` | Connection triggers (see -008). Serde shows `SourceTrigger` with 5 fields (S:0x52c9740) and keys `kind instance path filter items` (S:0xa1166a8). |
| `event_read_only` | bool (extra) | Seen in the `speaker-reconciliation` built-in; automatic (event) runs are restricted to read-only behaviour (inferred). |
| UI extras | `title`, `description`, `icon`, `featured`, `template` | Unchanged semantics, now present on all built-ins. |

Other observed status fields (`PipeStatus`, S:0x61874e0): `config, last_run, last_success, is_running, has_saved_context,
is_bundled_builtin, prompt_body, raw_content, last_error, current_execution_id, consecutive_failures, source_slug,
installed_version, locally_modified, next_run`, plus list-only `execution_count`, `recent_executions` (dev mock, F:_next/static/chunks/app/layout-3ec03e167c097e07.js).

**Config update rules** (`POST /pipes/:id/config`): body fields are top-level (a body wrapped in a `config` envelope is
rejected with an explanatory error, S:0x517af0e); `raw_content` and `timeout` are accepted; each of `schedule_config`,
`trigger`, `run_in`, `effort`, `cloud_agent` is validated separately (S:0x517af73..0x517b0b1). Managed (enterprise) pipes
cannot be edited, enabled/disabled or deleted locally (S:0x517abad, S:0x517acde, S:0x517aff4).

**Base files.** `crates/screenpipe-core/src/pipes/mod.rs` (`PipeConfig`, `PipeStatus`, `update_config`, `serialize_pipe`),
`apps/screenpipe-app-tauri` pipe settings UI. Confidence: field names observed; semantics of `event_read_only` inferred.

### agents_pipes-004 - Structured schedules and schedule preview (new)

**Shape** `ScheduleConfig` (S:0x6187760, S:0x52c9950): `frequency` (`minutes`|`hours`|`days`|`weeks`|`months`),
`interval` (int >= 1), `days_of_week` (int list), `day_of_month` (int or null), `at_hour`, `at_minute`, `timezone` (IANA),
`starting` (RFC3339), `ending` (RFC3339 or null), `max_occurrences` (int or null).

UI defaults (F:_next/static/chunks/5977-dd591e0cc8c27cfa.js @474329): weeks, interval 1, days_of_week [1], at 09:00, local
timezone, starting = today 00:00, never ends. The UI converts legacy strings (`every 30m`, `every 2h`, daily-at, simple
5-field crons) into this structure (@478055).

**Endpoint** `POST /pipes/schedule/preview` (route S:0x5258c98; UI call @479463, debounced 350 ms): body = ScheduleConfig;
response `{summary: string, next: [ISO timestamps]}`. Summary is a human sentence; `next` are the upcoming fire times. The UI
blocks preview for incomplete configs (weekly with no days).

**Scheduler.** The pipe's effective schedule is derived from `schedule_config` when present; `ending`/`max_occurrences`
stop the schedule (the pipe auto-disables, like base one-off). `next_run` is reported in status. Weekday numbering
(cron-style, 0=Sunday) is inferred.

**Base files.** `pipes/mod.rs` (`parse_schedule`, `should_run`), `engine/pipes_api.rs` (new handler), server route table.
Confidence: shapes observed; occurrence-limit semantics inferred.

### agents_pipes-005 - Pipes HTTP API additions (changed)

Pipe sub-router (S:0x5258c20..0x5258ff8, top-level S:0x5259028):

| Route | Method | Shape / behaviour |
|---|---|---|
| `/pipes` | GET | `{data:[PipeStatus...], pagination:{limit, offset, total}}` (dev mock). Query flags (S:0x5185300): `include_executions`, `execution_limit`, `include_execution_counts`, `before_id`, `include_output`. |
| `/pipes/bundled/:id/install` | POST `{}` | Copies a built-in pipe from the app bundle into `pipes/`; error if copied but not loadable (S:0x5185450). Used by onboarding and workflow-task setup (F:...app/onboarding/page-960f31d90328bcc8.js @99394). |
| `/pipes/schedule/preview` | POST | see -004 |
| `/pipes/activity` | GET `?limit&before_id` | `{data:[{pipe_name, execution_count, latest_execution_id, last_run_at, status}], has_more, next_before_id}` (home page @615016, S:0x6a01368). |
| `/pipes/:id` | GET | Now returns HTTP 404 when missing (UI branches on status 404). |
| `/pipes/:id/executions` | GET `?limit&before_id&include_output` | `{data:[PipeExecution], has_more}`. stdout/stderr empty unless `include_output`. |
| `/pipes/:id/executions/:exec_id` | GET | One execution with full output (route S:0x5258e38). |
| `/pipes/:id/run` | POST | Body `RunPipeBody` (S:0x51853b0): `notification_context`, `run_context`, `trigger_type`, `meeting_summary {meeting_id, meeting_end}`. Refuses to start when the execution store is unavailable (an untrackable-run error, S:0x5184e60). Meeting-summary manual runs require positive `meeting_id` and `meeting_end` (S:0x5184ec8) and set trigger_event `meeting_ended`, trigger types `summary_generation`/`manual_summary` (S:0x5184ff0). Response must carry the persisted execution id. |
| `/pipes/:id/stream` | GET | see -006 |
| `/pipes/:id/history` | DELETE | Clears remembered context: Pi sessions and ACP session map; refused while running (S:0x517b14c). |
| `/pipes/info` | GET | Agent-facing pipe info (in default allowlist; base already lists it). |

`EnableRequest` keeps `enabled`; enabling writes `.local-overrides.json` as base; `enabled_override` appears in the response
(S:0x5185488).

**Base files.** `crates/screenpipe-engine/src/pipes_api.rs`, `server.rs` (register before `/:id`), `pipes/mod.rs`
(`install_bundled_pipe`, `list_pipes_paged`). Confidence: observed routes and shapes; 404 semantics inferred from UI.

### agents_pipes-006 - Live pipe output stream `/pipes/:id/stream` (new)

Engine module `pipe_stream.rs` (S:0x6187ed8). Route `GET /pipes/:id/stream` (S:0x5259048). Server-sent events of a running
execution's NDJSON agent events, with `progress` and `keep-alive` frames (S:0x6187ed8 neighbourhood) and a cap on
concurrent streams (an error is returned once the stream limit is reached). Each stream ends with a `{"type":"pipe_done"}` line
(S:0x499e048). Recommended: cap 8 concurrent streams, keep-alive every 15 s (numbers inferred). Base touchpoints: base
already forwards output lines via `OnPipeOutputLine`; add a broadcast channel per running execution in `PipeManager` and
an axum SSE handler. Confidence: inferred (only route and messages observed).

### agents_pipes-007 - Scheduler behaviour changes (changed)

Observed scheduler log vocabulary at S:0x499d370..0x499e048.

1. **Event sources.** In addition to base topics, the scheduler consumes `meeting_summary_refresh_requested`,
   `workflow_event`, `connection_trigger` (S:0x499d460) and `workflow_ready:<pipe>` (built-in frontmatter, -012). Event
   names are matched exactly against `trigger.events`.
2. **Idempotency** via -002 claims; duplicate (pipe,event,key) suppressed.
3. **Busy handling.** If the target pipe is running, the event is *deferred* (not dropped) and retried; older duplicates for
   the same pipe are coalesced into the newest generation (S:0x499db5e..0x499dc0b); a backlog cap drops excess deferred
   triggers with a warning (S:0x499d5a9). Cap value not observed - use 32 (inferred).
4. **Trigger context file.** When woken by an event the scheduler writes `.trigger-context.json` in the pipe folder,
   shape `{"event": <name>, "key": <key>, ...}` (S:0xa10e2b3). Built-in prompts read it. If it cannot persist source
   context the run is deferred (S:0x499daca).
5. **Trivial meetings.** `meeting-summary` is skipped for trivial meetings (S:0x499d4ed); a meeting-summary finaliser
   (engine `meeting_summary/finalizer.rs`) post-validates runs (see -001).
6. **Preset resolution.** `preset` list entries are tried in order; the literal `"*"` stands for appending the user's configured
   presets (built-ins use `preset: [screenpipe-cloud, "*"]`, S:0xa10fe79). If no preset is configured the user's default
   preset is used (S:0x499d9e5). If a named preset is missing the run fails with error_type `ai_preset_unavailable` and
   **does not** fall back to another provider (S:0x499d8ed). Retry on a failing preset moves to the next fallback slot
   (S:0x499df0d).
7. **One-off pipes** are auto-disabled after firing; a skipped stale one-off is persisted as a skipped run
   (S:0x499d620..0x499d7a7). (Base already auto-disables; persistence of skipped row is new.)
8. **Context overflow.** If a run fails with context overflow the pipe's Pi session is deleted so the next run starts
   fresh (S:0x499df88).
9. **Log archive.** Run logs archived under `<pipe>/logs/%Y%m%d_%H%M%S...` by a dedicated blocking archive
   task (S:0x499d580); old executions cleaned periodically (S:0x499d536).
10. **Model fallback status.** Agents emit `{"type":"status","kind":"model_fallback","requested":..,"used":..}` and
    `{"type":"status","kind":"rate_limit_retry","wait_secs","attempt","max_attempts","capacity_reserved"}` lines
    (S:0xa0b9590, S:0xa0b78f1) which the UI renders.

**Base files.** `crates/screenpipe-core/src/pipes/mod.rs` (`start_scheduler`), `pipes/preset_fallback.rs`,
`crates/screenpipe-events`. Confidence: observed log strings; numeric caps inferred.

### agents_pipes-008 - Connection triggers watcher (new)

Core module `pipes/connection_triggers.rs` + `voice.rs` (S:0xa0e5de8..0xa0e69d5, S:0xa115fb8..0xa116d38).

- A background watcher (generation-tagged like the scheduler) polls connected apps **through the local engine's own
  connection endpoints** and fires pipes whose `trigger.sources` match. Supported apps (S:0xa0e5ff8):
  `slack` (`/connections/slack/history`, cursor via `oldest`/`next_cursor`), `notion` (`/connections/<id>/proxy/v1/search`
  and `/proxy/v1/databases/<id>/query`, `last_edited_time`), `imap` (`/connections/imap/messages?mailbox=INBOX`),
  `google-calendar` (`/connections/google-calendar/proxy/calendar/v3/calendars/primary/events` with timeMin/timeMax,
  singleEvents, orderBy=startTime), `outlook-email` (`/connections/outlook-email/proxy/me/mailFolders/{inbox|sentitems}/messages`
  with `$top/$orderby/$select`), `github` (`/connections/github/proxy/repos/<owner>/<repo>/issues?state=all&sort=created&direction=desc&per_page`),
  `linear` (GraphQL via `/connections/linear/proxy/graphql`, two queries: issue activity history and created issues),
  `todoist` (`/connections/todoist/proxy/api/v1/tasks`), `obsidian` (vault path watch; error if not a directory,
  S:0xa0dd338), and **voice** (phrases detected in new `audio_transcriptions` rows via `/raw_sql`, selecting rows with
  `id >` cursor, `LIMIT 50`, `is_input_device`).
- Detected item kinds (S:0xa1162ba): `item`, `task`, `issue_created`, `issue`, `event_started`, `page`, `note`, `phrase`;
  Linear status/assignee changes produce descriptions naming the old and new status or the new assignee.
- Cursors persisted to `<pipes_dir>/.connection-triggers.json` (S:0xa11617e). First poll initialises the cursor without
  firing (an initialisation log line, S:0xa1169f0).
- Delivery emits event `connection_trigger` with `pipe_name` + `source_delivery_id` (S:0xa0e5e78); trigger_type
  `connection`; the scheduler awaits completion; after N failed attempts the delivery is abandoned and the cursor advanced
  (S:0xa1165c4, S:0xa1167b0). N not observed: use 3.
- `filter` is a free-text/structured match on the item (inferred: case-insensitive substring over title/text; for voice,
  `phrases` list).

**Base files.** new `crates/screenpipe-core/src/pipes/connection_triggers.rs`; scheduler wiring in `pipes/mod.rs`;
needs local API port + pipe token. Confidence: endpoints observed; matching semantics inferred.

### agents_pipes-009 - Run destinations and conversation continuity (new)

- `run_in: {mode: "existing_chat", chat_id}` routes a scheduled run into an existing chat conversation instead of a
  standalone pipe execution view; only the streaming runtime supports it (an error says this destination needs the
  streaming runtime, S:0xa043cc8); chat id validated (S:0xa04942f). Core module `agents/chat_destination.rs`.
- When `history: true`, the run reuses the same agent session and its `conversation_id` is persisted on the execution row
  (-001). For ACP agents the per-pipe session map is `<pipe>/.screenpipe-acp-sessions.json` (S:0x517b108), cleared by
  `DELETE /pipes/:id/history`.
- Agent prompt gets a scheduled-task preamble: task directory and output directory `./output/` (S:0xa049378..0xa04940a).

**Base files.** `pipes/mod.rs` (`start_pipe_background`), `agents/pi.rs`, app `pi.rs` + chat storage (chat must accept
pipe-produced turns). Confidence: observed identifiers; UI behaviour inferred.

### agents_pipes-010 - Pipe artifacts and the outputs registry (new; shares storage with outputs dimension)

- DDL (migration `create outputs`, S:0xa2a1af2) - record verbatim:

```sql
CREATE TABLE IF NOT EXISTS outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'pipe',
    title TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'text',
    original_path TEXT,
    output_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    preview TEXT,
    metadata TEXT DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_outputs_source ON outputs(source);
CREATE INDEX IF NOT EXISTS idx_outputs_source_type ON outputs(source_type);
CREATE INDEX IF NOT EXISTS idx_outputs_created_at ON outputs(created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_outputs_output_path ON outputs(output_path);
ALTER TABLE outputs ADD COLUMN saf_kind TEXT;
ALTER TABLE outputs ADD COLUMN artifact_id TEXT;
ALTER TABLE outputs ADD COLUMN saf_version INTEGER;
CREATE INDEX IF NOT EXISTS idx_outputs_source_type_original_path_id
ON outputs(source_type, original_path, id DESC)
WHERE original_path IS NOT NULL;
```

  Files are copied to `~/.screenpipe/outputs/`. Scheduled-pipe artifacts get a run-specific `output_path`; unchanged
  snapshots of the same producer path are skipped (S:0xa2aacd2).
- Routes: `POST /artifacts/register`, `GET /artifacts` (`q`, `id`, `limit`, `offset` -> `{data, pagination, sources}`),
  `GET /artifacts/:id` (S:0x5257ca8 blob; dev mock).
- `register_artifact` Pi tool (core extension `register-artifact.ts`, S:0xa0ac9d0): params `file_path` (absolute, must
  exist and be readable), `title`, optional `kind` (`markdown`, `json`, `text`, `image`, else inferred from extension);
  posts `{source: <SCREENPIPE_PIPE_NAME>, source_type: "pipe", title, kind, file_path}`; only installed inside a pipe.
- `save_artifact` (app chat extension, S:0x529f648): text-based artifacts from chat; filename sanitised to a basename;
  written into a session-scoped temp dir so repeated saves upsert; uses `SCREENPIPE_CHAT_SESSION_ID`.
- After a run, declared `artifacts:` paths under the pipe folder are registered automatically (inferred from
  the validation that rejects artifact paths outside the pipe directory, and the outputs index comment).

**Base files.** new migration(s), new `crates/screenpipe-db/src/db/outputs.rs`, new `engine/src/routes/artifacts.rs`,
core extension asset `register-artifact.ts`, app extension `save-artifact.ts`. Coordinate with the outputs/artifacts
dimension owner (same tables). Confidence: DDL/routes observed; auto-registration inferred.

### agents_pipes-011 - Structured output targets for pipes (new; coordinate with live-views/outputs dimension)

- Pi tool `structured_output` (core extension, S:0xa0b0049 marker; params at L697885..698114): `action`
  (`get_targets`|`submit`), `target_id`, `target_revision` (int), `payload` (object matching target JSON schema),
  `evidence` (<= 50 items of `{event_id, frame_id, transcription_id, ts, device_id}`).
- `GET /outputs/targets?pipe=<name>` lists targets assigned to the pipe (each with `schema_name`, instruction, prior
  `output_payload`, `item_actions`). Before agent start the extension injects a short metadata block listing assigned
  targets (treated as data). Target instruction and `x-screenpipe-time-range` override the pipe's default lookback.
- `POST /outputs/targets/:target_id/submit` body `SubmitStructuredOutputRequest {target_revision, source_pipe, payload, evidence}`;
  `POST /outputs/targets/:target_id/feedback` (`SetStructuredOutputFeedbackRequest {rating, correction}`);
  `POST /outputs/targets/:target_id/items/:item_id/actions` (`SetStructuredOutputItemActionRequest`, includes
  `snoozed_until`, `item_actions`) (S:0x522d17b..0x522d344).
- Storage: `structured-outputs/` with `locks/`, `values/`, `*.saf.json` (field `saf_version`), artifacts registered as
  outputs `saf_kind=structured-output` (S:0x522d360..0x522d4e2). Pipes cannot rate or act on their own outputs
  (S:0x522d4fb, S:0x522d61b). Feedback stored in the unified `feedback` table (-044).
- Missing targets must not fail the pipe (comment summary).

**Base files.** core extension `structured-output.ts`; engine `routes/structured_outputs.rs`. Confidence: observed.

### agents_pipes-012 - Built-in pipes catalogue and built-in migrations (changed)

Bundled `pipe.md` set (frontmatter dump via `tools/ap_frontmatter.py`, L700998..L702339; duplicate copy at L692553..):

| Pipe | schedule | enabled | Key frontmatter |
|---|---|---|---|
| `workflow-discover` | every 1h | false | timeout 900, history false, trigger `workflow_ready:workflow-discover`, permissions allow `GET/POST /workflows/workspace`, `GET /workflows/context`, read APIs |
| `workflow-deepen` | every 5m | false | trigger `workflow_ready:workflow-deepen`, same permissions |
| `workflow-review` | every 5m | false | trigger `workflow_ready:workflow-review` |
| `workflow-maintain` | every 5m | false | trigger `workflow_ready:workflow-maintain` |
| `workflow-activity` | every 24h | false | timeout 600, `GET/POST /workflows/pipeline` |
| `workflow-patterns` | every 24h | false | trigger `pipe_completed:workflow-activity` |
| `workflow-procedures` | every 24h | false | trigger `pipe_completed:workflow-patterns` |
| `workflow-timing` | every 24h | false | trigger `pipe_completed:workflow-procedures` |
| `workflow-discovery` | every 24h | false | trigger `pipe_completed:workflow-timing`; allow `POST /workflows/catalog`, `GET /feedback`, `POST /notify` |
| `skill-learning` | every 6h | false | model `claude-haiku-4-5`, timeout 180, allow `GET /search`, `POST /agent/learning/chats`, `POST /agent/skills/manage`, artifacts `output/latest-change.md` |
| `automate-my-work` | manual | true | template, featured |
| `missed-todos` | manual | true | template, featured |
| `day-recap`, `standup-update`, `time-breakdown`, `ai-habits`, `video-export` | manual | true | as base (template) |
| `meeting-summary` | manual | true | preset `[screenpipe-cloud, "*"]`, timeout 600, trigger `meeting_ended` |
| `speaker-reconciliation` | manual | false | trigger `meeting_ended`, `event_read_only: true`, explicit allow/deny lists incl. `Content(audio, accessibility, parsed)` allow, `Content(input, ocr)` deny, artifacts `output/speaker-reconciliation.md` |
| `commitments` | every 30m | false | trigger `meeting_ended`, history false |
| `accounting-follow-through` | every day at 9am | false | history false |

Base `ai-prompt-journal` is not present in the target set (removed).

**Built-in migrations** (`pipes/builtin_migrations.rs`, S:0xa10e070..): an ordered list of text patches applied to
*installed* copies of built-in pipes when the user has not diverged, each identified by a short rationale (issue numbers
#4247, #5481, latency/portability items). Behaviour: for each migration, if the installed `pipe.md` contains the old
snippet, replace with the new snippet and save; skip silently otherwise. Patches include switching meeting saves to the
dedicated `PUT /meetings/:id` / meeting summary endpoint, reading `.trigger-context.json`, replacing `jq` with the bundled
bun JSON runtime, adding a pipe-local `memory.md` continuous-improvement section, adding `"*"` to meeting-summary's
preset list, and fixing post-summary notification buttons (`{"type":"pipe","open_in_chat":true,...}`).

**Implementation.** Freenpipe must write its own pipe bodies (do not copy target prompt text). Ship frontmatter matching the
table so state interoperates. Implement `builtin_migrations` as `(id, find, replace)` tuples applied idempotently,
recording applied ids in a sidecar (e.g. `.builtin-migrations.json`, Freenpipe-chosen name). Base files:
`crates/screenpipe-core/assets/pipes/*`, `pipes/mod.rs::install_builtin_pipes`. Confidence: frontmatter observed.

### agents_pipes-013 - Permission defaults and MCP access gate (changed)

- Default reader allowlist adds (S:0xa0c8030, S:0xa0c8088): `GET /outputs/targets`, `POST /outputs/targets/*/submit`,
  `GET /starred-sessions`, `GET /workflows`, `GET /workflows/*`, `GET /feedback` (base list kept).
- Writer additions unchanged (`POST /meetings/start` ... `DELETE /memories/*`).
- Middleware messages (S:0x618835f neighbourhood): a pipe calling a route outside its rules gets an error naming
  the pipe and route and pointing at the `permissions` key in pipe.md; audio/speaker access can be denied separately; concurrent
  pipe queries are limited with recording-priority backpressure (a too-many-queries error, plus timeouts for pipe
  queries and workflow operations). Workflow routes `/workflows/workspace` and `/workflows/catalog` have their own limits.
- MCP access gate (`pipes/mcp_access.rs`): BYO MCP servers are filtered per session using header `x-screenpipe-session`
  and env `SCREENPIPE_MCP_SERVER_ALLOWLIST` (comma-separated server ids; unset = all enabled). Inferred: a pipe can only call
  servers in its allowlist.
- Skills denied by pipe permissions are removed from the pipe's skill dir (S:0xa0a6bd9).

**Base files.** `crates/screenpipe-core/src/pipes/permissions.rs` (`DEFAULT_ALLOWLIST`), `engine/pipe_permissions_middleware.rs`,
`engine/mcp_servers_api.rs`. Confidence: allowlist observed; MCP gating inferred.

### agents_pipes-014 - Pi extension set (changed)

Extensions written into `<project>/.pi/extensions/` (each logs an install line with its path, S:0x517b6f0..0x517b83a,
L695258..L698339 core; L190111..L192351 app):

| Extension (file) | Where | Tools / hooks | Notes |
|---|---|---|---|
| screenpipe-permissions | pipe | hooks | Adds a filesystem sandbox: tokenises bash commands (quote-aware) and blocks access outside allowed paths; reads `.screenpipe-permissions.json` and `SCREENPIPE_PIPE_NAME`. |
| orphan-guard, context-pruning | pipe+chat | hooks | Pruning: single tool result > 30K chars is replaced by size-limit feedback, `read` exempt; recent-message window protected. |
| sub-agent | pipe (opt-in `subagent: true`) | `sub-agent` | Now installs npm package `pi-subagents` (S:0x4992d90). |
| mcp-bridge | both | `sp_mcp_list_tools`, `sp_mcp_call`, `sp_mcp_read` | `sp_mcp_read` only for tools annotated `readOnlyHint=true`; list returns annotations and, with `server_id`, input schemas; MCP `isError` surfaced. |
| web-search | both | `sp_web_search` | cloud; see -035. |
| self-improvement | pipe+chat | `user_profile`, `skill_manage`; `before_agent_start` injects `/agent/self-improvement/context` | see -021/-022. |
| skill-learning | `skill-learning` pipe only | `learning_context`, `learning_inventory`, `learning_save` | see -023. |
| register-artifact | pipe | `register_artifact` | -010 |
| structured-output | pipe | `structured_output` | -011 |
| workflow workspace | workflow pipes | `workflow_workspace` | -026 |
| chat-control | chat | `search_chats`, `send_to_chat`, `start_worktree` | -019/-020 |
| connection-gate (app) | chat | `screenpipe_list_connections`, `screenpipe_connect_app` | -043 |
| save-artifact (app) | chat | `save_artifact` | -010 |
| Live Views (app) | chat | `screenpipe_live_view_propose`, `screenpipe_live_view` | live-views dimension |
| video-sop (app) | chat | `read_video_sop`, `edit_video_sop`, `render_video_sop` (env `SCREENPIPE_VIDEO_CLI`) | workflow-video dimension |
| work context (app) | Workflows UI chat | `fill_work_context` | proposal-only tool; UI merges it |
| refine (app) | Workflows UI chat | `refine_workflow` (`{learning <=1000 chars, changes}`) | app persists after success |
| local web-search (app) | ACP chat | `sp_web_search` via local proxy | -035 |
| screenpipe-tinfoil | chat/pipes on screenpipe-cloud | provider `screenpipe` API `screenpipe-tinfoil` | decloud -036 |

Per-pipe install failures are logged and non-fatal. The chat app also supports **third-party Pi extension packages**
(`pi_install_extension_package`, `pi_list_extension_packages`, `pi_remove_extension_package`; sources `npm:<pkg>`,
`npm:@scope/pkg` or GitHub URLs only, S:0x52ab6f6); if a required package fails to install the session starts in
a safe mode that loads only managed extensions, with third-party packages disabled (S:0x4990faf..0x499140d). bun install falls back to npm on
EPERM/hardware failures (S:0x5299294).

**Base files.** `crates/screenpipe-core/src/agents/pi.rs` (`ensure_*_extension`), `crates/screenpipe-core/assets/extensions/`,
`apps/.../src-tauri/src/pi.rs`, `apps/.../src-tauri/assets/extensions/`. Write all extension sources from scratch.

### agents_pipes-015 - Pi runtime upgrade and hardening (changed)

- Pins: `@earendil-works/pi-coding-agent@0.84.1`, `@earendil-works/pi-ai@0.84.1`; the managed `pi-agent/package.json` is
  seeded with these as dependencies **and** `overrides` (also `@anthropic-ai/sdk`, `tinfoil`), re-seeded if unreadable
  (S:0xa0b5c00). Install command `bun add ... --force --ignore-scripts` (S:0xa0b9a60). Base pins 0.75.4.
- Isolated Pi config: `SCREENPIPE_PI_AGENT_DIR`/`PI_CODING_AGENT_DIR` points at `~/.screenpipe/pi-agent/pi-config`; on
  first run it is seeded from the user's global Pi config (`auth.json`, `trust.json`, `settings.json` `packages`,
  `sessions/`) guarded by `.migration.lock` and marker `.migrated-from-global` (S:0xa0b42f8..0xa0b4ad6). Env
  `PI_SKIP_VERSION_CHECK=1`, `JITI_TRY_NATIVE=0`, `SCREENPIPE_CLI` (S:0xa0b67e0).
- `models.json` merge: managed providers merged into `pi-config/models.json`; providers without a baseUrl skipped; corrupt
  file rebuilt (S:0x517ba25..0x517bb73). Provider API kinds: `openai-completions`, `anthropic-messages`,
  `openai-codex-responses`, `screenpipe-tinfoil`; key vars `$CUSTOM_API_KEY`, `$ANTHROPIC_API_KEY`, `$OPENAI_API_KEY`,
  `$OPENAI_CHATGPT_TOKEN`; provider ids `anthropic-byok`, `openai-byok`, `ollama`, `openai-chatgpt` (S:0x4992a28).
  Extra headers `x-screenpipe-latency`, `x-screenpipe-workload`, `sendSessionAffinityHeaders`.
- Windows: baseline (non-AVX2) bun 1.3.10 downloaded from GitHub with SHA-256 check when the stock bun crashes with
  `0xC000001D` (S:0xa0b6228..0xa0b6496, S:0xa0b7285); bash candidate startup probe `printf screenpipe-bash-ok` with
  timeout; WSL bash launcher ignored; PortableGit 2.47.1 as base (S:0xa0b7148..0xa0b8805). Pi `shellPath` setting written
  (S:0x4991680).
- **Compaction patch** (`agents/pi_compaction.rs`, S:0xa0445f3..0xa049318): the runtime applies a pinned diff to the
  installed pi dist files (markers `screenpipe-context-compaction-v1`, `screenpipe-compaction-failure-v1`,
  `screenpipe-summary-evidence-v1`, `screenpipe-summary-contract-v1`, `screenpipe-summary-validation-v1`,
  `screenpipe-transport-cause-v1`). Behaviour to reproduce (paraphrased): compact proactively at 70% of the context window
  after a tool step; size reserve/keep-recent tokens from observed prompt overhead; keep 8K-char tool excerpts (head+tail)
  in summaries; serialise history as JSON records for the summariser; reject incomplete summaries; abort the run instead
  of looping when compaction fails; append recognised network error codes to transport errors. Refuse to run if the
  installed runtime does not match the patch.
- Rate limit / capacity: retries with `reset_in`/`retry_after_seconds`, status lines (-007). Errors classified
  `daily_cost_limit_exceeded`, `daily_limit_exceeded`, `credits_exhausted`, `model_not_allowed`, `hosted_ai_capacity_reserved`
  (S:0xa0b5251) - cloud-only (-036).
- Effort: `pipe streaming using provider/model/effort` (S:0xa0b9609); thinking level per chat (`pi_set_thinking_level`).

**Base files.** `crates/screenpipe-core/src/agents/pi.rs`, new `agents/pi_compaction.rs`, app `src-tauri/src/pi.rs`.
Implementation: implement compaction as a Freenpipe Pi extension or patch written independently; do not copy the diff.
Confidence: observed identifiers; patch semantics paraphrased.

### agents_pipes-016 - ACP (Agent Client Protocol) agent runtime (new)

**Catalog** (frontend constant, F:_next/static/chunks/7451-47878874a9162526.js, module 9984; images under
`frontend/images/acp/`). Entries `{id, name, imageSrc, presetName, description, launch, login?, installUrl?, installer?, httpMcp?, flag?, cloudRouting?, disabled?, invertInDark?}`:

| id | launch | extras |
|---|---|---|
| `pi-acp` | npx `pi-acp@0.0.32` | |
| `codex-acp` | npx `@agentclientprotocol/codex-acp@1.1.7` | |
| `claude-acp` | npx `@agentclientprotocol/claude-agent-acp@0.66.0` args `--hide-claude-auth` | `cloudRouting {baseUrlEnv: ANTHROPIC_BASE_URL, tokenEnv: ANTHROPIC_AUTH_TOKEN, pathPrefix: /anthropic, clearEnv: [ANTHROPIC_API_KEY]}` |
| `opencode` | binary `opencode acp`, login `auth login` | disabled |
| `cursor` | binary `cursor-agent acp`, login `login` | installer shell script (cursor.com), `httpMcp`, flag `acp_agent_cursor` |
| `github-copilot-cli` | npx `@github/copilot@1.0.74 --acp` | `httpMcp`, flag `acp_agent_github_copilot` |
| `kimi` | binary `kimi acp`, login | disabled |
| `custom` (UI label: a generic agent-command entry) | user command | flag `acp_agent_custom` |

Display order preference: codex-acp, claude-acp, cursor, github-copilot-cli, pi-acp.

**Runtime.** The app/engine spawn the app executable itself with `--screenpipe-acp-runtime` (plus a
`--screenpipe-acp-process-guard` helper) which hosts an ACP client (Rust crate `agent-client-protocol` 1.2.0) talking to
the agent over stdio, translating ACP updates into the Pi-compatible JSON event stream the chat UI already consumes
(`/assistantMessageEvent/delta`, `agent_end`, ...; S:0xa0bf360). Environment contract (S:0xa0bef96..0xa0bf167, S:0x4991508):
`SCREENPIPE_ACP_ID`, `SCREENPIPE_ACP_CWD`, `SCREENPIPE_BUN_PATH`, `SCREENPIPE_ACP_UNATTENDED=1` (pipes), `SCREENPIPE_ACP_ARGS_JSON`,
`SCREENPIPE_ACP_ENV_JSON`, `SCREENPIPE_ACP_SESSION_CONFIG_JSON`, `SCREENPIPE_ACP_COMMAND`, `SCREENPIPE_ACP_AUTH_METHOD`,
`SCREENPIPE_ACP_SYSTEM_PROMPT`, `SCREENPIPE_ACP_RESUME_SESSION_ID`, `SCREENPIPE_ACP_TOOL_ALLOWLIST`, `SCREENPIPE_ACP_USER_MCP_JSON`,
plus session/chat/MCP vars and `DISABLE_MCP_CONFIG_FILTERING`. Stdout lines that are not JSON are ignored; oversized lines
dropped (S:0x5229f80). Runtime process tree is contained (job object) and reaped.

**MCP for ACP agents.** Each ACP session gets the `screenpipe-tools` MCP server (`screenpipe-tools.mjs`, dependency-free,
bundled bun, protocol `2025-06-18`, server info `screenpipe-tools 0.1.0`) over stdio, or Streamable HTTP when
`SCREENPIPE_TOOLS_HTTP_PORT` is set for agents flagged `httpMcp` (L699104..). Tools: `query_recordings`, `search_chats`,
`send_to_chat`, `start_worktree`, `list_connections`, `save_artifact`, `sp_web_search`, `screenpipe_connect_app`,
`live_view`, `user_profile`, `skill_manage`, `sp_mcp_list_tools`, `sp_mcp_call`; in HTTP mode also `activity_summary`,
`keyword_search`, `search_elements`, `frame_context`, `get_frame_elements`, `list_meetings`, `get_meeting`, `health_check`.
It never advertises an MCP server pointing back at the local engine. ACP extensions (`acp/extensions.rs`) can also start
the portable screenpipe MCP (-030).

**Session config & approvals.** ACP session config options (`SessionConfigSelectGroup`, select/boolean options, modes)
are cached per session/agent (`useAcpSessionConfig`), and stored in the preset as `acpAgent.config`, `modeId`; approval
mode `ask` (default) or `allow-all` (F:...3696-4fbfea4ced3de899.js). Agent events: `acp_ready`, `acp_status`
(`downloading`), `acp_update`, `acp_session_config`, `acp_authenticated`, `acp_auth_cancelled`, `acp_auth_error`,
`acp_external_auth_required`, `acp_fatal`, `acp_reinit_resend`.

**Install/login.** Tauri commands `pi_acp_agent_install`, `pi_acp_agent_install_status`, `pi_acp_agent_download_pending`,
`pi_acp_probe_agent`, `pi_acp_external_login`, `pi_acp_reauthenticate`, `pi_acp_set_mode` (S:0x5177a68). Messages cover:
an agent that needs no separate install; platforms where automatic install is unavailable (the user is pointed to the
vendor's installer); an agent binary missing from PATH; and a login timeout (S:0x499ed40..0x499efb9). Custom ACP agents
require a command.

**Pipes on ACP.** A pipe whose preset has provider `acp` runs through the ACP scheduled-task runtime (streaming runner
required, S:0xa0bed18).

**Base files.** new `crates/screenpipe-core/src/agents/acp/*`, app `src-tauri/src/pi.rs` (backend switch), settings UI,
`lib/utils/tauri.ts` bindings. Freenpipe: implement ACP client against the public ACP spec; npm-launched agents use
`bun x` (no screenpipe cloud). Confidence: observed.

### agents_pipes-017 - AI presets: `acp` provider, thinking level (changed)

- `AIProviderType` adds `acp`; preset gets `acpAgent: AcpAgentConfig {id, command, args, env, authMethod, config,
  modeId, approvalMode, useScreenpipeCloud}` (9 fields, S:0x52c8150) and `enterpriseManaged` (S:0x52c8248). Validation: an
  ACP preset needs `acpAgent.id` (S:0xa0bf320).
- Preset id/name defaults derive from provider/agent (`presetName` of the ACP entry, or sanitised model id, or provider
  label map `openai-chatgpt->chatgpt`, `anthropic->claude`, `native-ollama->ollama`, ...); collisions get " 2", " 3".
- Provider icons map includes `acp: /images/acp.svg`.
- `PiProviderConfig` sent to `pi_start` adds `backend` (`acp`|null), `acpAgent`, `systemPrompt`, `allowedTools`,
  `resumeSessionId`, `unattended` (S:0x52c8680). `maxContextChars` default for new ACP presets 512000.
- A chat remembers its preset (`presetId` saved per conversation).

**Base files.** `apps/.../lib/hooks/use-settings.tsx`, `lib/enterprise-ai-preset-policy.ts`, `src-tauri/src/store.rs`,
`src-tauri/src/pi.rs`, `crates/screenpipe-engine/src/cli/presets.rs`. Confidence: observed.

### agents_pipes-018 - Chat runtime surface (changed)

New Tauri commands (absent in base `lib/utils/tauri.ts`; S:0x5177a68): `pi_start_and_prompt`, `pi_stop_if_idle`,
`pi_request_state`, `pi_get_thinking_level`, `pi_set_thinking_level`, `pi_extension_ui_response`, extension package
commands (-014), ACP commands (-016), `list_chat_entries_by_mtime`, `get_workflows_runtime`. Pool behaviour: start stages
(`requested`, `config_ready`, `waiting_for_pool`, `pool_acquired`, `process_spawned`), refusal to restart a busy session,
an error when the pool is at capacity and every session is busy (S:0x4991251). Agent event types consumed by the UI (frontend scan):
`agent_start`, `agent_end`, `agent_action`, `message_start/update/end`, `thinking_start/delta/end`,
`tool_execution_start/update/progress/end`, `tool_result`, `tool_use`, `compaction_end`, `pipe_done`,
`thinking_level_changed`, `extension_ui_request` (incl. `setStatus` key `screenpipe-confidential`), plus ACP events. Tool
labels map ACP agent ids to friendly names (F:...2097-3b7ee8e30a63eb93.js).

Chat conversation file fields (dev mock): `id, title, titleSource, kind, pinned, createdAt, updatedAt, lastUserMessageAt,
lastContentAt, lastViewedAt, importedFrom, messages, presetId`. Conversations can be archived (hidden) and branched.

**Base files.** app `src-tauri/src/pi.rs`, `pi_command_queue.rs`, `components/standalone-chat.tsx`,
`components/hooks/use-chat-conversations.ts`, `lib/chat-storage.ts`. Confidence: observed command names; flows inferred.

### agents_pipes-019 - External chat import and chat-control broker (new)

- **Import.** The home recents list syncs local AI-tool sessions: Claude Code `~/.claude/projects/**/*.jsonl` and Codex
  `~/.codex/sessions/**/*.jsonl` (F:app/(main)/home/page-c18385b042b3382e.js @601799; parser F:378-9f286ebe7c0f0713.js).
  Imported conversations carry `importedFrom {source: "claude-code"|"codex", sourceId, importedAt, config {model,
  reasoningEffort, modeId}, harness}`; tool-output tags (bash stdout etc.) are stripped; a live Codex turn shows a
  placeholder assistant message. Recents filter sources `screenpipe`, `codex`, `claude-code`. Continuing an imported chat
  auto-selects or creates an ACP preset (`codex-acp` or `claude-acp`) matching the original model/effort/mode.
- **Chat-control broker.** The app runs a local TCP broker; agents get `SCREENPIPE_CHAT_CONTROL_ADDR` (`host:port`) and
  `SCREENPIPE_CHAT_CONTROL_TOKEN` (disabled with `SCREENPIPE_CHAT_CONTROL_DISABLED=1`). Protocol: one newline-terminated
  JSON request `{id, token, action, payload}` -> one line `{id, ok, data|error}`; 20 s client timeout
  (L696806..696870). Actions: `search` (`{query, sources[], limit<=50}`; sources `screenpipe`, `codex`, `claude`,
  `cursor`, `gemini`), `send` (`{source, id, message<=20000, mode: queue|steer, confirmed:true, origin_session_id}`;
  Gemini is search-only), `worktree` (-020). Broker rules (S:0x499e5a0..0x499ea5a): exact ids only, `confirmed=true`
  required, steer only for a running screenpipe chat, refuse sending a chat to itself, message size cap, Cursor resume
  starts a process, Codex/Claude accept queued/background messages.
- **Learning search.** `/agent/learning/chats` (-023) reads the same chat corpus.

**Base files.** new app `src-tauri/src/chat_control.rs`, core `agents/chat_control.rs`, chat-control extension;
frontend recents sync. Confidence: observed protocol; parsing details inferred.

### agents_pipes-020 - Agent worktrees for coding chats (new)

Core `agents/worktree.rs` + app `coding_workspace.rs` (S:0xa04a0e0..0xa04aaf8). When a chat session id starts with
`__worktree-route:` the agent receives `start_worktree(repository_path)` (exactly once, from a candidate list) and the
broker creates an isolated git worktree for the conversation. Invariants: record file `agent-worktree-owner.json`
(`version, ownerId, repoRoot, gitCommonDir, worktreePath, baseCommit, sourceDirty, createdAt`), deterministic managed
location under a `worktrees` store, managed branch prefix, git run with hooks and fsmonitor disabled
(`GIT_CONFIG_KEY_0=core.hooksPath`, `core.fsmonitor`, `GIT_TERMINAL_PROMPT=0`), refuse to claim pre-existing branches,
keep the worktree if removal would lose work, repo must have at least one commit, scan for candidate repos under common
folders (Documents, Desktop, Developer, Projects, src) skipping caches/node_modules/target. Agent system prompt tells it to
work only inside its worktree. Confidence: observed strings; algorithm inferred.

### agents_pipes-021 - Agent skills store and `/agent/skills/manage` (new)

**Route** `POST /agent/skills/manage` (S:0x5258510). Request: `{action: "list"|"read"|"create"|"patch", name?,
description?, instructions?, expected_sha256?, confirmed?, source}` where `source` is the session/pipe id or
`skill-learning`. Responses (from extension usage, L697692..697745):
- list -> `{skills: [{key, name, description, origin, sha256}]}`
- read -> `{skill: {key, name, description, instructions, sha256, origin}}`
- create/patch -> `{skill: {..., sha256}}`.

**Rules** (engine `agent_skills.rs`, S:0x6a0f32f neighbourhood L632775..632810):
- Origins: `user` (imported/hand-authored, read-only to agents), `agent` (created by an agent), bundled screenpipe skills
  (immutable).
- Provenance manifest `.screenpipe-agent-skills.json` in the skills store: `{schema, skills: {<name>: {sha256,
  created_by: "screenpipe-agent", updated_at}}}`; a skill without a valid marker is not patchable.
- `create` requires `confirmed:true` and fails if the name exists (the error tells the agent to read the existing skill
  first). `patch` requires the current `expected_sha256` (stale -> an error saying the skill changed after it was read)
  and agent origin.
- Validation: name/description/instruction length limits; content rejected if it matches unsafe-pattern ids
  `prompt_injection`, `system_prompt_override`, `disregard_rules`, `role_hijack`, `role_pretend`,
  `system_prompt_exfiltration`, `html_comment_injection`, `hardcoded_secret`, `secret_material`, `exfiltration_url`.
  Limits: use description 1..300 chars, instructions 1..20000 bytes (inferred; learning pipe enforces 3000/6000).
- Writes are atomic (temp + rename) and produce `SKILL.md` with frontmatter `name`, `description`.
- Store location: base skills store `<data_dir>/skills/<name>/SKILL.md`, mirrored into `.pi/skills` for each session.

**Base files.** new `crates/screenpipe-engine/src/agent_skills.rs`, route in `server.rs`, app `skills.rs` (list now shows
origin). Confidence: observed rules/ids; numeric limits partly inferred.

### agents_pipes-022 - User profile and self-improvement context (new)

- `POST /agent/profile/manage` (S:0x5258540): `{action: "list"|"save"|"delete", id?, content? (<=2000 chars),
  tags?, importance? (0..1), confirmed? (required for delete), source?}`. Profile facts are memories tagged
  `user-profile` (tag always added). Engine `agent_profile.rs` (S:0x6a43757..0x6a43c57): entries with instruction-like or
  secret-bearing content are stored/rendered as a BLOCKED placeholder; invisible-unicode and threat regex checks; tag
  length and total profile-memory caps.
- `GET /agent/self-improvement/context` (S:0x5258570) -> `{system_prompt}`: a rendered block containing short guidance for
  `user_profile`/`skill_manage` plus a frozen profile snapshot wrapped in `<screenpipe_user_profile_data>` tags and labelled
  as data. The extension caches it once per agent session so profile edits apply to new sessions only.
- Pi tools `user_profile`, `skill_manage` (core extension `self-improvement.ts`, L697365..697565); ACP equivalents in
  screenpipe-tools.

**Base files.** new `engine/src/agent_profile.rs`, memories DB layer (`crates/screenpipe-db/src/db/memories.rs`), core
extension asset. Confidence: observed.

### agents_pipes-023 - Skill-learning task and `/agent/learning/chats` (new)

- Built-in pipe `skill-learning` (-012) with config file `<pipe>/.screenpipe-learning-config.json` `{port}` written at
  run start (S:0x499e1f1) and the pipe token from `.screenpipe-permissions.json`.
- Extension `skill-learning.ts` (L697567..697750) restricts the agent to three tools: `learning_context` (sources
  `activity` -> `/search?content_type=all&limit=20&start_time=<now-24h>&filter_pii=true&q=`, or `chats` ->
  `POST /agent/learning/chats {query}`; max 4 calls, 5 items each, items deduped by SHA-256 of normalised text),
  `learning_inventory` (20 per page from `skill_manage list`, read at most 2 owned bodies), `learning_save` (one write
  per run). Saved skills must be named `screenpipe-learned-<slug>` (<= 80 chars), cite >= 2 new evidence refs incl. one
  from activity, provide exactly 3 check sentences, avoid private identifiers/URLs/paths/credentials and executable
  content; total owned learned skills capped at 30. State in `output/learning-state.json` (pending-write guard, used refs
  capped to last 300), report `output/latest-change.md`. Writes go through `/agent/skills/manage` with
  `source: "skill-learning"` and are read back to verify.
- `POST /agent/learning/chats` (S:0x52584e0): body `{query}` -> `{results|data: [{preview, updated_at, ...}], warnings}`
  over local chats (screenpipe chats and imported external chats). Response shape partly inferred.

**Base files.** new pipe asset, extension asset, engine route. Write the prompt text originally. Confidence: tool rules
observed; endpoint corpus inferred.

### agents_pipes-024 - Bundled and starter skills (changed)

- Bundled skill dirs: `screenpipe-api`, `screenpipe-cli`, `screenpipe-team` (base) + `screenpipe-chats`,
  `screenpipe-workflow-maintenance`, `video-sop` (S:0x52bd2d3, S:0xa0dae76, S:0xa05fb93, S:0x52a32c6). Deprecated skill dirs
  are removed on upgrade: `screenpipe-analytics`, `screenpipe-connections`, `screenpipe-elements`, `screenpipe-media`,
  `screenpipe-pipe-creator`, `screenpipe-pipes`, `screenpipe-retranscribe`, `screenpipe-search`, `screenpipe-qa`
  (S:0xa082bc3, L694616). Reserved names cannot be imported.
- Starter skills (20, S:0xa0c17d8..0xa0c6373): `screenpipe-recall`, `screenpipe-meeting-prep`, `screenpipe-meeting-follow-up`,
  `screenpipe-worklog`, `screenpipe-research-synthesis`, `screenpipe-focus-review`, `screenpipe-durable-learning`,
  `screenpipe-shareable-recap`, `screenpipe-decision-history`, `screenpipe-project-handoff`, `screenpipe-resume-work`,
  `screenpipe-commitment-review`, `screenpipe-project-status`, `screenpipe-customer-context`, `screenpipe-bug-report`,
  `screenpipe-incident-timeline`, `screenpipe-workflow-discovery`, `screenpipe-process-guide`, `screenpipe-interview-synthesis`,
  `screenpipe-writing-context`. Installation markers: `.screenpipe-starter-v1` (per skill), `.screenpipe-starter-next`,
  `.screenpipe-starters-installed-v1`, `.screenpipe-starters-index-next`; symlinks refused (S:0xa0c6708). Each is a short
  SKILL.md method; write Freenpipe's own text for each name.
- HTML artifacts guidance for agents: first line `<!-- screenpipe:render=human -->`, inline assets only.

**Base files.** `crates/screenpipe-core/assets/skills/`, `agents/pi.rs` (`ensure_screenpipe_skill*`). Confidence: names
observed.

### agents_pipes-025 - Skills registry browser and export to external agents (new + decloud)

- Tauri `fetch_skills_registry`, `install_registry_skill {id, gitRef}`, `install_external_agent_skills`,
  `remove_external_agent_skills`, `scan_device_skills`, `import_skill`, `list_imported_skills`, `remove_imported_skill`.
- Registry: curated JSON bundled in the app and refreshed from
  `https://raw.githubusercontent.com/screenpipe/screenpipe/main/apps/screenpipe-app-tauri/src-tauri/skills-registry.json`
  (S:0x4995d13); entries `RegistrySkill` (11 fields incl. `repo`, `path`, `git_ref`, `repo_url`, `homepage`, `featured`,
  `imported`) pointing at public GitHub skill folders (`anthropics/skills`, `openai/skills`). Install downloads the folder
  via GitHub API (`application/vnd.github+json`) with file-count and size limits, falls back to manual import.
- Export targets (S:0x52241b7, S:0x6a5b52d): Claude Code `~/.claude/skills`, Codex `~/.codex/skills`, Cursor
  `~/.cursor/skills`, Gemini `~/.gemini/skills`, OpenClaw `~/.openclaw/skills`, Hermes `~/.hermes/skills`.
- **Decloud:** do not fetch the registry from the upstream repo; ship a Freenpipe-authored registry file in the app and
  allow an optional user-configured URL. GitHub downloads are third-party and user-initiated (allowed).

**Base files.** `apps/.../src-tauri/src/skills.rs`. Confidence: observed.

### agents_pipes-026 - Workflows subsystem (new)

Engine routes (S:0x5257ca8 blob): `/workflows/rollout`, `/workflows/catalog`, `/workflows/workspace`,
`/workflows/pipeline`, `/workflows/context`, `/workflows/edits`, `/workflows/corrections`, `/workflows`, `/workflows/:id`.
Storage files (S:0x59fb4e8, S:0x5153f2f, S:0x6209a98, S:0x6208d70): `catalog.json` + `catalog.backup.json` + `catalog.lock`,
`pipeline.json` + `pipeline.backup.json` + `pipeline.lock`, `profile.json`, `workflows-model.json` (`{"mode":"intelligent"}`),
`.screenpipe-workflows.json` (agent side). Directory: a `workflows/` folder in the screenpipe data dir (inferred).

- **Workflow record** (`workflowOutputContract`, S:0x62092e8): `id, title, description, trigger, outcome, confidence,
  apps[], people[], teams[], handoffs[], variations[], openQuestions[], limitations[], timingRuns[{start{timestamp,app,quote},
  end{...}, summary}], captureSequence[{timestamp, app}], stages[{timingRuns, name, description, screenshotFrameIds[],
  confidence, apps[], procedure[{kind: action|input|output|decision|check, text, timestamp, app, quote}], openQuestions,
  evidence[{timestamp, app}]}], bottlenecks[{label, stage, type: waiting|switching|rework|handoff|unclear, control:
  direct|influence|external|required, controlReason, detail, confidence}], evidence[]`. Discovery output wrapper:
  `{evidenceVersion: 2, workflows: [...], timeCategories: [{label, description, rowIds}]}` (S:0x6208dc8). Catalog extras:
  `userEdits`, `userCorrection`, `lastReviewedAt`, `quality`, `observedDays`, `observedOccurrences`, `durationSource`,
  `evidenceStatus`, `catalogStatus` (S:0x52c7d50, S:0x52c7fb0).
- **Workspace** (`/workflows/workspace`, GET `?task=<pipe>`, POST action body): shared multi-agent drafting area for
  `workflow-discover|deepen|review|maintain`. State: `revision`, `catalogRevision`, `cycle {start, end, status
  running|finished|paused|complete, checkedThrough}`, `drafts{id: {id, status open|published|rejected, assignee,
  version, payload, history[{note}], publicationRetry}}`, `receipts`, `checkpoints`, `researchNotes`. Actions:
  `context` (read, optional `draft_id`/`workflow_id`), `start` and `propose` (discover; maintain may propose),
  `checkpoint` (<= 8000 chars), `handoff` (`assignee`), `reject`/`publish` (review only, needs `catalog_revision`),
  `finish`. Optimistic concurrency via `expected_revision`; a stale revision gets an error asking the agent to re-read context;
  capacity limit (an error when the workspace has no room); only the assigned agent can change an open draft; publication requires source verification and
  a payload matching the output contract; republishing with different payload rejected (S:0xa0bfca0..0xa0c0390,
  S:0x6209f2d..0x620a1f0). The engine emits `workflow_ready:<task>` events to wake the right task pipe (inferred).
- **Pipeline** (`/workflows/pipeline?task=`): enrichment stages for `workflow-activity`, `workflow-patterns`,
  `workflow-procedures` (+timing); each task can only read/save its own stage (`StageCommit {input_revision, coverage}`,
  `schemaVersion`, `inputRevision`, `checked_through`, coverage `[{start,end,complete}]`) (S:0x6209a10..0x6209ca8).
- **Context** (`GET /workflows/context`): saved catalog + user profile + corrections for agents; large snapshots are written
  to a file and the path returned (tool comment L698247).
- **Catalog** (`POST /workflows/catalog` from `workflow-discovery`): atomic commit with backup; size limit; previous
  catalog kept on failure (S:0x5151818..0x5151940).
- **Edits/corrections**: `POST /workflows/edits` `EditRequest {expected_revision, title, trigger, outcome, stages:[StageEdit
  {sourceIndex, description, procedure:[ProcedureEdit {kind, text}]}]}` - desktop owner only (S:0x5151638..0x5151720);
  `POST /workflows/corrections` free text <= 20000 bytes (S:0x62097b0).
- **List/get**: `GET /workflows?q&limit&offset`, `GET /workflows/:id?include_automation=true` (adds automation evidence:
  captured accessibility nodes with match distance/truncation and a caution that historical selectors are not live)
  (S:0x5153b08..0x5153e35). Requires unfiltered-history permission (S:0x51539e0).
- Model choice: `workflows-model.json` modes (`intelligent`, preset choice); invalid/unsupported preset errors
  (S:0xa0bb130).
- UI: Workflows section in home and `dev/workflows-ux` dev harness (F:_next/static/chunks/app/dev/workflows-ux/page-c4e3cdf58b630690.js)
  which installs bundled workflow tasks via `/pipes/bundled/:id/install` and toggles them.

**Base files.** new `crates/screenpipe-core/src/workflows/*`, `crates/screenpipe-engine/src/routes/workflow_*.rs`, server
routes, built-in pipes (-012), MCP tools `list-workflows`/`get-workflow` (-030). Freenpipe can stage this: storage + API
first, agents second. Confidence: identifiers observed; storage directory inferred.

### agents_pipes-027 - Workflows cloud gating (decloud)

Target gates workflows on the cloud account: `workflow_rollout_disabled`, `workflow_business_required` (Business tier),
`workflow_allowance_paused` / `cost_limit_reached` / `background_pipe_advisory` (AI allowance via `/hosted_ai/plan` and
`/usage`), `workflow_sign_in_required`, `workflow_usage_unavailable`, `acp_model_access` (S:0xa0bf443, S:0xa074b28,
S:0xa0b8dac). `/workflows/rollout` is desktop-writable access state. **Local replacement:** rollout always enabled
(store a local boolean in settings), no tier/allowance checks; optional local budget (max runs/day) in settings; never
call `/hosted_ai/*` or `/usage`.

### agents_pipes-028 - Cloud coding agents (new; third-party BYO)

- Pipe `agent: cloud-agent` with `cloud_agent: CloudAgentConfig {provider: cursor|codex|claude, environment_id, branch,
  session_id, agent_id, repository, starting_ref, send_screenpipe_context, context_lookback_hours, context_max_items}`
  (S:0x52ca560, S:0xa049fa0). Providers: **Cursor Cloud** (`https://api.cursor.com` `/v1/agents`, API key stored in the
  encrypted secret store as `cloud_agents.cursor.api_key` / env `CURSOR_API_KEY`; returns agent URL `https://cursor.com/agents/<id>`),
  **Codex Cloud** (local `codex` CLI cloud tasks; requires environment selection), **Claude Cloud** (`claude --cloud` in an
  isolated git snapshot repo committed with a fixed screenpipe-branded message) (S:0xa049628..0xa049e80).
- Routes: `GET /cloud-agents/status` -> `{providers}`, `POST /cloud-agents/:provider/connect`,
  `GET /cloud-agents/cursor-agents` -> `{agents:[{id,name,status,url}]}`, `GET /cloud-agents/:provider/codebases` ->
  `{codebases:[{value,label}]}`, `PUT /cloud-agents/cursor-key` (`CursorCloudKeyRequest`) (dev mock shapes).
- Optional context capsule (`agents/cloud_context.rs`): when `send_screenpipe_context` is true, a bounded activity capsule
  (activity summary, key texts, memories, snippets; truncated marker) is appended to the task; otherwise a note that no
  context was shared. Lookback/max items from config.
- **Privacy:** this sends user data to third-party clouds; Freenpipe must keep it opt-in per pipe (default false), show
  what is sent, and never route through screenpipe servers.

**Base files.** new `crates/screenpipe-core/src/agents/cloud.rs`, `cloud_context.rs`, engine `routes/cloud_agents.rs`,
secret store keys. Confidence: observed.

### agents_pipes-029 - Provider automations viewer (new)

App `provider_automations.rs` + Tauri `list_provider_automations`, `manage_provider_automation` (S:0x5a64cd4..0x5a65045).
Lists scheduled jobs owned by external agents: Claude (`.claude/scheduled_tasks.json`, session-only vs durable,
recurring jobs expiring after 7 days) and Codex (`~/.codex/automations/*/automation.toml`, `CodexAutomationManifest` with
`rrule`, `updated_at`). Item `ProviderAutomation {key, provider, nativeId, schedule, scheduleLabel, executionScope,
manageability, availableActions, lifecycleNote, revision, updatedAtMs, ...}` (13 fields, S:0x52bb228). Session-scoped jobs
can be managed through a live ACP control session (`control-sessions`, `agentId`, `acpSessionId`, `chatSessionId`,
`processId`); provider-owned ones are read-only (the user is told to edit them in the owning tool). Confidence: observed identifiers; actions
inferred.

### agents_pipes-030 - Built-in MCP server v0.20 surface (changed)

- Tool names known to the engine (S:0x6a9a7c0): `search-content`, `synced-devices`, `search-synced-content`,
  `list-meetings`, `activity-summary`, `search-elements`, `frame-context`, `export-video`, `update-memory`, `get-feedback`,
  `send-notification`, `health-check`, `list-audio-devices`, `list-monitors`, `add-tags`, `search-speakers`,
  `list-unnamed-speakers`, `update-speaker`, `merge-speakers`, `start-meeting`, `stop-meeting`, `get-meeting`,
  `update-meeting`, `keyword-search`, `get-frame-elements`, `control-recording`, `list-pipes`, `create-pipe`, `run-pipe`,
  `pipe-logs`, `team-search`, `team-devices`, `team-records`, `team-frame`, `list-workflows`, `get-workflow`,
  `screenpipe-skills`. **New vs base 0.18.10:** `synced-devices`, `search-synced-content`, `get-feedback`, `list-pipes`,
  `create-pipe`, `run-pipe`, `pipe-logs`, `team-frame`, `list-workflows`, `get-workflow`, `screenpipe-skills`
  (skills exposed as MCP prompts/resources, inferred). `search-content` supports `content_type: memory` (memory recall).
- **Portable MCP launcher.** The app executable accepts `--screenpipe-portable-mcp <mode>`; it validates the installed MCP
  package root and entrypoint (must not escape the package), then runs it with the bundled bun (S:0xa1170fe). HTTP mode
  env: `SCREENPIPE_MCP_TRANSPORT=http`, `SCREENPIPE_MCP_HOST=127.0.0.1`, `SCREENPIPE_MCP_PORT`, `SCREENPIPE_MCP_PATH=/mcp`
  (S:0xa117260). Client identity via `SCREENPIPE_MCP_CLIENT`. External clients otherwise run `screenpipe-mcp@latest` via npx.
- Memory sync to external agents writes MCP recall guidance into `CLAUDE.md` / `AGENTS.md` sidecars between
  `<!-- screenpipe-memories:start v... -->` / `:end -->` markers (S:0xa0ba9e8..0xa0bae71).

**Base files.** `packages/screenpipe-mcp/src/index.ts` (add tools), `http-server.ts`, app main arg parsing for
`--screenpipe-portable-mcp`. Version bump to 0.20.x. Confidence: tool names observed; schemas to be designed from engine
routes.

### agents_pipes-031 - MCP + skills setup into external agents (new)

Engine `cli/agent.rs` (S:0x6a5af4c..0x6a5b52d; L158664 `npx -y screenpipe@latest agent setup`): `screenpipe agent setup
--target <t>` (targets `openclaw`, `hermes`, `claude-code`, `claude-desktop`, `codex`, `cursor`, `gemini`, `runner`,
`windsurf`, `vscode`). For each target it writes an MCP server entry named `screenpipe` (command/args/env/transport) into
the agent's config and installs skills: Claude Desktop `%APPDATA%/Claude/claude_desktop_config.json` (`mcpServers`),
Claude Code `.claude.json`, Codex `.codex/config.toml` (`[mcp_servers.screenpipe]`, `[mcp_servers.screenpipe.env]`), Cursor
`.cursor/mcp.json`, Gemini `.gemini/settings.json`, OpenClaw `.openclaw/openclaw.json` (JSON5, `mcp.servers`), Hermes
`.hermes/config.yaml` (`mcp_servers:`), Runner `.runner/mcp.json`, Windsurf `.codeium/windsurf/mcp_config.json`, VS Code
`AppData/Roaming/Code/User/mcp.json`. Backups `.screenpipe-backup-*`, lock `.screenpipe.lock`; refuses to touch
non-standard blocks (prints manual snippet). Env passed: `SCREENPIPE_API_URL`, `SCREENPIPE_LOCAL_API_KEY`. Tauri
`resolve_ai_tool_config_path`, `set_ai_tool_auto_connect_opt_out`. A "Grok Bot" variant installs skills through that
assistant's API (`grokbot_connection`, L173729..173849). **Decloud:** point `command` at the local Freenpipe binary
(portable MCP) instead of `npx screenpipe@latest`. Confidence: observed.

### agents_pipes-032 - BYO MCP servers additions (changed + decloud)

- `GET /mcp-servers/registry?search&cursor&limit=30` -> `{data}` proxying the public MCP registry
  `https://registry.modelcontextprotocol.io/v0/servers` (`servers`, `nextCursor`) (S:0x618835f region; F:5977 @112511).
  Third-party, user-initiated: keep, with offline fallback.
- Existing routes unchanged: `/test`, `/:id/test`, `/:id/tools`, `/:id/call` (`CallBody {tool, arguments}`),
  `/:id/oauth/{status,start,callback,disconnect}`; `ProbeBody {transport, url, headers, command, env}`; `OAuthStartBody`
  gains `app_scheme` (`screenpipe` or `screenpipe-enterprise`).
- Session scoping header `x-screenpipe-session` (-013). Bridges hide servers that point back at the local engine.
- Well-known MCP-OAuth connections (Linear, Stripe, Sentry, Intercom, Asana, Monday, ClickUp, Airtable, Confluence, Jira,
  Notion - vendor MCP URLs, S:0x6a33590) surface as `mcp=true` connections that agents must use via `sp_mcp_*`.
- **Decloud:** MCP OAuth redirect via `https://screenpi.pe/api/oauth/mcp/` (S:0x6a8dd01) -> use loopback
  `http://localhost:<port>/mcp-servers/:id/oauth/callback` (already exempt from auth in base). Composio managed auth
  (`https://screenpipe.com/api/composio/mcp`, `/api/composio/status`) -> not available; users add the vendor's MCP server
  directly.

### agents_pipes-033 - Local OpenAI-compatible LLM proxy (new)

`POST /v1/local/chat/completions` (S:0x5259090; engine `local_chat.rs`, S:0x5253b30). Forwards an OpenAI-style chat
completion (streaming passthrough with `no-cache`) to the user's default AI preset from settings (`aiPresets`), using
the preset's base URL/key; errors when no preset is configured or the preset lacks a base URL, or when the provider is
unreachable. Lets agents and pipes call "the user's model" without seeing keys. **Freenpipe:** also make this the target
for the decloud of -034. Base files: new `engine/src/local_chat.rs`, preset resolution shared with `pipes/mod.rs::resolve_preset`.

### agents_pipes-034 - Cloud chat proxy `/v1/chat/completions` (decloud)

Base and target forward to `https://api.screenpipe.com/v1/chat/completions` with the cloud JWT (error
`cloud_token_missing`, S:0x518379f). **Local replacement:** keep the route (agents call it for media analysis) but serve it
from the -033 logic (user preset; vision-capable model required for images); return a clear error if none.

### agents_pipes-035 - Web search `/v1/web-search` and `sp_web_search` (decloud)

Target: engine proxy `POST /v1/web-search` -> `https://api.screenpipe.com/v1/web-search` (S:0x6188ef3 neighbourhood,
errors `cloud_token_missing`, `upstream_unreachable`, `upstream_timeout`); Pi tools call it directly with the cloud key or
via the local proxy (ACP). Tool result `{text, sources:[{title,url}], query}`. **Local replacement:** `sp_web_search`
backed by a user-configured provider (e.g. SearXNG URL or a BYO API key) via the same local route; disabled when
unconfigured.

### agents_pipes-036 - Screenpipe-cloud LLM provider, gateway models, confidential GLM (decloud)

- `screenpipe-cloud` provider uses `https://api.screenpipe.com/v1` gateway; model list fetched from `/v1/models` with a
  fallback list (S:0xa0b759a); default hosted model `glm-5.3-flash-reap50-iq3m`; tier errors and a notice that the pipe's model was
  not available on the current tier so a substitute model ran (S:0xa0b9f11); hosted capacity/allowance statuses.
- Confidential provider: Pi extension registers provider `screenpipe` with API `screenpipe-tinfoil`, encrypting requests to
  a Tinfoil enclave (`https://pii.screenpipe.containers.tinfoil.dev`, config repo `screenpipe/privacy-filter`) and
  emitting attestation status (L694636..694720).
- Other URLs: `https://ai.ai-genesis.app`, `https://api.ai-genesis.app` (S:0xa0b88e0).
- **Local replacement:** hide/omit `screenpipe-cloud`; built-ins' `preset: [screenpipe-cloud, "*"]` resolves to the user's
  presets via `"*"`; model lists come from configured presets (Ollama `http://localhost:11434/v1`, OpenAI, Anthropic,
  custom OpenAI-compatible, ChatGPT OAuth). No tinfoil SDK.

### agents_pipes-037 - Pipe store registry (decloud)

`/pipes/store*` proxies `${SCREENPIPE_API_BASE_URL:-https://screenpi.pe}/api/pipes/store[/<slug>[/install|/review]|/publish]`
(S:0x5182e2a..0x5183409, S:0x6a92f80); error codes `registry_http_error`, `registry_network`, `registry_timeout`,
`pipe_not_found`, `invalid_registry_response`; auto-update reports `skipped_modified`; updates back up `pipe.md.bak`.
**Local replacement:** keep route shapes; default registry = a Freenpipe-controlled static index (Git repo of pipe.md
files) or none; publish/review disabled unless a user-configured registry supports them.

### agents_pipes-038 - MCP/agent telemetry (decloud)

Routes `/internal/telemetry/mcp-value`, `/internal/telemetry/mcp-call` (S:0x52585a8, S:0x52585e0) and engine `mcp_call.rs`
(`McpCall` with agent identity from `X-Screenpipe-Agent`: claude, chatgpt, codex, cursor, gemini, openclaw, hermes, runner,
windsurf, vscode, grok, grokbot, unknown). **Local replacement:** accept and discard (204) or log locally only; never
forward.

### agents_pipes-039 - ACP feature flags and cloud routing (decloud)

ACP agents are gated by flags `acp_agent_cursor`, `acp_agent_github_copilot`, `acp_agent_custom`; `claude-acp` can route
through screenpipe cloud using `ANTHROPIC_BASE_URL=<cloud>/anthropic` + `ANTHROPIC_AUTH_TOKEN` (`useScreenpipeCloud`).
**Local replacement:** flags are local settings (default on for custom/cursor/copilot); `useScreenpipeCloud` unsupported -
the agent uses its own login or a user key.

### agents_pipes-040 - Agent CLI resolution via npm (decloud)

`agents/cli_runtime.rs` resolves `screenpipe@latest` through `bun install` into a cache (`sp-cli-resolve`, 180 s timeout,
`@screenpipe/cli-win32-x64`, `receipt.json`) so agents can run the `screenpipe` CLI (S:0xa0c6840..0xa0c6d4a).
**Local replacement:** expose the bundled Freenpipe engine binary path via `SCREENPIPE_CLI` and PATH; no npm resolution.

### agents_pipes-041 - Enterprise-managed agents/pipes (ee-stub)

Organization-managed pipes (cannot edit/enable/delete locally), managed team skills (`ManagedTeamSkill`,
`sync_managed_team_skills`, package risk metadata), injection of enterprise team skills into native Pi sessions,
`allow_employee_custom_presets`, `enterpriseManaged` presets. Upstream ee/ is excluded: implement only stubs that report
nothing as managed and keep the hooks.

### agents_pipes-042 - Pipe run trajectories (new)

`pipes/trajectory.rs` (S:0xa0ec646..0xa0eca35): optional JSONL recording of agent events per run (`jsonl_delta`), file name
`%Y-%m-%dT%H-%M-%S%.3fZ.jsonl` in `SCREENPIPE_TRAJECTORIES_DIR` (default a `trajectories` folder), retaining
`SCREENPIPE_TRAJECTORIES_MAX_RUNS` runs. Off unless the env var is set (inferred). Useful for eval/debug.

### agents_pipes-043 - Connect broker and connection-gate tools (new)

Engine `routes/connect_broker.rs` (S:0x5255960): `POST /v1/connect-request` `ConnectRequestBody {connection_id,
required_for, session_id}` -> desktop raises a connect card in the originating chat (event `acp_connect_request`);
`POST /v1/connect-response` `ConnectResponseBody {request_id, status: connected|declined}` (event `acp_connect_response`).
Tools: `screenpipe_list_connections` (connected apps incl. MCP-OAuth ones with `mcp_server_id`) and
`screenpipe_connect_app` (requests the user to connect an app, then continues). Native connections overlapping a managed
one keep their proxy path. Base files: new route, app chat UI card, extension asset.

### agents_pipes-044 - Unified feedback store (new; agent-facing)

DDL (migration `create feedback`, S:0xa2aaf2a) - verbatim:

```sql
CREATE TABLE IF NOT EXISTS feedback (
    id TEXT PRIMARY KEY,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    target_version TEXT NOT NULL DEFAULT '',
    producer_ref TEXT,
    actor_id TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    comment TEXT,
    snapshot TEXT,
    context TEXT NOT NULL DEFAULT '{}',
    device_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (target_kind, target_id, target_version, actor_id, device_id)
);
CREATE INDEX IF NOT EXISTS idx_feedback_target
    ON feedback (target_kind, target_id, target_version);
CREATE INDEX IF NOT EXISTS idx_feedback_producer_updated
    ON feedback (producer_ref, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_feedback_rating_updated
    ON feedback (rating, updated_at DESC);
```

Route `/feedback` (S:0x5258728; engine `routes/ai_feedback.rs` S:0x5183423): set via `SetFeedbackRequest`; list filters
`target_kind, target_id, producer, since, since_inclusive, after_id, order`. Rules: target kind lowercase ASCII/digits/`._-`;
a comment requires a `down` rating and is length-capped; actor `local-user`; pipes cannot rate their own outputs. Pipes
read it (`GET /feedback` in reader allowlist, MCP `get-feedback`). Coordinate with any feedback/ratings dimension.

### agents_pipes-045 - Pipe count limit by tier (decloud)

Target unloads over-limit pipes beyond an account tier quota and stops running ones (S:0x517ab67, S:0x61872b0).
**Local replacement:** no limit.

### agents_pipes-046 - Workflow-to-skill drafting (new)

Tauri `generate_workflow_skill`, `save_workflow_skill` (S:0x49f2770, S:0x5222a9c..0x5223dde). From a reviewed workflow with
at least two mapped stages, an LLM drafts `{name (lowercase-hyphenated), description, instructions}` JSON treating the
workflow as untrusted evidence and incorporating `userCorrection`; the result is saved as a local skill (with ownership
marker) and can be exported to external agents. The skill body links back to `get-workflow` / `GET /workflows/<id>?include_automation=true`.
Target uses the hosted model; **Freenpipe:** use the user's preset via -033. Confidence: observed.

---

## 5. Cloud dependency summary and local replacements

| Dependency | Where | Replacement |
|---|---|---|
| `https://api.screenpipe.com/v1` chat/completions, models, web-search | -034, -035, -036 | user presets via `/v1/local/chat/completions`; BYO search |
| Tinfoil confidential GLM (`pii.screenpipe.containers.tinfoil.dev`) | -036 | none (remove) |
| `https://screenpi.pe/api/pipes/store` | -037 | static Freenpipe/Git index or disabled |
| `https://screenpi.pe/api/oauth/mcp/`, Composio `screenpipe.com/api/composio/*` | -032 | loopback OAuth; no Composio |
| `/internal/telemetry/mcp-*` | -038 | no-op |
| Workflows rollout/Business/allowance (`/hosted_ai/plan`, `/usage`) | -027 | local toggle, no gating |
| ACP flags + `/anthropic` cloud routing | -039 | local flags, own agent auth |
| `screenpipe@latest` / `screenpipe-mcp@latest` via npm | -040, -031 | bundled binary / portable MCP |
| Skills registry from raw.githubusercontent (upstream main) | -025 | bundled Freenpipe registry |
| Tier pipe limits, model tier fallback | -045, -036 | none |
| Third-party (keep, opt-in): Cursor/Codex/Claude clouds, GitHub skill downloads, MCP registry, baseline bun / PortableGit downloads, ACP agent npm packages | -028, -025, -032, -015, -016 | keep user-initiated, document |

## 6. Open questions

- Exact `pipe_event_runs` prune age, deferred-trigger backlog cap, connection-trigger retry count, pipe-stream cap (not in strings).
- `/agent/learning/chats` exact response fields and corpus (screenpipe chats only vs. imported external chats).
- Whether `workflow_ready:<task>` is emitted by the engine on workspace changes or by a timer.
- `screenpipe-skills` MCP item type (tool vs prompt/resource) and input schemas for new MCP tools.
- Storage directory name for workflows files (`catalog.json`, `pipeline.json`, `profile.json`).

---

## Verification

Adversarial pass, 2026-10-14. Method: (a) grepped the BASE tree (`<repo>`, clean at `0e2397a` over the
`892199f` import, so no parallel-agent additions) with `tools/ver_basegrep.py` (apps, crates, packages; ids list in
`work/ver_ids1.txt`, output `work/ver_base1.out`), then read the specific base files named below; (b) re-read every cited
`S:` offset with `tools/verify_offs.py` and multi-line SQL with the new `tools/ver_range.py`, and re-read the `F:` chunk
positions; (c) checked for duplicates inside this list and against other dimension specs. Tauri command diff:
`tools/tauri_cmds.py` over the bindings chunk `8001-00ed120106af2449.js` vs base `lib/utils/tauri.ts`
(`work/ver_tauri_diff.txt`). REA: a literal-seed trace for `/pipes/schedule/preview` on parent
`ev_57f23857...216d` returned no-match on the partial graph (`ev_984310d55e313a2feddd73a21da5acba0404cc3187632f2cbd7c0eaea0d08dda`),
so frontend facts rest on direct chunk reads, as in section 0.

Result: 31 confirmed, 14 partially in base, 1 already in base, 0 unsupported, 0 duplicate (several overlap other
dimensions; see the last column).

| Item | Verdict | Note |
|---|---|---|
| -001 | partially_in_base | Base `engine/src/pipe_store.rs` already has the orphan recovery that marks `running` **and** `queued` failed with `error_type='interrupted'` (SQL identical to S:0x6a010e0), the output-less list variant (`'' AS stdout`, :254), julianday `duration_ms` finish, `COALESCE(?, pid)`/`COALESCE(started_at, ?)`, ROW_NUMBER retention (`db/write_queue.rs:1444`), and the `timed_out`/`timeout` types (`pipes/mod.rs:2160`). Genuinely new: the 3 ALTERs, the insert carrying `trigger_event, trigger_key` (S:0xa2b6bb8), the running guard `AND status IN ('queued','running') AND finished_at IS NULL` (S:0x6a0129b), keyset `id < ?` paging (S:0x6a01864), the activity aggregate (S:0x6a01368), `SET conversation_id` (S:0x6a02058), the finaliser, and `summary_not_saved`/`ai_preset_unavailable`/`nothing_to_summarize`. DDL overlaps db-009. |
| -002 | confirmed | DDL S:0xa2aa964/0xa2aaa65, claim/prune S:0xa2b6e03/0xa2b6ec5, duplicate-suppression log S:0x499d7be, release-failure logs S:0x499d946/0x499dd80. The base has no table and drops events for a busy pipe (`pipes/mod.rs:3357-3367`). The DDL is also recorded as db-009, so keep DDL ownership in db and the claim behaviour here. |
| -003 | confirmed | Serde lists verified (S:0x61874e0, S:0x6187698, S:0xa11740c); envelope rejection S:0x517af0e. Base `PipeConfig`/`PipeStatus` (`pipes/mod.rs:70,585`) lack every listed field, and base `ConfigUpdateRequest` flattens any body without validation. Caveat: `history` already works in base as an untyped extra (`mod.rs:1930` reads it and drives Pi `--continue`); only its typing is new. |
| -004 | confirmed | `ScheduleConfig` S:0x6187760; route S:0x5258c98; UI defaults at F:5977 @474329 and the debounced 350 ms POST at @479463 verified verbatim. Nothing in base. |
| -005 | confirmed | Routes S:0x5258c20/0x5258e38, query flags S:0x5185300, `RunPipeBody` S:0x51853b0, untrackable-run error S:0x5184e60, bundled-load error S:0x5185450, history-while-running refusal S:0x517b14c. Base `pipes_api.rs`: list returns `{data,total}`, `get_pipe` returns 200 with an error body, executions take only `limit`, `RunPipeBody` has only `notification_context`, `clear_pipe_history` never refuses. |
| -006 | confirmed | Route S:0x5259048, `progress`/`keep-alive` S:0x6187ea0, cap message S:0x6187ed8. Caveat: the `{"type":"pipe_done"}` terminator is already emitted by the base scheduler (`pipes/mod.rs:2031,2524,3704`) and consumed by the base app (`standalone-chat.tsx:5720`). Only the SSE route, frames and cap are new. |
| -007 | partially_in_base | Base already clears the Pi session on context overflow (`mod.rs:2196,3886`), archives logs as `%Y%m%d_%H%M%S` (`:3880`), falls back to the user's default preset when none is configured (`:3511`), runs old-execution cleanup (`:3963`), auto-disables one-offs and has the 60 s chain guard. New: topics `meeting_summary_refresh_requested`/`connection_trigger`/`workflow_ready:*` (S:0x499d460, S:0x6a7a9c1), defer/coalesce/backlog cap (S:0x499dbe0..0x499dc0b, S:0x499d5a9; base skips busy events), `.trigger-context.json` (S:0xa10e2b3), trivial-meeting skip (S:0x499d4ed), `"*"`, and refusal to fall back when a named preset is missing (S:0x499d8ed; base silently uses `config.model/provider`, `:3502`). |
| -008 | confirmed | S:0xa0e5de8 (watcher start), S:0xa0e5ff8 (app list), S:0xa0e615e (Linear GraphQL), S:0xa0e68f8 (voice SQL). None of these identifiers is in base. |
| -009 | partially_in_base | Base already resumes the Pi session when `history: true` (`mod.rs:1930`, `agents/pi.rs:1126` `--continue`), and the base prompt already names `Output directory: ./output/` (`mod.rs:4231`). New: `run_in`/`existing_chat` with streaming-runtime requirement (S:0xa043cc8, S:0xa04942f), `conversation_id` persistence, `.screenpipe-acp-sessions.json` (S:0x517b108), and the scheduled-task directory line in the preamble (S:0xa049378). Missed detail: per-device `.local-run-destinations.json` (see m02). |
| -010 | confirmed | DDL S:0xa2a1af2/0xa2a2d2a/0xa2aad72, `register-artifact.ts` S:0xa0ac9d0, save-artifact install log S:0x52a0afd. Nothing in base. The `outputs` DDL is also db-002 (and meetings_memory.md); keep this item to the agent tools and `/artifacts` routes. |
| -011 | confirmed | S:0x522d17b (`SubmitStructuredOutputRequest`), S:0x522d360 (`structured-outputs`/`locks`), S:0x5258618 (route), S:0xa0b0049 (install log). Nothing in base. |
| -012 | confirmed | Frontmatter re-read for `skill-learning` (S:0xa0fc668: every 6h, claude-haiku-4-5, timeout 180, artifacts) and the `workflow_ready:*` triggers (S:0xa054593..). `ai-prompt-journal` has 0 hits in the target strings. Base bundles 7 pipes. Caveats: `automate-my-work` existed in base only as a frontend fallback template (`lib/summary-templates.ts:22`), not a bundled pipe.md, and base `meeting-summary` already had `preset: [screenpipe-cloud]` plus `meeting_ended` (target adds `"*"`). |
| -013 | partially_in_base | Base already removes skills denied by pipe permissions with the same log text (`agents/pi.rs:582`), has the route-not-permitted middleware message (`pipe_permissions_middleware.rs:88`) and the recording-priority concurrent-query backpressure (`:156`), and already uses `x-screenpipe-session` (for owned-browser routing, `agents/bash_env.rs`). New: the allowlist entries (S:0xa0c8030/0xa0c8088 vs base `permissions.rs:126-139`), the `mcp_access.rs` gate (S:0x499d1d8) with `SCREENPIPE_MCP_SERVER_ALLOWLIST` (S:0x5295a15), and workflow-operation limits (S:0x6188d38/0x6188e28). |
| -014 | confirmed | Install logs verified (S:0x517b6f0, S:0x52a0afd, S:0x52a9dec, S:0xa0a26ba, S:0xa0a8aa4). Base has only 6 core + 2 app extensions. Caveats: the filesystem sandbox (`screenpipe-permissions.ts:222`) and 30K pruning (`context-pruning.ts:38`) rows describe base behaviour. Base tool names `mcp_list_tools`/`mcp_call`/`web_search` were renamed `sp_mcp_*`/`sp_web_search`, a rename to record. This is an umbrella item: its tool ids overlap -010/-011/-019/-020/-022/-023/-043, and its unique content is the package commands, `sp_mcp_read`, `workflow_workspace`, video-sop, `fill_work_context`, `refine_workflow` and `pi-subagents`. |
| -015 | partially_in_base | Base `agents/pi.rs` already merges providers into `models.json`, skips ones without baseUrl and rebuilds a corrupt file (:721-898), has `anthropic-byok`/`openai-codex-responses` (:802,:819), re-seeds an unreadable pi-agent `package.json` with `overrides` (:1870-1928), and pins PortableGit 2.47.1 (:2276). New: the 0.84.1 pins as deps+overrides (S:0xa0b5c00; base 0.75.4), isolated pi-config + `.migrated-from-global` (S:0xa0b4900), baseline bun 1.3.10 (S:0xa0b6228), the bash probe (S:0xa0b7148), the compaction patch and the extra env vars. |
| -016 | confirmed | Catalog in F:7451 (@9112 `pi-acp@0.0.32`, @9622 cloudRouting, @10437 cursor flag); env S:0xa0bef96/0xa0bf167; process guard S:0xa0cab68; `SCREENPIPE_TOOLS_HTTP_PORT` S:0xa0d95a5; crate `agent-client-protocol-1.2.0` in crates.txt. 0 base hits. The Tauri diff also shows `pi_acp_set_config_option` and `pi_acp_set_approval_mode`, which the command list omits (frontend.md has them). |
| -017 | partially_in_base | Base already flags managed presets with `enterpriseManaged` (`lib/hooks/use-enterprise-policy.ts:73`, `lib/enterprise-ai-preset-policy.ts:133`), defaults managed presets to `maxContextChars` 512000 (:71), and has `system_prompt` in `PiProviderConfig` (`src-tauri/src/pi.rs:1030`). New: provider `acp`, 9-field `acpAgent` (S:0x52c8150), `backend`/`allowedTools`/`resumeSessionId`/`unattended` (S:0x52c8680), and per-chat `presetId`. |
| -018 | partially_in_base | Base already has archived (hidden) chats (`lib/chat-storage.ts:54`), `branchConversation` (`components/hooks/use-chat-conversations.ts:1258`), the pool-full error (`src-tauri/src/pi.rs:1495`, i.e. the cited S:0x4991251), and most agent_event types (`thinking_*`, `agent_end`, `pipe_done`). New, per the bindings diff: `pi_start_and_prompt`, `pi_stop_if_idle`, `pi_request_state`, `pi_get/set_thinking_level`, `pi_extension_ui_response`, `list_chat_entries_by_mtime`; start stages; `thinking_level_changed`/`extension_ui_request`/`compaction_end`; `importedFrom` and `presetId`. |
| -019 | confirmed | F:home @601799 roots `.claude/projects` and `.codex/sessions` verified; F:378 @42049 `importedFrom` validation; broker strings S:0x499e5a0, `chat-control.ts` S:0xa0a0418. 0 base hits. |
| -020 | confirmed | S:0xa04a0e0 (git env hardening), S:0xa04aaf8 (owner record fields), S:0xa0a105f (route prefix). 0 base hits. Overlaps frontend.md `coding_workspace_*` commands. |
| -021 | confirmed | Route S:0x5258510, provenance message S:0x6a0f32f, unsafe-pattern regex ids at L632803 (`0x6a0f519`), tool params S:0xa0a759e. 0 base hits. |
| -022 | confirmed | Routes S:0x5258540/0x5258570, BLOCKED placeholder S:0x6a43757, `self-improvement.ts` S:0xa0a6ce0. 0 base hits. |
| -023 | confirmed | Route S:0x52584e0, config file S:0x499e1f1, extension S:0xa0a8acf, pipe S:0xa0fc668. 0 base hits. |
| -024 | partially_in_base | Base `agents/pi.rs:376-396` already deletes exactly the same 9 deprecated skill dirs. New: `screenpipe-chats`, `screenpipe-workflow-maintenance`, `video-sop`, the 20 starter skills and their markers (S:0xa0c17d8, S:0xa0c6708). |
| -025 | confirmed | Registry URL S:0x4995d13, bundled catalog S:0x4995df0, export targets S:0x52241b7, `RegistrySkill` S:0x52c96e0. Base `skills.rs` only scans and imports from `~/.claude/skills` (:122); it has no registry and no export. |
| -026 | confirmed | `workflowOutputContract` S:0x62092e8, discovery wrapper S:0x6208dc8, `pipeline.json` S:0x6209a98, workspace S:0x6209eb0. Base has only `workflow_event` classification. |
| -027 | confirmed | Gate strings verified at S:0xa074b28 and further into the S:0xa0bf443 line (+340: `/hosted_ai/plan`, `workflow_usage_unavailable`, `workflow_business_required`, `cost_limit_reached`, `background_pipe_advisory`). The first 300 chars of that line are ACP runtime errors, which is why it looks mis-cited. Nothing in base. |
| -028 | confirmed | S:0x52ca560/0xa049fa0 (`CloudAgentConfig`), S:0xa049628, S:0x522a0b8 (`https://api.cursor.com`). 0 base hits. |
| -029 | confirmed | S:0x5a64cd4 (Claude job scopes), S:0x5a65045 (`CodexAutomationManifest`), S:0x52bb228 (`ProviderAutomation`, 13 fields). 0 base hits. |
| -030 | partially_in_base | Base already syncs memory sidecars into `CLAUDE.md`/`AGENTS.md` between the same `<!-- screenpipe-memories:start v... -->`/`:end -->` markers (`crates/screenpipe-core/src/memories/external_sync.rs:40-91`), and base `search-content` already accepts `content_type: memory` (`packages/screenpipe-mcp/src/index.ts:279`). The target changes the sidecar body to static MCP recall guidance (S:0xa0babeb, S:0xa0bae71). New: 11 tools (base 0.18.10 has 26 tools, none of the 11), `--screenpipe-portable-mcp` (S:0xa0ba9e8, S:0xa1170fe), and `SCREENPIPE_MCP_*` env (S:0xa117260; base `http-server.ts` takes only `--port`). |
| -031 | partially_in_base | Base onboarding already writes the `screenpipe` MCP entry for Claude Desktop (`claude_desktop_config.json` `mcpServers`), Cursor (`.cursor/mcp.json`) and Codex (`[mcp_servers.screenpipe]` in `~/.codex/config.toml`) (`components/onboarding/connect-apps.tsx:132-213`), and has Hermes/OpenClaw cards with skill paths (`components/settings/hermes-card.tsx`, `components/settings/openclaw-card.tsx`). The base CLI has only `mcp setup`. New: engine `agent setup --target` with 10 targets, backups/locks, Rust `resolve_ai_tool_config_path`/`set_ai_tool_auto_connect_opt_out`, Gemini/Windsurf/VS Code/Runner configs, grokbot. |
| -032 | confirmed | F:5977 @112511 (`/mcp-servers/registry?...limit=30`), `app_scheme` + `https://screenpi.pe/api/oauth/mcp/` S:0x6a8dd01, vendor list S:0x6a33590. The base OAuth flow takes `redirect_uri` from the caller (`crates/screenpipe-connect/src/mcp_servers.rs:485`) with no screenpi.pe relay; it has no registry proxy and no vendor list. |
| -033 | confirmed | Route S:0x5259090, module S:0x5253b30. 0 base hits. |
| -034 | already_in_base | The route, the forwarding to `https://api.screenpipe.com` with the cloud JWT, and the exact `cloud_token_missing` (media-analysis wording) / `upstream_unreachable` bodies are identical in base (`engine/src/routes/cloud_proxy.rs:23-53`, registered at `server.rs:791`). So this is not a v2.5.27 to v2.7.104 delta. The decloud is still required but applies to base code (owned by api-024 / cloud-016); fold the local-routing behaviour into -033. |
| -035 | partially_in_base | Base already ships `web-search.ts` extensions (core and app) calling `https://api.screenpipe.com/v1/web-search` directly with the cloud token (`assets/extensions/web-search.ts:47`, tool `web_search`). New: the engine proxy route `/v1/web-search` (S:0x5258ae8, S:0x6188ef3), the rename to `sp_web_search`, and the local ACP web-search extension (S:0x52aaea1). The decloud covers both base and target paths; overlaps api-025. |
| -036 | partially_in_base | Base already has the `screenpipe-cloud` provider, gateway `/v1/models` fetches (`components/settings/ai-presets.tsx:634..`) and GLM models (`lib/hooks/use-settings.tsx:852`). Base `tinfoil` is only the PII-redaction backend (`src-tauri/src/server_core.rs:624`). New: the `screenpipe-tinfoil` Pi LLM provider (S:0xa084025), the tier fallback to a substitute model (S:0xa0b9f11), the ai-genesis URLs (S:0xa0b88e0) and the hosted capacity statuses. |
| -037 | partially_in_base | Base `engine/src/routes/pipe_store.rs` already proxies to `SCREENPIPE_API_BASE_URL` (default `https://screenpi.pe`, :30) including `skipped_modified` auto-update (:387-488). New: only the typed `registry_*`/`invalid_registry_response` error codes (S:0x51832c0). The decloud is of base code; overlaps cloud.md (pipe store item). |
| -038 | confirmed | Routes S:0x52585a8/0x52585e0, `mcp_call.rs` S:0x6a9ac38, agent identities S:0x6a9aca8. 0 base hits. Overlaps api-012 and the cloud.md telemetry item. |
| -039 | confirmed | The cited S:0x49f3a2d is unrelated (a message about third-party extension packages being off). The claim instead rests on F:7451 @10437 (`flag: acp_agent_cursor`), @9622 (`cloudRouting` `ANTHROPIC_BASE_URL` `/anthropic`) and S:0x52c8150 (`useScreenpipeCloud`). 0 base hits. |
| -040 | confirmed | The cited S:0xa0c6840 is the connection-app list (unrelated). The real evidence is S:0xa0c68be..0xa0c6d4a (`sp-cli-resolve`, a timeout message for the 180 s `bun install screenpipe@latest`, `@screenpipe/cli-win32-x64`, `receipt.json`). Base has no `cli_runtime` and no `SCREENPIPE_CLI` env; `npx screenpipe@latest` appears in base only as skill/UI text. |
| -041 | partially_in_base | Base frontend already carries the enterprise preset policy (`allow_employee_custom_presets`, `enterpriseManaged`: `lib/enterprise-ai-preset-policy.ts`, `lib/hooks/use-enterprise-policy.ts`). New: managed-pipe locks (S:0x517abad), `ManagedTeamSkill` (S:0x52c9a38), `sync_managed_team_skills`, plus `list_managed_team_skills` (in the bindings diff, missing from the item) and the injected team skill (S:0x499137c). |
| -042 | confirmed | S:0xa0ec646 (`trajectory.rs`, `jsonl_delta`), S:0xa0eca35 (env vars). Base "trajectory" hits are unrelated (MCP synthesized kinds). |
| -043 | confirmed | Module S:0x5255960, route S:0x5258b70. 0 base hits. |
| -044 | confirmed | DDL S:0xa2aaf2a, route S:0x5258728, module S:0x5183423, filters S:0x5183528. 0 base hits. The DDL is also db-010; keep it there and keep the agent-facing rules here. |
| -045 | confirmed | S:0x517ab67 (a log that over-limit pipes stay on disk but are not loaded), S:0x61872b0. Base has no pipe-count limit. |
| -046 | confirmed | S:0x49f2770 (two-stage minimum), S:0x5222a9c (drafting prompt; must not be copied), S:0x5223dde (ownership-marker failure), S:0x5223ef3 (`get-workflow` link). Both commands are in the bindings diff. 0 base hits. |

### Missed items

- **agents_pipes-m01 - Pipe run outcome verification and new failure classes (changed).** The target no longer treats a
  zero exit code as success by itself. A run is marked failed when it exits cleanly but the agent stream has no verified
  final result. When context compaction fails before a final answer, the run gets error_type `context_compaction` (with
  `compaction_end` seen in the stream). Model refusals are also classified: `content_filter`, safety refusals,
  `finish_reason`/`stop_reason` = refusal, and a provider marker for cybersecurity-risk flagging. Evidence: S:0xa078838
  (messages for a clean exit with no verified result, for compaction failing before a final answer, and for an automation
  ending without a verifiable result; plus `context_compaction`, `compaction_end`),
  S:0xa077533 (refusal vocabulary next to `event_read_only`), S:0xa0789b4 (core `pipes/mod.rs:1210`). Base has none of
  these (0 hits for the verified-result wording, `content_filter`, `context_compaction`). Base only classifies quota/credits/
  rate-limit text (`pipes/preset_fallback.rs:315-335`); the target's quota phrase list (S:0xa077829) is a refinement of
  that. Size S.
- **agents_pipes-m02 - Per-device run destinations `.local-run-destinations.json` (new).** The `run_in` chat targets are kept
  in a pipes-dir sidecar `.local-run-destinations.json`, apart from pipe.md and parallel to base `.local-overrides.json`,
  so a synced pipe does not point at another device's chat. A corrupt file is ignored with a warning. Evidence: S:0xa078528
  (file name + `pipes/mod.rs:1171`), S:0xa0787e8 (a warning that the corrupt file is being ignored). The per-device rationale
  is inferred. Add it to -009. Size XS.
- **agents_pipes-m03 - Pipe system-prompt preamble additions (changed).** The base preamble (`pipes/mod.rs:4231`: an identity
  line casting the agent as the pipe, `./output/`, API URL, auth note, connection write policy) gains several directives. Each is paraphrased here
  and Freenpipe must write its own wording: (1) read the pipe's user ratings and corrections from `GET /feedback` before
  regenerating recurring output (keep what was rated up, fix what was rated down); (2) prefer notification action buttons
  (`"actions": [...]`) to ask the user rather than send a passive FYI; (3) when `subagent: true`, use the subagent tool only
  for two or more independent parallel workstreams; (4) keep a compact user profile of stable preferences through the
  profile tool. Evidence: S:0xa077ba6, S:0xa077dbb, S:0xa077f06, S:0xa07815a. Base: 0 hits for the distinctive key
  phrases of these four directives. Ties into -022 and -044. Size S.

Checked and not counted as misses here, because another dimension owns them: removal of the base pipe-suggestions
scheduler (`pipe_suggestions_scheduler.rs` plus its two Tauri commands, 0 hits in the target, owned by frontend.md);
`analyze_workflows`/`ensure_workflows_runtime` (frontend.md, frontend-047); workflow contribution sharing
(`redact_workflow_contribution`, cloud-035). The names `workflow-memory.ts`/`workflow-catalog.ts` (S:0xa0a8acf) have no
install log and look like stale-file cleanup, not a feature.

---

## Verification (independent pass 2, 2026-10-15)

This is a second adversarial pass. It does not rely on pass 1's conclusions: I re-opened every base citation pass 1 made
instead of trusting it.

**Method.**
- **(a) Base check.** The base is `<repo>` at `0e2397a`. It has two commits; the only modified file is
  `apps/screenpipe-app-tauri/bun.lock`, and there are no untracked files, so no parallel-agent code is present.
  - I grepped 211 identifiers with `tools/ver_basegrep.py` (`work/v2_ids.txt` -> `work/v2_base.out`).
  - I opened every non-zero hit: `engine/src/pipe_store.rs`, `core/src/pipes/{mod,permissions}.rs`,
    `engine/src/{pipes_api,server,mcp_servers_api}.rs`, `routes/cloud_proxy.rs`, `core/src/agents/{pi,bash_env}.rs`,
    app `src-tauri/src/pi.rs`, `packages/screenpipe-mcp/src/index.ts`, the agent, Hermes and OpenClaw cards, the
    built-in `assets/pipes/*` and `assets/extensions/*`.
- **(b) Target evidence.**
  - I re-read every cited `S:` offset with `tools/verify_offs.py` (`work/v2_offs{1,2,3}.txt`).
  - I dumped the region SQL and log runs with `tools/ver_range.py` (`work/v2_pipesmod.txt`, `work/v2_sched.txt`,
    `work/v2_routes.txt`).
  - I re-read the `F:` positions directly in the chunks: home @601799, 5977 @474329/@479463/@112511, 3696 @631185,
    7451 catalog literals @9120..@11465, and onboarding @99394.
  - Every offset resolves. Three point at unrelated strings; see the per-item notes.
- **REA.**
  - A literal-seed trace of `/pipes/activity` on parent `ev_57f23857...216d` returned no-match, because the graph is
    partial (`ev_134986bde14b7eb2d83d861c09cfce70108226537032b57567fe4ab8c57c83d3`).
  - A module trace of `7451-47878874a9162526.js` (`ev_fa243ad128d77a21a413777e4ee44b8f8fa670986e8af07c1bbc21b318ef6c82`)
    shows the ACP catalog chunk is shared. It is reached from all 15 page entry points, including chat, settings,
    home, onboarding and `dev/workflows-ux`. This agrees with -016 and -039.

**Result: 30 confirmed, 15 partially_in_base, 1 already_in_base, 0 unsupported, 0 duplicate.**

The only verdict that differs from pass 1 is -003, which moves from confirmed to partially_in_base because `history`
already works in base. The notes below add corrections that pass 1 did not record.

| Item | Verdict | Note (pass 2) |
|---|---|---|
| -001 | partially_in_base | **Already in base** (`engine/src/pipe_store.rs`): orphan recovery of `running`+`queued` rows to `interrupted` (:133-138); the output-less `'' AS stdout` list (:254); julianday `duration_ms` (:87); and `COALESCE(?, pid)` / `COALESCE(started_at, ?)` (:49).<br>**New** (target `pipe_store.rs` strings S:0x6a008a0..0x6a02058): the 3 ALTERs; the insert carrying `trigger_event, trigger_key`; the running guard `AND status IN ('queued','running') AND finished_at IS NULL`; `id < ?` keyset paging (full and output-less); the activity aggregate; a per-pipe `COUNT(*)` (S:0x6a01dd0, backs `include_execution_counts`); `SET conversation_id`; and `summary_not_saved` / `ai_preset_unavailable` / `nothing_to_summarize` (0 base hits). |
| -002 | confirmed | **Target:** DDL, claim and prune verified (S:0xa2aa964, 0xa2aaa65, 0xa2b6e03, 0xa2b6ec5); the duplicate-suppression log (S:0x499d7be); cancelling a queued run before it starts releases the claim (S:0x499dd80, 0x499ddc0).<br>**Base:** 0 hits. Base skips an event when the pipe is busy (`pipes/mod.rs:3363`). |
| -003 | partially_in_base | **Already in base:** `history` is functional as an untyped flattened extra (`pipes/mod.rs:1932`, 2436, 3565). It drives Pi `--continue` (`agents/pi.rs:1126`, `agents/mod.rs:54`).<br>**New:** `effort`, `run_in`, `artifacts`, `schedule_config`, `cloud_agent`, `trigger.sources`, `event_read_only`; the PipeStatus fields (`has_saved_context`, `is_bundled_builtin`, `next_run`; 0 base hits); per-field validation; and envelope rejection (S:0x517af0e). Base `ConfigUpdateRequest` is a bare flattened map (`pipes_api.rs:45`).<br>**Spec correction:** artifact declarations also carry `title`. The built-in frontmatter is `artifacts: - path / title / kind` (S:0xa108953). |
| -004 | confirmed | **Target:** `ScheduleConfig` (S:0x6187760); the preview route (S:0x5258c98); UI defaults @474329; debounced POST @479463, returning `{summary,next}`. The Rust summary vocabulary (interval, weekly and monthly phrasings) is at S:0xa077481..0xa077515.<br>**Base:** already parses human strings (`every day at 9am`, `every monday at 6pm`; `pipes/mod.rs:4384-4424`). So only the structured config and the preview are new. |
| -005 | confirmed | **Base** (`pipes_api.rs`):<br>- list returns `{data,total}` and accepts only `include_executions`;<br>- `get_pipe` returns 200 with an error body;<br>- `ExecutionsQuery` has only `limit`;<br>- `RunPipeBody` has only `notification_context`.<br>**Target** routes: S:0x5258c20..0x5258e38. Frontend use of `/pipes/activity` is at home @615016, and of bundled install at onboarding @99394.<br>**Unsupported row:** `/pipes/info` has no route in base `server.rs` or in the target pipe sub-router strings. It exists only as an allowlist entry, in both. |
| -006 | confirmed | **Target:** route S:0x5259048; frames `progress`/`keep-alive` (S:0x6187ea0); cap message S:0x6187ed8, in engine `pipe_stream.rs`. 0 base hits; base `pipe_stream` is only the e2e command `e2e_emit_pipe_stream`.<br>**Caveat:** the `{"type":"pipe_done"}` marker already exists in base (`pipes/mod.rs:2031`). |
| -007 | partially_in_base | **Already in base:**<br>- topics `meeting_started`, `meeting_ended`, `workflow_event`, `pipe_completed:*` (`pipes/mod.rs:3247-3315`);<br>- a busy event is skipped (:3363);<br>- default-preset fallback (:3511);<br>- clearing the Pi session on context overflow (:2196, 3886);<br>- the generation-tagged scheduler (:3211, 3273), circular-chain guard (:3347) and agent-registry checks (:3437);<br>- the one-off past-time error (:4333).<br>**New:** topics `meeting_summary_refresh_requested` and `connection_trigger` (S:0x499d460); defer, coalesce and backlog cap (S:0x499d5a9, 0x499db5e..0x499dc0b); `.trigger-context.json`; the trivial-meeting skip; `"*"`; refusal to fall back for a missing named preset (S:0x499d8ed; base silently uses `config.model` at :3502-3508); and persisting a skipped run (S:0x499d620). |
| -008 | confirmed | **Target:** S:0xa0e5de8, 0xa0e5ff8, 0xa0e615e, 0xa0e68f8 verified.<br>**Base:** 0 hits for `connection_trigger`, `.connection-triggers.json`, `source_delivery_id`. |
| -009 | partially_in_base | **Already in base:** `history` resume, and `Output directory: ./output/` in the base preamble (`pipes/mod.rs:4231`).<br>**New:** `run_in` / `existing_chat` (S:0xa043cc8, 0xa04942f); `.screenpipe-acp-sessions.json` (S:0x517b108); the scheduled-task directory line in the preamble (S:0xa049378); and `conversation_id`.<br>**Missing from the spec:** the per-device `.local-run-destinations.json` (m02). |
| -010 | confirmed | **Target:** DDL S:0xa2a1af2, 0xa2a2d2a, 0xa2aad72; `register-artifact.ts` (S:0xa0ac9d0); routes in the S:0x5257ca8 blob.<br>**Base:** 0 hits.<br>Overlaps db.md, api.md and meetings_memory.md, which own the table and routes. |
| -011 | confirmed | **Target:** S:0x522d17b, 0x522d360, 0x5258618..0x52586c0 (4 routes), 0xa0b0049.<br>**Base:** 0 hits. |
| -012 | confirmed | **Target:** exactly 21 bundled `pipe.md` frontmatters, in two embedded copies (S:0xa05449b.. and S:0xa0efbea..); frontmatter spot-checked for `commitments`, `accounting-follow-through`, `speaker-reconciliation`, `skill-learning`. `ai-prompt-journal` has 0 target hits.<br>**Base:** 7 bundled pipes.<br>**Caveat:** base `meeting-summary` already has `preset: [screenpipe-cloud]` + `meeting_ended`; the target adds `"*"` and `timeout`. |
| -013 | partially_in_base | **Already in base:**<br>- the same log for a skill removed because pipe permissions deny it (`agents/pi.rs:582`);<br>- the route-not-permitted and concurrent-query backpressure messages (`pipe_permissions_middleware.rs`);<br>- an `x-screenpipe-session` header added by the bash shim (`agents/bash_env.rs:108`).<br>**New:**<br>- reader allowlist additions (S:0xa0c8030, 0xa0c8088 vs base `permissions.rs:126-139`);<br>- `mcp_access.rs` (S:0x499d1d8);<br>- `SCREENPIPE_MCP_SERVER_ALLOWLIST`;<br>- workflow operation limits. |
| -014 | confirmed | **Target:** install logs verified (S:0x517b6f0 region, 0x52a0afd, 0x52a9dec, 0xa0a26ba). The full embedded extension list is in the frontmatter/source sweep (core: chat-control, context-pruning, workflow-workspace, skill-learning, mcp-bridge, orphan-guard, register-artifact, self-improvement, structured-output, sub-agent, web-search, screenpipe-permissions; app: web-search, mcp-bridge, save-artifact, live-views, connection-gate, context-pruning, tinfoil, work-context, workflow-feedback, S:0x528f260).<br>**Base:** 6 core + 2 app extensions; tools named `mcp_list_tools` / `mcp_call` / `web_search`. The target renames them to `sp_*`.<br>**Umbrella item:** the tool ids overlap -019, -021, -022 and -043. |
| -015 | partially_in_base | **Already in base:** `models.json` merge, skip-without-baseUrl and rebuild-on-corrupt (`agents/pi.rs:721-898`); `anthropic-byok`; `package.json` `overrides` seeding (:1847-1928); PortableGit 2.47.1.<br>**New:** the 0.84.1 pins (S:0xa0b5c00; base 0.75.4 at `agents/pi.rs:18-19`); isolated pi-config with `.migrated-from-global` (S:0xa0b4900); baseline bun 1.3.10; the bash probe (S:0xa0b7148); the compaction patch; and the `PI_*` / `SCREENPIPE_PI_AGENT_DIR` env. |
| -016 | confirmed | **Target:**<br>- F:7451 literals @9120 (`pi-acp@0.0.32`), @9334, @9573, @10445/@10768/@11465 (flags), @9630 (`cloudRouting`);<br>- env S:0xa0bef96, 0xa0bf167;<br>- install messages S:0x499ed40;<br>- REA `ev_fa243ad1...` shows the chunk is shared.<br>**Base:** 0 hits for every ACP id.<br>The command list omits `pi_acp_set_config_option` and `pi_acp_set_approval_mode` (in the bindings diff). |
| -017 | partially_in_base | **Already in base:** `enterpriseManaged` and `maxContextChars` 512000 (`lib/enterprise-ai-preset-policy.ts:82,133`, `lib/hooks/use-enterprise-policy.ts:71-73`). `PiProviderConfig` is already `rename_all = "camelCase"` with `systemPrompt` (`src-tauri/src/pi.rs:1014-1030`; 6 fields vs target 12), so the item's "adds systemPrompt" is wrong.<br>**New:** provider `acp`; `acpAgent` (9 fields, S:0x52c8150); `backend` / `allowedTools` / `resumeSessionId` / `unattended` (S:0x52c8680; F:3696 @631185); and per-chat persisted `presetId`. Base `presetId` is used only in `watch_pipe`. |
| -018 | partially_in_base | **Already in base:** archived chats (`lib/chat-storage.ts:54`); `branchConversation` (`use-chat-conversations.ts:1258`); the pool-full error (`src-tauri/src/pi.rs:1495`, which is the cited S:0x4991251); and `titleSource`, `lastContentAt`, `lastViewedAt`.<br>**New:** the 7 commands (0 base hits); start stages; and events `thinking_level_changed`, `extension_ui_request`, `compaction_end` (0 base hits). |
| -019 | confirmed | **Target:** F:home @601799 (`.codex/sessions` root); F:378 `importedFrom` @42061; broker S:0x499e5a0; `chat-control.ts` S:0xa0a0418; `SCREENPIPE_CHAT_CONTROL_*` present.<br>**Base:** 0 hits. |
| -020 | confirmed | **Target:** S:0xa04a0e0 (git env hardening), 0xa04aaf8 (owner record), 0xa0a105f (route prefix).<br>**Base:** 0 hits.<br>`coding_workspace_*` commands are owned by frontend.md. |
| -021 | confirmed | **Target:** route S:0x5258510; provenance marker S:0x6a0f32f; the unsafe-pattern id list at L632803 (`0x6a0f519`); `skillManageParameters` S:0xa0a759e.<br>**Base:** 0 hits. |
| -022 | confirmed | **Target:** S:0x5258540, 0x5258570, 0x6a43757, 0xa0a6ce0.<br>**Base:** 0 hits. |
| -023 | confirmed | **Target:** S:0x52584e0, 0x499e1f1, 0xa0a8acf, 0xa0fc668.<br>**Base:** 0 hits. |
| -024 | partially_in_base | **Already in base:** removal of exactly the same 9 deprecated skill dirs (`agents/pi.rs:376-396`).<br>**New:** `screenpipe-chats`, `screenpipe-workflow-maintenance`, `video-sop`, and the 20 starter skills. Their 20 frontmatters are at S:0x511e4e0.. (three embedded copies), with markers at S:0xa0c6708. |
| -025 | confirmed | **Target:** S:0x4995d13, 0x4995df0, 0x52241b7, 0x52c96e0.<br>**Base:** the agent, Hermes and OpenClaw cards only save screenpipe's own api/cli `SKILL.md` to a download for manual copying (`components/settings/agent-card.tsx:220`). There is no registry and no export command. |
| -026 | confirmed | **Target:** S:0x62092e8, 0x6208dc8, 0x6209a98, 0x6209eb0; 9 workflow routes in the S:0x5257ca8 blob.<br>**Base:** `/workflows` appears only as an MCP `team-records` path (`index.ts:1793`). |
| -027 | confirmed | **Target:** the gate strings sit about 330 chars into the S:0xa0bf443 line (`/hosted_ai/plan`, `workflow_usage_unavailable`, `workflow_business_required`, `cost_limit_reached`, `background_pipe_advisory`), plus S:0xa074b28.<br>**Base:** 0 hits. |
| -028 | confirmed | **Target:** S:0x52ca560, 0xa049fa0, 0xa049628..0xa049e80 (Codex, Claude and Cursor cloud launch paths); `https://api.cursor.com` (S:0x522a0b8).<br>**Base:** 0 hits.<br>**Decloud note:** the Claude Cloud snapshot commits with a screenpipe-branded git identity (S:0xa049730). Freenpipe must use its own. |
| -029 | confirmed | **Target:** S:0x5a64cd4, 0x5a65045, 0x52bb228.<br>**Base:** 0 hits. |
| -030 | partially_in_base | **Already in base:** MCP 0.18.10 with 26 tools; `search-content` accepts `memory` (`index.ts:279`); memory sidecar markers `screenpipe-memories:start/end` (`core/src/memories/external_sync.rs:40-44`).<br>**New:** 11 tool names (S:0x6a9a7c0; 0 base hits, except `pipe-logs` and `screenpipe-skills` in unrelated skill/UI files); `--screenpipe-portable-mcp` (S:0xa1170fe); `SCREENPIPE_MCP_*` (S:0xa117260). |
| -031 | partially_in_base | **Already in base:** onboarding writes the `screenpipe` entry for Claude Desktop, Cursor and Codex (`connect-apps.tsx`, `connections-section.tsx`). The base CLI `mcp setup` (`cli/mcp.rs:40`) only stages MCP files.<br>**New:** `agent setup --target` x10 (S:0x6a5af4c..0x6a5b52d); backups and locks; `resolve_ai_tool_config_path` / `set_ai_tool_auto_connect_opt_out`. |
| -032 | confirmed | **Target:** registry proxy (F:5977 @112511; URL S:0x6188550); `app_scheme` + screenpi.pe relay (S:0x6a8dd01); vendor list (S:0x6a33590); Composio (S:0x52a6792 and `api/composio` in F:5977 and onboarding).<br>**Base:** 0 hits for all of these.<br>**Decloud note:** base MCP OAuth already defaults to the loopback callback (`mcp_servers_api.rs:395-397`). The decloud is therefore "do not adopt the relay", not "replace it". |
| -033 | confirmed | **Target:** S:0x5259090, 0x5253b30.<br>**Base:** 0 hits. |
| -034 | already_in_base | The route, `api.screenpipe.com` forwarding and the `cloud_token_missing` / `upstream_unreachable` bodies are identical in base (`routes/cloud_proxy.rs:23-53`, `server.rs:792`). This is a decloud of base code, not a delta. |
| -035 | partially_in_base | **Already in base:** `web-search.ts` (core and app) calls `https://api.screenpipe.com/v1/web-search` with tool `web_search` (`assets/extensions/web-search.ts:20,47`).<br>**New:** the engine route `/v1/web-search` (S:0x5258ae8; `routes/web_search.rs` S:0x6188ef3), the `sp_web_search` rename, and the local web-search extension (S:0x52aaea1). |
| -036 | partially_in_base | **Already in base:** the `screenpipe-cloud` provider, and a fetch of `https://api.screenpipe.com/v1/models` (`components/rewind/ai-presets-selector.tsx:209`).<br>**New:** `screenpipe-tinfoil` (S:0xa084025; `glm-protocol.ts`, `tinfoil-transport.ts`); the tier fallback notice that a substitute model ran (S:0xa0b9f11..0xa0b9f4a); ai-genesis URLs (S:0xa0b88e0). |
| -037 | partially_in_base | **Already in base:** `routes/pipe_store.rs` proxies `SCREENPIPE_API_BASE_URL` (default `https://screenpi.pe`) and reports `skipped_modified`.<br>**New:** only the typed `registry_*` error codes (S:0x51832c0). |
| -038 | confirmed | **Target:** S:0x52585a8, 0x52585e0, 0x6a9ac38, 0x6a9aca8.<br>**Base:** 0 hits.<br>Overlaps api.md and cloud.md. |
| -039 | confirmed | **Evidence fix:** the cited S:0x49f3a2d is unrelated (a message about third-party extension packages being off). The support is F:7451 @10445, @10768, @11465 (flags) and @9630 (`cloudRouting`), plus S:0x52c8150 and F:7451 @42953/@43038 (`useScreenpipeCloud`).<br>**Base:** 0 hits. |
| -040 | confirmed | **Evidence fix:** the cited S:0xa0c6840 is the connection-app list (unrelated). The support is S:0xa0c68be..0xa0c6d4a (`sp-cli-resolve`, the 180 s `bun install screenpipe@latest`, `@screenpipe/cli-win32-x64`, `receipt.json`), and the skill text using `${SCREENPIPE_CLI:-bun x screenpipe@latest}` (S:0x6a57403).<br>**Base:** the `SCREENPIPE_CLI` hits are only the `SCREENPIPE_CLI_SKILL_MD` constant. |
| -041 | partially_in_base | **Already in base:** `allow_employee_custom_presets` and `enterpriseManaged` (`lib/enterprise-ai-preset-policy.ts`).<br>**New:** managed-pipe locks (S:0x517abad); `ManagedTeamSkill` (S:0x52c9a38); `sync_managed_team_skills` / `list_managed_team_skills`; the injected team skill (S:0x499137c). |
| -042 | confirmed | **Target:** S:0xa0ec646, 0xa0eca35.<br>**Base:** 0 hits. |
| -043 | confirmed | **Evidence fix:** S:0x52a59e8 is only a bare source header. The support is S:0x5255960 (`connect_broker.rs`), 0x5258b70/0x5258ba0 (routes), 0x5255a49 and 0x6a6aba0 (`acp_connect_*` events), and 0xa0d302b.<br>**Base:** 0 hits. |
| -044 | confirmed | **Target:** S:0xa2aaf2a, 0x5258728, 0x5183423, 0x5183528.<br>**Base:** 0 hits.<br>db.md owns the DDL. |
| -045 | confirmed | **Target:** S:0x517ab67, 0x61872b0. The stronger evidence for the tier link is `free_pipe_limit_reached` with a message capping installed pipes on the free plan (S:0xa053668..0xa0536ae); the same code also appears in the onboarding chunk.<br>**Base:** 0 hits. |
| -046 | confirmed | **Target:** S:0x49f2770, 0x5222a9c, 0x5223dde, 0x5223ef3; Pi session `pi-workflow-skill` (S:0x49f2838). Drafting is gated on screenpipe sign-in; Freenpipe should use the user's preset instead.<br>**Base:** 0 hits. |

**Cross-dimension overlaps.** These are not duplicates inside this list; keep each item here to its agent-facing
behaviour.
- `pipe_event_runs` / `outputs` / `feedback` DDL: db.md, meetings_memory.md.
- `/artifacts*`, `/outputs/targets*`, `/v1/local/chat/completions`: api.md.
- Telemetry, web-search, cloud proxy: api.md, cloud.md.
- `coding_workspace_*`, `generate_workflow_skill`, `analyze_workflows`, `pipe_suggestions` removal: frontend.md.
- Skills registry, grokbot: cloud.md, connections.md.

### Missed items (pass 2)

pass 2 re-verified pass 1's m01-m03 against the base and the target, and extends m01. It adds m04-m06.

- **agents_pipes-m01: Run outcome verification and new failure classes (changed, S).**
  - **What changes.** A clean exit no longer counts as success on its own.
    - A run with no verified final result is marked failed.
    - A compaction failure before the final answer gets `context_compaction` (with `compaction_end` in the stream).
    - Refusals are classified: `content_filter`, safety refusal, `finish_reason` / `stop_reason` = refusal, and the
      cybersecurity-flag marker.
    - Workflow tasks that finish without saving their required update fail with error code `missing_output`.
  - **Evidence.** S:0xa078838, S:0xa077533, S:0xa0773ad, S:0xa0b8cf8.
  - **Base.** 0 hits for the verified-result wording, `missing_output`, `context_compaction`, `content_filter`.
- **agents_pipes-m02: Per-device run destinations `.local-run-destinations.json` (new, XS).**
  - **What changes.** `run_in` chat targets are kept in a pipes-dir sidecar beside base `.local-overrides.json`; a
    corrupt file is ignored with a warning.
  - **Evidence.** S:0xa078528 (with `pipes/mod.rs:1171`), S:0xa0787e8.
  - **Base.** 0 hits.
- **agents_pipes-m03: Pipe preamble additions (changed, S).** Freenpipe must write its own wording for each of these.
  - **What changes.** The base preamble (`pipes/mod.rs:4231`) gains four directives:
    - read the pipe's ratings and corrections from `GET /feedback` before regenerating recurring output;
    - prefer notification action buttons for decisions;
    - use the subagent tool only for two or more parallel workstreams;
    - keep a compact stable user profile.
  - **Evidence.** S:0xa077ba6, S:0xa077dbb, S:0xa077f06, S:0xa07815a.
  - **Base.** 0 hits in `crates` for these directives. The phrase "action buttons" appears only as a heading in the API
    skill doc.
- **agents_pipes-m04: Pipe `Content()` permission types gain `memory` and `parsed` (changed, XS).**
  - **What changes.** The pipe sandbox's content-type set grows from `ocr, audio, input, accessibility` to also include
    `memory` and `parsed`. The built-in `speaker-reconciliation` allows `Content(audio, accessibility, parsed)`.
  - **Evidence.** L696247 (`screenpipe-permissions` extension `getAllowedContentTypes`), S:0xa1087dd.
  - **Base.** `assets/extensions/screenpipe-permissions.ts:160` and `pipes/permissions.rs:56` know only the four
    original types.
  - **Ownership.** The `/search` `parsed` content type itself belongs to api.md and meetings_memory.md. This item is only
    the permission-gate side.
- **agents_pipes-m05: Pipe identifier validation (changed, XS).**
  - **What changes.** Pipe ids must be a single portable path component, which guards against path traversal through
    API and CLI pipe names.
  - **Evidence.** S:0xa0777e6..0xa077800.
  - **Base.** 0 hits.
- **agents_pipes-m06: Bulk pipe-log archive sweep (new, XS).**
  - **What changes.** A periodic blocking task scans pipe log directories and archives old per-run log files. It
    reports scanned, archived, byte and failure counts. Retention and format are not observed.
  - **Evidence.** S:0x499d580 (task name), S:0xa07776a..0xa0777d9.
  - **Base.** Writes one `logs/<ts>.json` per run (`pipes/mod.rs:2190`, 3880) but has no sweep (0 hits for
    the log-archive / pipe-log-files wording). Pass 1's -007 note credited base with "archiving", but base only writes the
    per-run logs.
