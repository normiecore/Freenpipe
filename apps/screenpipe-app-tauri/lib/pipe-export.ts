// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Exporting a pipe as one shareable file.
 *
 * There is no hosted pipe store to publish or share through, so a pipe is
 * shared as its pipe.md. The exported file is named `<pipe>.md`, which is
 * what `screenpipe pipe install <file>` (and the install dialog) turn back
 * into a pipe of the same name. Credentials that look like API keys or
 * tokens are masked first, because the file is meant to leave the machine.
 */

/** Characters Windows does not allow in file names, plus path separators. */
const UNSAFE_FILE_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g;

/** Device names Windows reserves regardless of extension. */
const RESERVED_WINDOWS_NAMES = /^(con|prn|aux|nul|com\d|lpt\d)$/i;

/** File name for an exported pipe: `<name>.md`, safe on every platform. */
export function pipeExportFileName(pipeName: string): string {
  let base = pipeName
    .trim()
    .replace(UNSAFE_FILE_CHARS, "-")
    .replace(/^[.\s]+|[.\s]+$/g, "");
  if (base.toLowerCase().endsWith(".md")) base = base.slice(0, -3);
  if (!base) base = "pipe";
  if (RESERVED_WINDOWS_NAMES.test(base)) base = `pipe-${base}`;
  return `${base}.md`;
}

/** Credential shapes that must not travel inside a shared pipe file. */
const SECRET_PATTERNS: Array<[RegExp, string]> = [
  [/sk-ant-[A-Za-z0-9_-]{20,}/g, "sk-ant-***REDACTED***"],
  [/sk-[A-Za-z0-9]{20,}/g, "sk-***REDACTED***"],
  [/gh[pousr]_[A-Za-z0-9]{36,}/g, "gh*_***REDACTED***"],
  [/xox[abpr]-[A-Za-z0-9-]+/g, "xox*-***REDACTED***"],
  [/AIza[A-Za-z0-9_-]{30,}/g, "AIza***REDACTED***"],
  [/AKIA[A-Z0-9]{16,}/g, "AKIA***REDACTED***"],
  [/Bearer\s+[A-Za-z0-9._~+/-]{16,}=*/g, "Bearer ***REDACTED***"],
  [
    /((?:API_KEY|SECRET|TOKEN|PASSWORD|CREDENTIALS)\s*[=:]\s*["']?)([A-Za-z0-9_-]{8,})(["']?)/gi,
    "$1***REDACTED***$3",
  ],
];

/** Mask credential-looking strings; `count` is the number of masked values. */
export function redactPipeSecrets(text: string): { redacted: string; count: number } {
  let count = 0;
  let redacted = text;
  for (const [pattern, replacement] of SECRET_PATTERNS) {
    redacted = redacted.replace(pattern, (...args: any[]) => {
      count += 1;
      // Expand $1/$3 style references against this match's groups.
      return replacement.replace(/\$(\d)/g, (_, n) => args[Number(n)] ?? "");
    });
  }
  return { redacted, count };
}

export interface PipeExport {
  fileName: string;
  content: string;
  /** How many credential-looking values were masked. */
  redactedCount: number;
}

/** Everything needed to write a pipe export file. */
export function buildPipeExport(pipeName: string, rawContent: string): PipeExport {
  const { redacted, count } = redactPipeSecrets(rawContent);
  const content = redacted.endsWith("\n") ? redacted : `${redacted}\n`;
  return { fileName: pipeExportFileName(pipeName), content, redactedCount: count };
}
