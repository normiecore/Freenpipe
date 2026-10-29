// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import {
  afterToolStep,
  checkSummary,
  estimateMessagesTokens,
  excerpt,
  excerptLimitFor,
  latestStepTokens,
  messageRecord,
  noteCompactionFailure,
  observedOverheadTokens,
  overThreshold,
  serializeConversationRecords,
  settingsForSession,
  sizeCompactionSettings,
  summaryProblems,
  TOOL_EXCERPT_CHARS,
  withMinimumKeep,
} from "./freenpipe-compaction.js";

const DEFAULTS = { enabled: true, reserveTokens: 16384, keepRecentTokens: 20000 };

describe("excerpts", () => {
  test("short text is kept whole", () => {
    expect(excerpt("hello", 100)).toBe("hello");
  });

  test("long text keeps its head and its tail", () => {
    const text = `${"a".repeat(10_000)}MIDDLE${"z".repeat(10_000)}`;
    const out = excerpt(text, 8000);
    expect(out.startsWith("a".repeat(4800))).toBe(true);
    expect(out.endsWith("z".repeat(3200))).toBe(true);
    expect(out).not.toContain("MIDDLE");
    expect(out).toContain("characters omitted");
    expect(out.length).toBeLessThan(8100);
  });

  test("the excerpt length follows the context window", () => {
    expect(excerptLimitFor(200_000)).toBe(TOOL_EXCERPT_CHARS);
    expect(excerptLimitFor(40_000)).toBe(TOOL_EXCERPT_CHARS);
    expect(excerptLimitFor(8_192)).toBe(1638);
    expect(excerptLimitFor(1_000)).toBe(1200);
    expect(excerptLimitFor(0)).toBe(TOOL_EXCERPT_CHARS);
  });
});

describe("history records", () => {
  test("each message becomes one JSON record", () => {
    const messages = [
      { role: "user", content: [{ type: "text", text: "find my notes" }] },
      {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "use search" },
          { type: "text", text: "Searching." },
          { type: "toolCall", id: "1", name: "bash", arguments: { command: "curl localhost" } },
        ],
      },
      { role: "toolResult", toolCallId: "1", toolName: "bash", content: [{ type: "text", text: "x".repeat(20_000) }], isError: false },
      { role: "toolResult", toolCallId: "2", toolName: "read", content: [{ type: "text", text: "nope" }], isError: true },
    ];
    const lines = serializeConversationRecords(messages, 8000).split("\n");
    expect(lines).toHaveLength(5);
    expect(lines[0]).toContain("JSON record");
    expect(JSON.parse(lines[1])).toEqual({ role: "user", text: "find my notes" });
    const assistant = JSON.parse(lines[2]);
    expect(assistant.thinking).toBe("use search");
    expect(assistant.text).toBe("Searching.");
    expect(assistant.tool_calls).toEqual([{ tool: "bash", arguments: { command: "curl localhost" } }]);
    const result = JSON.parse(lines[3]);
    expect(result.tool).toBe("bash");
    expect(result.output.length).toBeLessThan(8100);
    expect(JSON.parse(lines[4]).is_error).toBe(true);
  });

  test("huge tool-call arguments are shortened too", () => {
    const rec = messageRecord(
      { role: "assistant", content: [{ type: "toolCall", name: "write", arguments: { path: "a", content: "y".repeat(50_000) } }] },
      2000,
    );
    expect(rec.tool_calls[0].arguments.path).toBe("a");
    expect(rec.tool_calls[0].arguments.content.length).toBeLessThan(2100);
  });

  test("empty and unknown messages are skipped", () => {
    expect(messageRecord({ role: "user", content: [] })).toBeUndefined();
    expect(messageRecord({ role: "custom", content: "x" })).toBeUndefined();
    expect(messageRecord(null)).toBeUndefined();
  });
});

