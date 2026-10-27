// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Runs under both vitest and `bun test` (bun maps the vitest import).
import { describe, expect, it } from "vitest";
import {
  CONNECTIONS_CONTEXT_CLOSE as CLOSE,
  CONNECTIONS_CONTEXT_OPEN as OPEN,
  hasConnectionsContext,
  stripConnectionsContext,
} from "../strip-connections-context";
import { extractConversationHistorySyncUserText } from "../../chat-utils";

const block = (body: string) => `${OPEN}${body}${CLOSE}`;

// The shape the chat runtime sends: header, list, then the user's turn.
const runtimeBlock = block(
  "\nCurrent connected integrations, generated again for this message.\n\n## Google Calendar (google-calendar)\n  proxy: http://localhost:3030/connections/google-calendar/proxy/<api-path>\n",
);

describe("stripConnectionsContext", () => {
  it("leaves a message without a block untouched", () => {
    for (const text of [
      "",
      "hello",
      "  spaced out  \n",
      "<conversation_history>\nuser: hi\n</conversation_history>\n\nhello",
      "what is connections_context anyway?",
    ]) {
      expect(stripConnectionsContext(text)).toBe(text);
      expect(hasConnectionsContext(text)).toBe(false);
    }
    expect(stripConnectionsContext(null)).toBeNull();
    expect(stripConnectionsContext(undefined)).toBeUndefined();
  });

  it("removes the block the runtime puts in front of a turn", () => {
    const sent = `${runtimeBlock}\n\nwhat's on my calendar today?`;
    expect(hasConnectionsContext(sent)).toBe(true);
    expect(stripConnectionsContext(sent)).toBe("what's on my calendar today?");
    expect(stripConnectionsContext(`${block("x")}\r\n\r\nhi`)).toBe("hi");
  });

  it("removes a block in the middle and keeps the text around it", () => {
    expect(stripConnectionsContext(`before\n\n${block("list")}\n\nafter`)).toBe(
      "before\n\nafter",
    );
    expect(stripConnectionsContext(`a ${block("list")} b`)).toBe("a b");
  });

  it("removes trailing and repeated blocks", () => {
    expect(stripConnectionsContext(`hello\n\n${block("x")}`)).toBe("hello");
    expect(
      stripConnectionsContext(`${block("a")}\n\nfirst\n\n${block("b")}\n\nsecond`),
    ).toBe("first\n\nsecond");
    expect(stripConnectionsContext(block("only"))).toBe("");
  });

  it("removes nested blocks as a whole", () => {
    const nested = `${OPEN}outer ${OPEN}inner${CLOSE} rest${CLOSE}\n\nhi`;
    expect(stripConnectionsContext(nested)).toBe("hi");
  });

  it("keeps malformed tags as the user's own text", () => {
    const unclosed = `${OPEN} please keep this`;
    expect(stripConnectionsContext(unclosed)).toBe(unclosed);
    expect(hasConnectionsContext(unclosed)).toBe(false);

    expect(stripConnectionsContext(`${CLOSE} kept ${block("gone")}`)).toBe(`${CLOSE} kept`);
    expect(stripConnectionsContext(`${OPEN} a ${block("b")} c`)).toBe(`${OPEN} a c`);

    const otherSpelling = "<connections_context attr=1>x</connections_context >";
    expect(stripConnectionsContext(otherSpelling)).toBe(otherSpelling);
    const reversed = `${CLOSE}x${OPEN}`;
    expect(stripConnectionsContext(reversed)).toBe(reversed);
  });

  it("gives back the typed text of an echoed turn with history", () => {
    // The chat sends history + text; the runtime adds the block in front.
    const echoed = `${runtimeBlock}\n\n<conversation_history>\nuser: earlier\nassistant: reply\n</conversation_history>\n\nand now?`;
    const visible = stripConnectionsContext(echoed);
    expect(extractConversationHistorySyncUserText(visible) ?? visible).toBe("and now?");
  });
});
