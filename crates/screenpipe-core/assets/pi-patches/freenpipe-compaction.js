// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Freenpipe's changes to Pi's context compaction.
 *
 * The Freenpipe runtime copies this file into the installed Pi package as
 * `dist/core/compaction/freenpipe-compaction.js` and adds a few one-line
 * calls to it in Pi's own files (see `pi-0.84.1.fpatch` next to this file).
 * Pi stays in charge of compaction; these helpers change when it runs, how
 * much it keeps, what the summariser sees and what counts as a usable
 * summary:
 *
 * - After a step that ran tools, the session is compacted as soon as the
 *   context passes 70% of the model's window, instead of waiting for the end
 *   of the run, so long pipe runs do not overflow mid-way.
 * - The reserve and keep-recent budgets shrink for small context windows,
 *   sized from the prompt overhead (system prompt and tool schemas) seen in
 *   the provider's usage figures, so compaction can actually make room.
 * - The summariser gets the history as one JSON record per message, with
 *   long tool output cut to a head-and-tail excerpt of up to 8,000
 *   characters.
 * - A summary that was cut off, came back empty or misses its main sections
 *   is rejected rather than stored.
 * - If compaction fails in the middle of a run, the run stops with an error
 *   instead of carrying on into an overflow and compacting again and again.
 *
 * The module imports nothing, so it adds no import cycle to Pi's modules;
 * the hooks pass in the few Pi functions it needs. Decisions that go wrong
 * fall back to Pi's stock behaviour.
 */

export const FREENPIPE_COMPACTION_VERSION = 1;

/** Share of the context window at which a tool step triggers compaction. */
export const PROACTIVE_FRACTION = 0.7;
/** Longest tool output (characters) kept in what the summariser reads. */
export const TOOL_EXCERPT_CHARS = 8000;
/** Smallest excerpt used on very small context windows. */
export const MIN_TOOL_EXCERPT_CHARS = 1200;
/** Pi's own token estimate is characters divided by four. */
const CHARS_PER_TOKEN = 4;
/** Floors for the sized budgets, in tokens. */
const MIN_RESERVE_TOKENS = 1024;
const MIN_KEEP_RECENT_TOKENS = 1024;
/** Share of the usable window (window minus prompt overhead) per budget. */
const RESERVE_SHARE = 0.25;
const KEEP_RECENT_SHARE = 0.3;
/** Proactive compactions in a row that may leave the context above the
 *  threshold before the run is stopped. */
const MAX_INEFFECTIVE_COMPACTIONS = 2;

// ---------------------------------------------------------------------------
// Excerpts and history records
// ---------------------------------------------------------------------------

let excerptLimit = TOOL_EXCERPT_CHARS;

/** Excerpt length for a model with `contextWindow` tokens: the full 8,000
 *  characters from about 40K tokens up, proportionally less below. */
export function excerptLimitFor(contextWindow) {
    if (!(contextWindow > 0)) return TOOL_EXCERPT_CHARS;
    const scaled = Math.floor((contextWindow * CHARS_PER_TOKEN) / 20);
    return Math.max(MIN_TOOL_EXCERPT_CHARS, Math.min(TOOL_EXCERPT_CHARS, scaled));
}

/** Set the excerpt length used by {@link serializeConversationRecords}. */
export function setExcerptLimit(limit) {
    excerptLimit = limit > 0 ? Math.floor(limit) : TOOL_EXCERPT_CHARS;
}

/** Current excerpt length. */
export function currentExcerptLimit() {
    return excerptLimit;
}

/**
 * Shorten `text` to at most about `limit` characters, keeping its beginning
 * and its end (where errors and totals usually are) and saying how much was
 * left out in between.
 */
export function excerpt(text, limit = excerptLimit) {
    if (typeof text !== "string") return "";
    if (text.length <= limit) return text;
    const head = Math.ceil(limit * 0.6);
    const tail = Math.max(0, limit - head);
    const omitted = text.length - head - tail;
    return `${text.slice(0, head)}\n[… ${omitted} characters omitted …]\n${tail > 0 ? text.slice(-tail) : ""}`;
}

function textOf(content) {
    if (typeof content === "string") return content;
    if (!Array.isArray(content)) return "";
    const parts = [];
    for (const block of content) {
        if (block && block.type === "text" && typeof block.text === "string") parts.push(block.text);
        else if (block && block.type === "image") parts.push("[image]");
    }
    return parts.join("\n");
}