describe("summary validation", () => {
  const good = {
    stopReason: "stop",
    content: [{ type: "text", text: "## Goal\nShip it\n\n## Progress\n### Done\n- [x] a\n\n## Next Steps\n1. b\n" }],
  };

  test("a complete summary passes", () => {
    expect(summaryProblems(good)).toEqual([]);
    expect(() => checkSummary(good)).not.toThrow();
  });

  test("a truncated summary is rejected", () => {
    expect(() => checkSummary({ ...good, stopReason: "length" })).toThrow(/cut off/);
  });

  test("an empty summary is rejected", () => {
    expect(() => checkSummary({ stopReason: "stop", content: [{ type: "text", text: " " }] })).toThrow(/empty/);
  });

  test("a summary without its sections is rejected", () => {
    const loose = { stopReason: "stop", content: [{ type: "text", text: "The user asked about notes and we searched for a while." }] };
    expect(() => checkSummary(loose)).toThrow(/Goal, Progress, Next Steps/);
    // A turn-prefix summary has its own headings.
    expect(() => checkSummary(loose, "turn-prefix")).toThrow(/Original Request, Early Progress/);
    const prefix = {
      stopReason: "stop",
      content: [{ type: "text", text: "## Original Request\nPrint data\n\n## Early Progress\n- ran bash\n\n## Context for Suffix\n- output is x" }],
    };
    expect(() => checkSummary(prefix, "turn-prefix")).not.toThrow();
  });
});

describe("budget sizing", () => {
  test("large windows keep the configured budgets", () => {
    expect(sizeCompactionSettings(DEFAULTS, 200_000, 12_000)).toEqual(DEFAULTS);
  });

  test("small windows get budgets that fit after compaction", () => {
    const sized = sizeCompactionSettings(DEFAULTS, 8_192, 3_000);
    expect(sized.reserveTokens).toBe(1298);
    expect(sized.keepRecentTokens).toBe(1557);
    // overhead + summary (0.8 x reserve) + kept messages stays below 70%.
    expect(3_000 + 0.8 * sized.reserveTokens + sized.keepRecentTokens).toBeLessThan(8_192 * 0.7);
  });

  test("an overhead larger than the window cannot zero the budgets", () => {
    const sized = sizeCompactionSettings(DEFAULTS, 8_192, 50_000);
    expect(sized.reserveTokens).toBeGreaterThanOrEqual(1024);
    expect(sized.keepRecentTokens).toBeGreaterThanOrEqual(1024);
  });

  test("user settings below the sized values are kept", () => {
    const tight = { enabled: true, reserveTokens: 500, keepRecentTokens: 600 };
    expect(sizeCompactionSettings(tight, 8_192, 1_000)).toEqual(tight);
  });

  test("overhead comes from the last usage figures", () => {
    const est = () => 100;
    const messages = [
      { role: "user", content: "q" },
      { role: "assistant", content: [], usage: { input: 2500, cacheRead: 500, cacheWrite: 0 }, stopReason: "toolUse" },
      { role: "toolResult", content: [] },
    ];
    // 3000 prompt tokens, one 100-token message before it.
    expect(observedOverheadTokens(messages, "", [], est)).toBe(2900);
    // Without usage: system prompt and tools, chars / 4.
    expect(observedOverheadTokens([{ role: "user", content: "q" }], "s".repeat(400), [], est)).toBe(101);
  });

  test("the session hook falls back to the configured values", () => {
    expect(settingsForSession({}, DEFAULTS)).toBe(DEFAULTS);
    expect(settingsForSession({ model: { contextWindow: 8192 }, agent: { state: { messages: [], systemPrompt: "x", tools: [] } } }, DEFAULTS).keepRecentTokens).toBeLessThan(20_000);
  });
});

