// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * True for deep links screenpipe's website used to hand an account or a
 * purchase back to the app: a top-level `api_key` query parameter (sign-in),
 * and the `subscription-success` / `purchase-successful` targets.
 *
 * Handvel has no hosted account or plan, so the deep-link handler drops
 * these without reading their tokens. Only the link's own query string is
 * checked: a nested URL that merely contains `api_key=` (for example inside
 * an install-pipe `url=` parameter) is not an account link.
 */
export function isAccountDeepLink(url: URL): boolean {
  if (url.searchParams.has("api_key")) return true;
  const target = `${url.host}${url.pathname}`.toLowerCase();
  return (
    target.includes("subscription-success") ||
    target.includes("purchase-successful")
  );
}