function shortenValues(value, limit, depth = 0) {
    if (typeof value === "string") return excerpt(value, limit);
    if (depth > 6 || value === null || typeof value !== "object") return value;
    if (Array.isArray(value)) return value.map((v) => shortenValues(v, limit, depth + 1));
    const out = {};
    for (const [k, v] of Object.entries(value)) out[k] = shortenValues(v, limit, depth + 1);
    return out;
}

/** One summariser record for a Pi message, or `undefined` to skip it. */
export function messageRecord(message, limit = excerptLimit) {
    if (!message || typeof message !== "object") return undefined;
    switch (message.role) {
        case "user": {
            const text = textOf(message.content);
            return text ? { role: "user", text } : undefined;
        }
        case "assistant": {
            const record = { role: "assistant" };
            const thinking = [];
            const texts = [];
            const calls = [];
            for (const block of Array.isArray(message.content) ? message.content : []) {
                if (!block) continue;
                if (block.type === "thinking" && block.thinking) thinking.push(block.thinking);
                else if (block.type === "text" && block.text) texts.push(block.text);
                else if (block.type === "toolCall") {
                    calls.push({ tool: block.name, arguments: shortenValues(block.arguments ?? {}, limit) });
                }
            }
            if (thinking.length) record.thinking = thinking.join("\n");
            if (texts.length) record.text = texts.join("\n");
            if (calls.length) record.tool_calls = calls;
            if (message.stopReason === "error" && message.errorMessage) record.error = message.errorMessage;
            return Object.keys(record).length > 1 ? record : undefined;
        }
        case "toolResult": {
            const text = textOf(message.content);
            const record = { role: "tool_result", tool: message.toolName };
            if (message.isError) record.is_error = true;
            record.output = excerpt(text, limit);
            return record;
        }
        default:
            return undefined;
    }
}

/**
 * The history as the summariser reads it: a short legend line, then one JSON
 * object per message. Records keep roles and tool calls unambiguous, and the
 * summariser is less tempted to continue a transcript than with chat-like
 * text.
 */
export function serializeConversationRecords(messages, limit = excerptLimit) {
    const lines = [
        "Each line below is one JSON record of the conversation, oldest first (fields: role, text, thinking, tool_calls, tool, output, is_error, error).",
    ];
    for (const message of Array.isArray(messages) ? messages : []) {
        const record = messageRecord(message, limit);
        if (record) lines.push(JSON.stringify(record));
    }
    return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Summary validation
// ---------------------------------------------------------------------------

/** Sections a history summary must contain (Pi asks for these headings). */
export const REQUIRED_SUMMARY_SECTIONS = ["Goal", "Progress", "Next Steps"];
/** Sections the summary of a split turn's first part must contain. */
export const REQUIRED_TURN_PREFIX_SECTIONS = ["Original Request", "Early Progress"];
const MIN_SUMMARY_CHARS = 40;

function hasSection(text, name) {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/ /g, "\\s+");
    return new RegExp(`^\\s{0,3}#{1,6}\\s*${escaped}\\b`, "im").test(text);
}

/**
 * Problems that make a summariser response unusable, empty when it is fine.
 * `kind` is "history" for the main summary and "turn-prefix" for the summary
 * of the first part of a split turn.
 */
export function summaryProblems(response, kind = "history") {
    const problems = [];
    if (!response || typeof response !== "object") return ["no response"];
    if (response.stopReason === "length") problems.push("it was cut off at the output limit");
    if (response.stopReason === "aborted") problems.push("it was aborted");
    const text = textOf(response.content).trim();
    if (text.length < MIN_SUMMARY_CHARS) {
        problems.push("it is empty or too short");
    } else {
        const required = kind === "turn-prefix" ? REQUIRED_TURN_PREFIX_SECTIONS : REQUIRED_SUMMARY_SECTIONS;
        const missing = required.filter((s) => !hasSection(text, s));
        if (missing.length) problems.push(`it lacks the ${missing.join(", ")} section(s)`);
    }
    return problems;
}

/** Throw when the summariser response is unusable; Pi then reports the
 *  compaction as failed instead of storing a broken summary. */
export function checkSummary(response, kind = "history") {
    const problems = summaryProblems(response, kind);
    if (problems.length) {
        throw new Error(`the ${kind} summary was rejected: ${problems.join("; ")}`);
    }
}

// ---------------------------------------------------------------------------
// Budget sizing
// ---------------------------------------------------------------------------

function estimateTextTokens(chars) {
    return Math.ceil(chars / CHARS_PER_TOKEN);
}

