// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * User-visible identity of the app and the deep-link schemes it understands.
 *
 * The OS routes only `freenpipe://` (or `freenpipe-beta://` on the beta
 * channel) to this app; that list lives in the Tauri config. Links in
 * screenpipe's `screenpipe://` scheme still work once they are inside the
 * app (pipe notifications, skill-written chat answers, command-line
 * arguments), so pipes written for screenpipe keep working. Interop names
 * such as `~/.screenpipe`, `SCREENPIPE_*` and the `screenpipe` CLI are not
 * defined here and keep their upstream spelling.
 */

export const PRODUCT_NAME = "Freenpipe";

/** Scheme of links the app builds itself (copy-link actions and the like). */
export const DEEP_LINK_SCHEME = "freenpipe";

/** Every scheme the in-app link handling accepts, lowercase, primary first. */
export const ACCEPTED_DEEP_LINK_SCHEMES: readonly string[] = [
  "freenpipe",
  "freenpipe-beta",
  "screenpipe",
];

function schemeOf(url: string): string | null {
  const match = /^\s*([A-Za-z][A-Za-z0-9+.-]*):\/\//.exec(url);
  return match ? match[1].toLowerCase() : null;
}

/** True for `freenpipe://…` and the `screenpipe://…` alias. */
export function isAppDeepLink(url: unknown): url is string {
  if (typeof url !== "string") return false;
  const scheme = schemeOf(url);
  return scheme !== null && ACCEPTED_DEEP_LINK_SCHEMES.includes(scheme);
}

/** True when a parsed URL's `protocol` (`"freenpipe:"`) is an in-app scheme. */
export function isAppDeepLinkProtocol(protocol: string): boolean {
  return ACCEPTED_DEEP_LINK_SCHEMES.includes(protocol.replace(/:$/, "").toLowerCase());
}

/**
 * The part of an in-app link after `scheme://`, e.g. `meeting/12?live=1`, or
 * `null` when the URL is not an in-app link.
 */
export function deepLinkRoute(url: unknown): string | null {
  if (!isAppDeepLink(url)) return null;
  return url.trim().replace(/^[^:]+:\/\//, "");
}

/** True when an in-app link's route starts with `target` (`"frame"` matches
 *  `frame/12` and `frame?id=12`, not `frames`). */
export function isDeepLinkTo(url: unknown, target: string): boolean {
  const route = deepLinkRoute(url);
  if (route === null) return false;
  if (!route.startsWith(target)) return false;
  const next = route.charAt(target.length);
  return next === "" || next === "/" || next === "?" || next === "#";
}

/** Build an in-app link with the primary scheme: `appDeepLink("frame/42")`. */
export function appDeepLink(route: string): string {
  return `${DEEP_LINK_SCHEME}://${route.replace(/^\/+/, "")}`;
}
