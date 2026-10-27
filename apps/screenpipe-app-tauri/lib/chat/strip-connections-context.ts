// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * The chat runtime puts a fresh `<connections_context>` block in front of
 * every user turn it hands to the agent (the list of connected integrations,
 * rebuilt per turn; see `src-tauri/src/pi/connections_context.rs`). The agent
 * needs it, people do not: wherever the chat shows a user message, or turns
 * Pi's echo of a turn back into one, the block is removed with
 * `stripConnectionsContext`. Pi's own session log keeps it.
 *
 * Only exact, well-formed blocks are removed. A closing tag pairs with the
 * nearest unpaired opening tag before it; nested blocks go as a whole; an
 * opening tag that never closes and a closing tag with nothing to close are
 * left in place, so text a user typed is never hidden. The Rust side uses
 * the same rules.
 */

export const CONNECTIONS_CONTEXT_OPEN = "<connections_context>";
export const CONNECTIONS_CONTEXT_CLOSE = "</connections_context>";

function tagPositions(text: string, tag: string): number[] {
  const positions: number[] = [];
  let from = 0;
  for (;;) {
    const at = text.indexOf(tag, from);
    if (at === -1) return positions;
    positions.push(at);
    from = at + tag.length;
  }
}

/** Start/end offsets of the outermost well-formed blocks, in order. */
function blockRanges(text: string): Array<[number, number]> {
  const tags = [
    ...tagPositions(text, CONNECTIONS_CONTEXT_OPEN).map((at) => ({ at, open: true })),
    ...tagPositions(text, CONNECTIONS_CONTEXT_CLOSE).map((at) => ({ at, open: false })),
  ].sort((a, b) => a.at - b.at);

  const openAt: number[] = [];
  const pairs: Array<[number, number]> = [];
  for (const tag of tags) {
    if (tag.open) {
      openAt.push(tag.at);
    } else if (openAt.length > 0) {
      const start = openAt.pop()!;
      pairs.push([start, tag.at + CONNECTIONS_CONTEXT_CLOSE.length]);
    }
  }

  pairs.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const outer: Array<[number, number]> = [];
  for (const pair of pairs) {
    const last = outer[outer.length - 1];
    if (last && pair[0] < last[1]) {
      last[1] = Math.max(last[1], pair[1]);
    } else {
      outer.push([pair[0], pair[1]]);
    }
  }
  return outer;
}

/** True when `text` holds at least one well-formed block. */
export function hasConnectionsContext(text: string | null | undefined): boolean {
  return typeof text === "string" && blockRanges(text).length > 0;
}

/**
 * `text` without its `<connections_context>` blocks. The blank space after a
 * removed block goes with it, and so does space left over at either end. A
 * text without a block comes back unchanged.
 */
export function stripConnectionsContext(text: string): string;
export function stripConnectionsContext(text: string | null | undefined): string | null | undefined;
export function stripConnectionsContext(
  text: string | null | undefined,
): string | null | undefined {
  if (typeof text !== "string") return text;
  const ranges = blockRanges(text);
  if (ranges.length === 0) return text;

  const firstVisible = text.length - text.trimStart().length;
  let out = "";
  let cursor = 0;
  let trimFront = false;
  let trimBack = false;
  for (const [start, end] of ranges) {
    out += text.slice(cursor, start);
    const tail = text.slice(end);
    const after = end + (tail.length - tail.trimStart().length);
    if (start <= firstVisible) trimFront = true;
    if (after === text.length) trimBack = true;
    cursor = after;
  }
  out += text.slice(cursor);
  if (trimFront) out = out.trimStart();
  if (trimBack) out = out.trimEnd();
  return out;
}
