// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Frontend side of the local diagnostics bundle.
 *
 * `start_feedback_upload` (Rust, src-tauri/src/feedback_upload.rs) writes a
 * scrubbed zip under `<data dir>/feedback/` in the background and reports
 * back through the `feedback-upload-completed` event. Nothing is uploaded:
 * the result carries the zip path and a prefilled GitHub issue link that
 * the user may open and attach the zip to.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { commands, type FeedbackUploadRequest } from "@/lib/utils/tauri";

export const FEEDBACK_COMPLETED_EVENT = "feedback-upload-completed";

/** Payload of `feedback-upload-completed` (see FeedbackUploadCompleted in Rust). */
export interface FeedbackUploadCompleted {
  jobId: string;
  /** Always false: the bundle stays on this computer. */
  sent: boolean;
  path: string | null;
  fileName: string | null;
  sizeBytes: number | null;
  issueUrl: string | null;
  error: string | null;
}

export type DiagnosticsRequest = FeedbackUploadRequest;

type StartResult =
  | { status: "ok"; data: { jobId: string } }
  | { status: "error"; error: string };

/** Seams so the waiting logic can be tested without Tauri. */
export interface DiagnosticsDeps {
  start: (request: DiagnosticsRequest) => Promise<StartResult>;
  subscribe: (
    handler: (payload: FeedbackUploadCompleted) => void
  ) => Promise<UnlistenFn>;
}

const tauriDeps: DiagnosticsDeps = {
  start: (request) => commands.startFeedbackUpload(request),
  subscribe: (handler) =>
    listen<FeedbackUploadCompleted>(FEEDBACK_COMPLETED_EVENT, (event) =>
      handler(event.payload)
    ),
};

/** Bundles with logs and dumps can take a while; give up waiting after this. */
export const DEFAULT_BUNDLE_TIMEOUT_MS = 3 * 60 * 1000;

/**
 * Write a diagnostics bundle and wait for its result. Resolves with the
 * completion payload (check `error`); rejects only when the job could not
 * be started or no result arrived in time.
 *
 * The listener is registered before the job starts, so a fast job cannot
 * finish before anyone is listening.
 */
export async function createDiagnosticsBundle(
  request: DiagnosticsRequest,
  options: { timeoutMs?: number; deps?: DiagnosticsDeps } = {}
): Promise<FeedbackUploadCompleted> {
  const deps = options.deps ?? tauriDeps;
  const timeoutMs = options.timeoutMs ?? DEFAULT_BUNDLE_TIMEOUT_MS;

  let jobId: string | null = null;
  const early: FeedbackUploadCompleted[] = [];
  let settle: ((payload: FeedbackUploadCompleted) => void) | null = null;
  const done = new Promise<FeedbackUploadCompleted>((resolve) => {
    settle = resolve;
  });

  const unlisten = await deps.subscribe((payload) => {
    if (jobId === null) early.push(payload);
    else if (payload.jobId === jobId) settle?.(payload);
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const started = await deps.start(request);
    if (started.status !== "ok") {
      throw new Error(started.error || "could not start the diagnostics bundle");
    }
    jobId = started.data.jobId;
    const already = early.find((p) => p.jobId === jobId);
    if (already) return already;

    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(
        () => reject(new Error("timed out waiting for the diagnostics bundle")),
        timeoutMs
      );
    });
    return await Promise.race([done, timeout]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    unlisten();
  }
}

/** "1.4 MB" style size for the completion message. */
export function formatBundleSize(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Last `maxChars` characters of the console log the app keeps in localStorage. */
export function readConsoleLogTail(maxChars = 50_000): string {
  try {
    return (localStorage?.getItem("console_logs") || "").slice(-maxChars);
  } catch {
    return "";
  }
}