function roughMessageChars(message) {
    if (!message || typeof message !== "object") return 0;
    if (message.role === "assistant") {
        let chars = 0;
        for (const block of Array.isArray(message.content) ? message.content : []) {
            if (!block) continue;
            if (block.type === "text") chars += (block.text ?? "").length;
            else if (block.type === "thinking") chars += (block.thinking ?? "").length;
            else if (block.type === "toolCall") chars += JSON.stringify(block.arguments ?? {}).length + 32;
        }
        return chars;
    }
    return textOf(message.content).length + (message.role === "toolResult" ? 16 : 0);
}

function promptTokensOf(usage) {
    if (!usage) return 0;
    return (usage.input ?? 0) + (usage.cacheRead ?? 0) + (usage.cacheWrite ?? 0);
}

/**
 * Tokens the provider counts for everything except the messages: the system
 * prompt and tool schemas. Taken from the last assistant message with usage
 * figures (prompt tokens minus the estimated size of the messages it saw);
 * without usage, estimated from the system prompt and tool definitions.
 */
export function observedOverheadTokens(messages, systemPrompt, tools, estimateTokens) {
    const list = Array.isArray(messages) ? messages : [];
    const estimate = typeof estimateTokens === "function" ? estimateTokens : (m) => estimateTextTokens(roughMessageChars(m));
    for (let i = list.length - 1; i >= 0; i--) {
        const m = list[i];
        if (m && m.role === "assistant" && m.stopReason !== "error" && m.stopReason !== "aborted") {
            const prompt = promptTokensOf(m.usage);
            if (prompt > 0) {
                let seen = 0;
                for (let j = 0; j < i; j++) seen += estimate(list[j]);
                return Math.max(0, prompt - seen);
            }
        }
    }
    let chars = typeof systemPrompt === "string" ? systemPrompt.length : 0;
    try {
        chars += JSON.stringify(Array.isArray(tools) ? tools.map((t) => ({ name: t?.name, description: t?.description, parameters: t?.parameters })) : []).length;
    } catch {
        // Unserialisable tool definitions: count the system prompt only.
    }
    return estimateTextTokens(chars);
}

/**
 * Compaction budgets for a model window. Only ever lowers the configured
 * values: large windows keep Pi's defaults, small ones get budgets that
 * leave room after compaction (overhead + summary + kept messages stays
 * well below the proactive threshold).
 */
export function sizeCompactionSettings(base, contextWindow, overheadTokens) {
    if (!base || !(contextWindow > 0)) return base;
    const overhead = Math.max(0, overheadTokens || 0);
    const usable = Math.max(contextWindow - overhead, Math.floor(contextWindow * 0.25));
    const reserve = Math.max(MIN_RESERVE_TOKENS, Math.floor(usable * RESERVE_SHARE));
    const keep = Math.max(MIN_KEEP_RECENT_TOKENS, Math.floor(usable * KEEP_RECENT_SHARE));
    return {
        ...base,
        reserveTokens: Math.min(base.reserveTokens, reserve),
        keepRecentTokens: Math.min(base.keepRecentTokens, keep),
    };
}

/**
 * Raise `keepRecentTokens` to `minKeep` when given. Used while compacting in
 * the middle of a run: Pi only cuts the history at user or assistant
 * messages, so the budget must at least cover the latest step (its
 * assistant message and tool results), or there is no place to cut.
 */
export function withMinimumKeep(settings, minKeep) {
    if (!settings || !(minKeep > 0) || settings.keepRecentTokens >= minKeep) return settings;
    return { ...settings, keepRecentTokens: Math.ceil(minKeep) };
}

/** Hook in `AgentSession`: the compaction settings for its current model. */
export function settingsForSession(session, base, helpers = {}) {
    try {
        const contextWindow = session?.model?.contextWindow ?? 0;
        if (!(contextWindow > 0)) return base;
        setExcerptLimit(excerptLimitFor(contextWindow));
        const state = session.agent?.state ?? {};
        const overhead = observedOverheadTokens(state.messages, state.systemPrompt, state.tools, helpers.estimateTokens);
        const sized = sizeCompactionSettings(base, contextWindow, overhead);
        return withMinimumKeep(sized, runStates.get(session)?.minKeep);
    } catch {
        return base;
    }
}

// ---------------------------------------------------------------------------
// Proactive compaction after tool steps
// ---------------------------------------------------------------------------

const runStates = new WeakMap();

