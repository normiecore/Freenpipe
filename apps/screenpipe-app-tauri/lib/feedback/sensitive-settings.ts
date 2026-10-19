// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Masks secrets and personal values in a settings object before it is
 * written into a diagnostics bundle.
 *
 * The rule itself (which key names count as sensitive) lives in
 * `sensitive-keys.json` so the Rust bundle writer applies exactly the same
 * rule. Rust masks again on its side, so this helper is a convenience for
 * the UI, never the only line of defence.
 */

import rule from "./sensitive-keys.json";

export const MASKED_VALUE = "[redacted]";

type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };

/** Lower-case a key and keep only letters and digits (`api_key` -> `apikey`). */
export function normalizeSettingsKey(key: string): string {
  return key.toLowerCase().replace(/[^a-z0-9]/g, "");
}

const FRAGMENTS: string[] = rule.fragments.map(normalizeSettingsKey);
const SUFFIXES: string[] = rule.suffixes.map(normalizeSettingsKey);
const EXCEPTIONS = new Set<string>(rule.exceptions.map(normalizeSettingsKey));

/** True when values stored under `key` must not leave the machine as-is. */
export function isSensitiveSettingsKey(key: string): boolean {
  const normalized = normalizeSettingsKey(key);
  if (normalized.length === 0 || EXCEPTIONS.has(normalized)) return false;
  if (FRAGMENTS.some((fragment) => normalized.includes(fragment))) return true;
  return SUFFIXES.some((suffix) => normalized.endsWith(suffix));
}

/**
 * Replacement for a value found under a sensitive key. Booleans and nulls
 * carry no personal data and are kept so "is it on?" stays answerable;
 * empty strings and containers are kept so "is it set?" stays answerable.
 */
function maskValue(value: Json): Json {
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "string") return value.length === 0 ? "" : MASKED_VALUE;
  if (typeof value === "number") return MASKED_VALUE;
  if (Array.isArray(value)) {
    return value.length === 0 ? [] : `[redacted: ${value.length} items]`;
  }
  const size = Object.keys(value).length;
  return size === 0 ? {} : `[redacted: ${size} keys]`;
}

/**
 * Deep copy of `value` with every sensitive key masked. Non-JSON values
 * (functions, undefined, class instances) are dropped the same way
 * `JSON.stringify` would drop them.
 */
export function maskSensitiveSettings<T>(value: T): Json {
  const plain = JSON.parse(JSON.stringify(value ?? null)) as Json;
  return walk(plain);
}

function walk(value: Json): Json {
  if (Array.isArray(value)) return value.map(walk);
  if (value !== null && typeof value === "object") {
    const out: { [key: string]: Json } = {};
    for (const [key, child] of Object.entries(value)) {
      out[key] = isSensitiveSettingsKey(key) ? maskValue(child) : walk(child);
    }
    return out;
  }
  return value;
}
