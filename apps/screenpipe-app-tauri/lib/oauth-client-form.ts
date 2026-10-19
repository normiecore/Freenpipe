// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Pure helpers behind the "OAuth app" dialog in Settings > Connections.
 *
 * Every native OAuth integration runs on an OAuth app the user registers with
 * the provider. The backend (`oauth_client_*` Tauri commands) stores it in the
 * SecretStore and is the authority on validation; these helpers give instant
 * feedback in the form and translate it into the command input. They never
 * see a stored secret: the backend only reports whether one exists.
 */

/** Mirror of the backend's `OAuthClientInfo` (only the fields the form uses). */
export interface OAuthClientInfoLike {
  integration_id: string;
  configured: boolean;
  client_id: string | null;
  has_client_secret: boolean;
  redirect_uri: string | null;
  default_redirect_uri: string;
  requires_https_redirect: boolean;
  uses_pkce: boolean;
}

/** Mirror of the backend's `OAuthClientInput`. */
export interface OAuthClientInputLike {
  client_id: string;
  /** `null` keeps the stored secret, `""` removes it, other values replace it. */
  client_secret: string | null;
  /** `null` means "use the loopback default". */
  redirect_uri: string | null;
}

export interface OAuthClientFormState {
  clientId: string;
  /** Typed secret. Empty means "keep what is stored" unless `clearSecret`. */
  clientSecret: string;
  clearSecret: boolean;
  redirectUri: string;
}

export type OAuthClientFormErrors = Partial<
  Record<"clientId" | "clientSecret" | "redirectUri", string>
>;

/** Initial form values for an existing (or missing) OAuth app. */
export function initialOAuthClientForm(
  info: OAuthClientInfoLike | null,
): OAuthClientFormState {
  return {
    clientId: info?.client_id ?? "",
    clientSecret: "",
    clearSecret: false,
    redirectUri: info?.redirect_uri ?? "",
  };
}

function hasBadCharacters(value: string): boolean {
  // Whitespace inside an id/secret is always a paste accident; control
  // characters never belong in either.
  return /[\s\u0000-\u001f\u007f]/.test(value);
}

/** True for `http://localhost`, `http://127.0.0.1` and `http://[::1]`. */
export function isLoopbackHttpUrl(url: URL): boolean {
  if (url.protocol !== "http:") return false;
  const host = url.hostname.toLowerCase();
  return (
    host === "localhost" ||
    host === "[::1]" ||
    host === "::1" ||
    /^127(\.\d{1,3}){3}$/.test(host)
  );
}

/** `null` when the redirect is acceptable, otherwise a short reason. */
export function redirectUriProblem(value: string): string | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return "enter a full URL, starting with https://";
  }
  if (url.hash) return "the redirect URI cannot contain a #fragment";
  if (url.username || url.password) return "the redirect URI cannot contain credentials";
  if (url.protocol === "https:" && url.hostname) return null;
  if (isLoopbackHttpUrl(url)) return null;
  return "use https://, or http:// only on localhost";
}

/** Client-side validation; the backend repeats every check. */
export function validateOAuthClientForm(
  form: OAuthClientFormState,
  info: OAuthClientInfoLike | null,
): OAuthClientFormErrors {
  const errors: OAuthClientFormErrors = {};
  const clientId = form.clientId.trim();
  if (!clientId) {
    errors.clientId = "required";
  } else if (hasBadCharacters(clientId)) {
    errors.clientId = "remove spaces and line breaks";
  }

  const secret = form.clientSecret.trim();
  if (secret && hasBadCharacters(secret)) {
    errors.clientSecret = "remove spaces and line breaks";
  }

  const redirect = form.redirectUri.trim();
  if (redirect) {
    const problem = redirectUriProblem(redirect);
    if (problem) errors.redirectUri = problem;
  } else if (info?.requires_https_redirect) {
    errors.redirectUri = "this provider needs an HTTPS redirect you control";
  }
  return errors;
}

export function hasFormErrors(errors: OAuthClientFormErrors): boolean {
  return Object.values(errors).some(Boolean);
}

/** Translate the form into the `oauth_client_save` input. */
export function toOAuthClientInput(form: OAuthClientFormState): OAuthClientInputLike {
  const secret = form.clientSecret.trim();
  const redirect = form.redirectUri.trim();
  return {
    client_id: form.clientId.trim(),
    client_secret: form.clearSecret ? "" : secret ? secret : null,
    redirect_uri: redirect ? redirect : null,
  };
}

/** The redirect URI the user must register with the provider. */
export function redirectToRegister(info: OAuthClientInfoLike): string | null {
  if (info.redirect_uri) return info.redirect_uri;
  return info.requires_https_redirect ? null : info.default_redirect_uri;
}

/** Why "connect" is not possible yet, or `null` when it is. */
export function connectBlocker(info: OAuthClientInfoLike | null): string | null {
  if (!info) return "loading";
  if (!info.configured) return "add your OAuth app first";
  if (info.requires_https_redirect && !info.redirect_uri) {
    return "add an HTTPS redirect for this provider";
  }
  return null;
}

/**
 * Where to create an OAuth app for each provider. Opening one is a
 * user-initiated visit to the provider's own developer console.
 */
const DEVELOPER_CONSOLES: Record<string, string> = {
  "google-calendar": "https://console.cloud.google.com/apis/credentials",
  "google-docs": "https://console.cloud.google.com/apis/credentials",
  "google-sheets": "https://console.cloud.google.com/apis/credentials",
  gmail: "https://console.cloud.google.com/apis/credentials",
  microsoft365: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps",
  teams: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps",
  notion: "https://www.notion.so/profile/integrations",
  github: "https://github.com/settings/developers",
  slack: "https://api.slack.com/apps",
  zoom: "https://marketplace.zoom.us/develop/create",
  jira: "https://developer.atlassian.com/console/myapps/",
  calcom: "https://app.cal.com/settings/developer",
  calendly: "https://developer.calendly.com/",
  quickbooks: "https://developer.intuit.com/app/developer/dashboard",
  vercel: "https://vercel.com/dashboard/integrations/console",
  supabase: "https://supabase.com/dashboard/org/_/apps",
};

export function developerConsoleUrl(integrationId: string): string | null {
  return DEVELOPER_CONSOLES[integrationId] ?? null;
}
