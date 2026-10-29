// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import {
  createDiagnosticsBundle,
  formatBundleSize,
  type DiagnosticsDeps,
  type FeedbackUploadCompleted,
} from "./diagnostics-bundle";

function completed(jobId: string, extra: Partial<FeedbackUploadCompleted> = {}): FeedbackUploadCompleted {
  return {
    jobId,
    sent: false,
    path: `/data/feedback/${jobId}.zip`,
    fileName: `${jobId}.zip`,
    sizeBytes: 2048,
    issueUrl: "https://github.com/normiecore/Freenpipe/issues/new?title=x",
    error: null,
    ...extra,
  };
}

/** Fake Tauri: `emit` delivers to the subscribed handler. */
function fakeDeps(onStart: (emit: (p: FeedbackUploadCompleted) => void) => Promise<unknown>) {
  let handler: ((p: FeedbackUploadCompleted) => void) | null = null;
  let unlistened = 0;
  const emit = (p: FeedbackUploadCompleted) => handler?.(p);
  const deps: DiagnosticsDeps = {
    subscribe: async (h) => {
      handler = h;
      return () => {
        unlistened += 1;
        handler = null;
      };
    },
    start: async () => (await onStart(emit)) as Awaited<ReturnType<DiagnosticsDeps["start"]>>,
  };
  return { deps, emit, unlistenCount: () => unlistened };
}

describe("createDiagnosticsBundle", () => {
  it("resolves with the completion for its own job, ignoring others", async () => {
    const fake = fakeDeps(async (emit) => {
      setTimeout(() => {
        emit(completed("other-job"));
        emit(completed("job-1"));
      }, 5);
      return { status: "ok", data: { jobId: "job-1" } };
    });
    const result = await createDiagnosticsBundle({ feedbackText: "hi" }, { deps: fake.deps });
    expect(result.jobId).toBe("job-1");
    expect(result.sent).toBe(false);
    expect(result.path).toBe("/data/feedback/job-1.zip");
    expect(fake.unlistenCount()).toBe(1);
  });

  it("catches a completion that arrives before the job id is known", async () => {
    const fake = fakeDeps(async (emit) => {
      emit(completed("fast-job"));
      return { status: "ok", data: { jobId: "fast-job" } };
    });
    const result = await createDiagnosticsBundle({}, { deps: fake.deps });
    expect(result.jobId).toBe("fast-job");
  });

  it("passes failures through as a result with an error", async () => {
    const fake = fakeDeps(async (emit) => {
      setTimeout(() => emit(completed("job-2", { path: null, error: "disk full" })), 1);
      return { status: "ok", data: { jobId: "job-2" } };
    });
    const result = await createDiagnosticsBundle({}, { deps: fake.deps });
    expect(result.error).toBe("disk full");
    expect(result.path).toBeNull();
  });

  it("rejects when the job cannot start, and stops listening", async () => {
    const fake = fakeDeps(async () => ({ status: "error", error: "already running" }));
    await expect(createDiagnosticsBundle({}, { deps: fake.deps })).rejects.toThrow("already running");
    expect(fake.unlistenCount()).toBe(1);
  });

  it("rejects after the timeout when no result arrives", async () => {
    const fake = fakeDeps(async () => ({ status: "ok", data: { jobId: "slow" } }));
    await expect(
      createDiagnosticsBundle({}, { deps: fake.deps, timeoutMs: 10 })
    ).rejects.toThrow("timed out");
    expect(fake.unlistenCount()).toBe(1);
  });
});

describe("formatBundleSize", () => {
  it("formats bytes, kilobytes and megabytes", () => {
    expect(formatBundleSize(512)).toBe("512 B");
    expect(formatBundleSize(2048)).toBe("2 KB");
    expect(formatBundleSize(5 * 1024 * 1024 + 100)).toBe("5.0 MB");
    expect(formatBundleSize(null)).toBe("");
  });
});