describe("proactive compaction", () => {
  function fakeSession({ window = 10_000, compactionWorks = true, failWith, shrinkTo = 1_000 } = {}) {
    const branch: any[] = [];
    const session: any = {
      model: { contextWindow: window },
      isCompacting: false,
      settingsManager: { getCompactionSettings: () => DEFAULTS },
      sessionManager: { getBranch: () => branch },
      agent: { state: { messages: [] as any[] } },
      calls: 0,
      async _runAutoCompaction(reason: string) {
        expect(reason).toBe("threshold");
        session.calls++;
        if (failWith) {
          noteCompactionFailure(session, failWith);
          return false;
        }
        if (compactionWorks) {
          branch.push({ type: "compaction", id: `c${session.calls}` });
          session.agent.state.messages = [{ role: "compactionSummary", size: shrinkTo }];
        }
        return false;
      },
    };
    return session;
  }
  const helpers = (tokens: number) => ({
    estimateContextTokens: () => ({ tokens }),
    estimateTokens: (m: any) => m.size ?? 10,
  });
  const toolTurn = { toolResults: [{ role: "toolResult" }] };
  const context = { systemPrompt: "s", messages: [{ role: "user" }], tools: [] };

  test("below the threshold nothing happens", async () => {
    const s = fakeSession();
    expect(await afterToolStep(s, toolTurn, context, helpers(6_000))).toBe(context);
    expect(s.calls).toBe(0);
    expect(overThreshold(7_001, 10_000)).toBe(true);
    expect(overThreshold(7_000, 10_000)).toBe(false);
  });

  test("steps without tool results are left to Pi", async () => {
    const s = fakeSession();
    expect(await afterToolStep(s, { toolResults: [] }, context, helpers(9_000))).toBe(context);
    expect(s.calls).toBe(0);
  });

  test("over the threshold the next call sees the compacted messages", async () => {
    const s = fakeSession();
    const next = await afterToolStep(s, toolTurn, context, helpers(7_500));
    expect(s.calls).toBe(1);
    expect(next.messages).toEqual([{ role: "compactionSummary", size: 1_000 }]);
    expect(next.systemPrompt).toBe("s");
  });

  test("a failed compaction stops the run", async () => {
    const s = fakeSession({ failWith: "summary rejected" });
    await expect(afterToolStep(s, toolTurn, context, helpers(9_000))).rejects.toThrow(/summary rejected/);
  });

  test("nothing to compact leaves the context as it is", async () => {
    const s = fakeSession({ compactionWorks: false });
    expect(await afterToolStep(s, toolTurn, context, helpers(9_000))).toBe(context);
  });

  test("compactions that never get below the threshold stop the run", async () => {
    const s = fakeSession({ shrinkTo: 9_000 });
    await afterToolStep(s, toolTurn, context, helpers(9_500));
    await expect(afterToolStep(s, toolTurn, context, helpers(9_500))).rejects.toThrow(/stayed above/);
  });

  test("disabled compaction is respected", async () => {
    const s = fakeSession();
    s.settingsManager.getCompactionSettings = () => ({ ...DEFAULTS, enabled: false });
    expect(await afterToolStep(s, toolTurn, context, helpers(9_900))).toBe(context);
  });

  test("an unexpected error in the decision falls back to stock behaviour", async () => {
    const s = fakeSession();
    s.settingsManager.getCompactionSettings = () => {
      throw new Error("boom");
    };
    expect(await afterToolStep(s, toolTurn, context, helpers(9_900))).toBe(context);
  });

  test("while compacting mid-run the latest step is always kept", async () => {
    const s = fakeSession();
    let seenKeep = 0;
    s._runAutoCompaction = async () => {
      seenKeep = settingsForSession(s, DEFAULTS, { estimateTokens: (m: any) => m.size ?? 10 }).keepRecentTokens;
      return false;
    };
    s.model.contextWindow = 8_192;
    const big = {
      systemPrompt: "s",
      tools: [],
      messages: [
        { role: "user", size: 100 },
        { role: "assistant", size: 50 },
        { role: "toolResult", size: 5_000 },
      ],
    };
    await afterToolStep(s, toolTurn, big, { estimateContextTokens: () => ({ tokens: 7_000 }), estimateTokens: (m: any) => m.size ?? 10 });
    // The sized budget for 8K would be ~1.5K; the step needs 5,050 plus margin.
    expect(seenKeep).toBeGreaterThanOrEqual(5_050);
    // Outside a proactive compaction the sized budget applies again.
    expect(settingsForSession(s, DEFAULTS, { estimateTokens: () => 10 }).keepRecentTokens).toBeLessThan(5_050);
  });

  test("the latest step starts at the last assistant message", () => {
    const est = (m: any) => m.size;
    expect(latestStepTokens([{ role: "user", size: 1 }, { role: "assistant", size: 2 }, { role: "toolResult", size: 3 }], est)).toBe(5);
    expect(latestStepTokens([{ role: "user", size: 4 }], est)).toBe(0);
    expect(withMinimumKeep(DEFAULTS, 100)).toBe(DEFAULTS);
    expect(withMinimumKeep(DEFAULTS, 25_000).keepRecentTokens).toBe(25_000);
  });

  test("per-message estimates ignore usage figures", () => {
    expect(estimateMessagesTokens([{ size: 5 }, { size: 7 }], (m: any) => m.size)).toBe(12);
  });
});
