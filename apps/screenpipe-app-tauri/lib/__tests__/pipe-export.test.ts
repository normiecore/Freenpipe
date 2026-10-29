// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import { buildPipeExport, pipeExportFileName, redactPipeSecrets } from "../pipe-export";

describe("pipeExportFileName", () => {
  it("names the file after the pipe so installing it keeps the name", () => {
    expect(pipeExportFileName("daily-summary")).toBe("daily-summary.md");
    expect(pipeExportFileName("  notes  ")).toBe("notes.md");
    expect(pipeExportFileName("already.md")).toBe("already.md");
  });

  it("replaces characters Windows rejects and never yields an empty name", () => {
    expect(pipeExportFileName('a/b\\c:d*e?f"g<h>i|j')).toBe("a-b-c-d-e-f-g-h-i-j.md");
    expect(pipeExportFileName("...")).toBe("pipe.md");
    expect(pipeExportFileName("")).toBe("pipe.md");
    expect(pipeExportFileName("trailing.")).toBe("trailing.md");
  });

  it("avoids reserved Windows device names", () => {
    expect(pipeExportFileName("con")).toBe("pipe-con.md");
    expect(pipeExportFileName("LPT1")).toBe("pipe-LPT1.md");
    expect(pipeExportFileName("console")).toBe("console.md");
  });
});

describe("redactPipeSecrets", () => {
  it("masks credential-looking values and counts each one", () => {
    const text = [
      "use key sk-abcdefghijklmnopqrstuvwxyz123456",
      "and sk-ant-abcdefghijklmnopqrstuvwxyz",
      "OPENAI_API_KEY=abcd1234efgh5678",
      'password: "hunter2hunter2"',
      "Authorization: Bearer abcdefghijklmnop1234",
    ].join("\n");
    const { redacted, count } = redactPipeSecrets(text);
    expect(redacted).not.toContain("abcdefghijklmnopqrstuvwxyz123456");
    expect(redacted).not.toContain("sk-ant-abcdefghijklmnopqrstuvwxyz");
    expect(redacted).toContain("OPENAI_API_KEY=***REDACTED***");
    expect(redacted).toContain('password: "***REDACTED***"');
    expect(redacted).toContain("Bearer ***REDACTED***");
    expect(count).toBe(5);
  });

  it("leaves ordinary pipe text alone", () => {
    const text = "---\nschedule: every 1h\n---\n\nsummarize my day in three bullet points\n";
    expect(redactPipeSecrets(text)).toEqual({ redacted: text, count: 0 });
  });
});

describe("buildPipeExport", () => {
  it("returns the file name, masked content and a trailing newline", () => {
    const result = buildPipeExport("standup", "---\nschedule: manual\n---\nTOKEN=abcdefgh12345678");
    expect(result.fileName).toBe("standup.md");
    expect(result.redactedCount).toBe(1);
    expect(result.content.endsWith("\n")).toBe(true);
    expect(result.content).toContain("TOKEN=***REDACTED***");
  });
});
