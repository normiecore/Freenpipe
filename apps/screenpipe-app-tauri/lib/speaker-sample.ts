// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Voice-sample playback helpers for the speakers settings page.
 *
 * The engine serves short WAV clips at
 * `GET /speakers/sample/:audio_chunk_id?start=<s>&end=<s>`, where both values
 * are seconds into the chunk file. Speaker metadata lists each sample's
 * `audio_chunk_id`; samples without one (older engines) fall back to playing
 * the chunk file directly.
 */

/** The engine refuses longer clips; never ask for more than this. */
export const MAX_SPEAKER_SAMPLE_SECONDS = 120;

/** Seconds rendered with at most millisecond precision, without trailing zeros. */
function formatSeconds(value: number): string {
  return String(Number(value.toFixed(3)));
}

/**
 * Build the sample-clip URL for one voice sample, or `null` when the sample
 * cannot be streamed (no chunk id, or no usable time window).
 *
 * `baseUrl` is the local API origin (for example `http://localhost:3030`).
 * The caller adds authentication, because `<audio src>` cannot send headers.
 */
export function buildSpeakerSampleUrl(
  baseUrl: string,
  audioChunkId: number | null | undefined,
  startTime: number,
  duration: number
): string | null {
  if (typeof audioChunkId !== "number" || !Number.isInteger(audioChunkId) || audioChunkId <= 0) {
    return null;
  }
  if (!Number.isFinite(startTime) || !Number.isFinite(duration) || duration <= 0) {
    return null;
  }
  const start = Math.max(0, startTime);
  const end = start + Math.min(duration, MAX_SPEAKER_SAMPLE_SECONDS);
  const origin = baseUrl.replace(/\/+$/, "");
  return `${origin}/speakers/sample/${audioChunkId}?start=${formatSeconds(start)}&end=${formatSeconds(end)}`;
}