function stateOf(session) {
    let state = runStates.get(session);
    if (!state) {
        state = { failure: undefined, ineffective: 0, minKeep: undefined };
        runStates.set(session, state);
    }
    return state;
}

/** Hook in `AgentSession`'s auto-compaction error path. */
export function noteCompactionFailure(session, message) {
    try {
        stateOf(session).failure = message || "compaction failed";
    } catch {
        // Never let bookkeeping break Pi's own error handling.
    }
}

/** Whether a step's context size calls for compaction. */
export function overThreshold(contextTokens, contextWindow, fraction = PROACTIVE_FRACTION) {
    return contextWindow > 0 && contextTokens > contextWindow * fraction;
}

/** Sum of per-message estimates, ignoring recorded usage figures. */
export function estimateMessagesTokens(messages, estimateTokens) {
    const estimate = typeof estimateTokens === "function" ? estimateTokens : (m) => estimateTextTokens(roughMessageChars(m));
    let total = 0;
    for (const m of Array.isArray(messages) ? messages : []) {
        try {
            total += estimate(m) || 0;
        } catch {
            total += estimateTextTokens(roughMessageChars(m));
        }
    }
    return total;
}

/** Tokens of the latest step: its assistant message and everything after. */
export function latestStepTokens(messages, estimateTokens) {
    const list = Array.isArray(messages) ? messages : [];
    let start = list.length;
    for (let i = list.length - 1; i >= 0; i--) {
        if (list[i] && list[i].role === "assistant") {
            start = i;
            break;
        }
    }
    return estimateMessagesTokens(list.slice(start), estimateTokens);
}

/** Margin over the latest step so the cut lands on its assistant message. */
const KEEP_MARGIN_TOKENS = 64;

function latestCompactionId(session) {
    const branch = session.sessionManager?.getBranch?.() ?? [];
    for (let i = branch.length - 1; i >= 0; i--) {
        if (branch[i]?.type === "compaction") return branch[i].id ?? i;
    }
    return undefined;
}

/**
 * Hook in `AgentSession`'s next-turn preparation, after a step's tool
 * results are in. When the context passed {@link PROACTIVE_FRACTION} of the
 * window, compacts through Pi's own auto-compaction and returns the context
 * the next model call should see. Throws (ending the run with an error) when
 * compaction fails, or keeps leaving the context over the threshold, so the
 * run does not loop into an overflow.
 */
export async function afterToolStep(session, turn, context, helpers = {}) {
    let contextWindow;
    let state;
    try {
        if (!turn || !Array.isArray(turn.toolResults) || turn.toolResults.length === 0) return context;
        if (session.isCompacting) return context;
        const settings = session.settingsManager?.getCompactionSettings?.();
        if (!settings || !settings.enabled) return context;
        contextWindow = session.model?.contextWindow ?? 0;
        if (!(contextWindow > 0) || typeof helpers.estimateContextTokens !== "function") return context;
        const messages = context?.messages ?? session.agent?.state?.messages ?? [];
        const tokens = helpers.estimateContextTokens(messages).tokens;
        if (!overThreshold(tokens, contextWindow)) {
            stateOf(session).ineffective = 0;
            return context;
        }
        state = stateOf(session);
        state.failure = undefined;
        state.minKeep = latestStepTokens(messages, helpers.estimateTokens) + KEEP_MARGIN_TOKENS;
    } catch {
        return context;
    }

    const before = latestCompactionId(session);
    try {
        await session._runAutoCompaction("threshold", false);
    } finally {
        state.minKeep = undefined;
    }
    if (state.failure) {
        const reason = state.failure;
        state.failure = undefined;
        throw new Error(
            `Context compaction failed during this run (${reason}). The run was stopped so it does not overflow the model's context window; start it again or pick a model with a larger context.`,
        );
    }
    if (latestCompactionId(session) === before) {
        // Nothing could be summarised (for example one huge recent message).
        return context;
    }
    const compacted = session.agent.state.messages.slice();
    // Kept messages still carry the usage figures of the larger, pre-compaction
    // context, so the size after compaction is estimated message by message.
    const after = estimateMessagesTokens(compacted, helpers.estimateTokens);
    state.ineffective = overThreshold(after, contextWindow) ? state.ineffective + 1 : 0;
    if (state.ineffective >= MAX_INEFFECTIVE_COMPACTIONS) {
        state.ineffective = 0;
        throw new Error(
            "The context stayed above the compaction threshold even after compacting, so the run was stopped instead of compacting again and again.",
        );
    }
    return { ...context, messages: compacted };
}
