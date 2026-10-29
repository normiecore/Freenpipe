// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import { MAX_SPEAKER_SAMPLE_SECONDS, buildSpeakerSampleUrl } from "../speaker-sample";

const base = "http://localhost:3035";

describe("buildSpeakerSampleUrl", () => {
  test("builds the sample route with start and end in seconds", () => {
    expect(buildSpeakerSampleUrl(base, 42, 1.5, 3)).toBe(
      "http://localhost:3035/speakers/sample/42?start=1.5&end=4.5"
    );
  });

  test("rounds to milliseconds and drops trailing zeros", () => {
    expect(buildSpeakerSampleUrl(base, 7, 0.123456, 2.0004)).toBe(
      "http://localhost:3035/speakers/sample/7?start=0.123&end=2.124"
    );
    expect(buildSpeakerSampleUrl(`${base}/`, 7, 0, 2)).toBe(
      "http://localhost:3035/speakers/sample/7?start=0&end=2"
    );
  });

  test("clamps a negative start and caps the clip length", () => {
    expect(buildSpeakerSampleUrl(base, 3, -2, 1)).toBe(
      "http://localhost:3035/speakers/sample/3?start=0&end=1"
    );
    expect(buildSpeakerSampleUrl(base, 3, 10, 500)).toBe(
      `http://localhost:3035/speakers/sample/3?start=10&end=${10 + MAX_SPEAKER_SAMPLE_SECONDS}`
    );
  });

  test("returns null when the sample cannot be streamed", () => {
    expect(buildSpeakerSampleUrl(base, undefined, 0, 2)).toBeNull();
    expect(buildSpeakerSampleUrl(base, null, 0, 2)).toBeNull();
    expect(buildSpeakerSampleUrl(base, 0, 0, 2)).toBeNull();
    expect(buildSpeakerSampleUrl(base, 1.5, 0, 2)).toBeNull();
    expect(buildSpeakerSampleUrl(base, 5, 0, 0)).toBeNull();
    expect(buildSpeakerSampleUrl(base, 5, Number.NaN, 2)).toBeNull();
    expect(buildSpeakerSampleUrl(base, 5, 0, Number.POSITIVE_INFINITY)).toBeNull();
  });
});
