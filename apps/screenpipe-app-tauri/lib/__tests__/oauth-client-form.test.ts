// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

import { describe, expect, it } from "vitest";
import {
  connectBlocker,
  developerConsoleUrl,
  hasFormErrors,
  initialOAuthClientForm,
  redirectToRegister,
  redirectUriProblem,
  toOAuthClientInput,
  validateOAuthClientForm,
  type OAuthClientInfoLike,
} from "../oauth-client-form";

const LOOPBACK = "http://localhost:3030/connections/oauth/callback";

function info(over: Partial<OAuthClientInfoLike> = {}): OAuthClientInfoLike {
  return {
    integration_id: "google-calendar",
    configured: false,
    client_id: null,
    has_client_secret: false,
    redirect_uri: null,
    default_redirect_uri: LOOPBACK,
    requires_https_redirect: false,
    uses_pkce: true,
    ...over,
  };
}

describe("initialOAuthClientForm", () => {
  it("prefills the client id and redirect but never a secret", () => {
    const form = initialOAuthClientForm(
      info({ configured: true, client_id: "abc", has_client_secret: true, redirect_uri: "https://r.example/cb" }),
    );
    expect(form).toEqual({
      clientId: "abc",
      clientSecret: "",
      clearSecret: false,
      redirectUri: "https://r.example/cb",
    });
    expect(initialOAuthClientForm(null).clientId).toBe("");
  });
});

describe("redirectUriProblem", () => {
  it("accepts https and loopback http", () => {
    for (const ok of [
      "https://relay.example.com/oauth",
      LOOPBACK,
      "http://127.0.0.1:3035/connections/oauth/callback",
      "http://[::1]:3030/connections/oauth/callback",
    ]) {
      expect(redirectUriProblem(ok)).toBeNull();
    }
  });

  it("rejects remote http, other schemes, fragments and junk", () => {
    for (const bad of [
      "http://relay.example.com/cb",
      "handvel://oauth",
      "https://relay.example.com/cb#x",
      "https://u:p@relay.example.com/cb",
      "relay.example.com/cb",
    ]) {
      expect(redirectUriProblem(bad)).not.toBeNull();
    }
  });
});

describe("validateOAuthClientForm", () => {
  it("requires a clean client id", () => {
    const base = initialOAuthClientForm(null);
    expect(validateOAuthClientForm(base, info()).clientId).toBeDefined();
    expect(validateOAuthClientForm({ ...base, clientId: "a b" }, info()).clientId).toBeDefined();
    const ok = validateOAuthClientForm({ ...base, clientId: " abc.apps.example " }, info());
    expect(hasFormErrors(ok)).toBe(false);
  });

  it("flags secrets with line breaks", () => {
    const form = { ...initialOAuthClientForm(null), clientId: "abc", clientSecret: "x\ny" };
    expect(validateOAuthClientForm(form, info()).clientSecret).toBeDefined();
  });

  it("demands a redirect for providers that refuse loopback", () => {
    const form = { ...initialOAuthClientForm(null), clientId: "abc" };
    const httpsOnly = info({ integration_id: "zoom", requires_https_redirect: true });
    expect(validateOAuthClientForm(form, httpsOnly).redirectUri).toBeDefined();
    expect(
      hasFormErrors(
        validateOAuthClientForm({ ...form, redirectUri: "https://relay.example.com/zoom" }, httpsOnly),
      ),
    ).toBe(false);
  });
});

describe("toOAuthClientInput", () => {
  const base = { clientId: " abc ", clientSecret: "", clearSecret: false, redirectUri: " " };

  it("keeps the stored secret when the field is left empty", () => {
    expect(toOAuthClientInput(base)).toEqual({ client_id: "abc", client_secret: null, redirect_uri: null });
  });

  it("replaces or clears the secret on request", () => {
    expect(toOAuthClientInput({ ...base, clientSecret: " new " }).client_secret).toBe("new");
    expect(toOAuthClientInput({ ...base, clientSecret: "typed", clearSecret: true }).client_secret).toBe("");
  });

  it("passes a trimmed redirect override", () => {
    expect(toOAuthClientInput({ ...base, redirectUri: " https://r.example/cb " }).redirect_uri).toBe(
      "https://r.example/cb",
    );
  });
});

describe("connectBlocker and redirectToRegister", () => {
  it("blocks connect until an app is configured", () => {
    expect(connectBlocker(null)).not.toBeNull();
    expect(connectBlocker(info())).not.toBeNull();
    expect(connectBlocker(info({ configured: true, client_id: "abc" }))).toBeNull();
  });

  it("blocks https-only providers without a relay", () => {
    const zoom = info({ configured: true, client_id: "abc", requires_https_redirect: true });
    expect(connectBlocker(zoom)).not.toBeNull();
    expect(redirectToRegister(zoom)).toBeNull();
    const relayed = { ...zoom, redirect_uri: "https://relay.example.com/zoom" };
    expect(connectBlocker(relayed)).toBeNull();
    expect(redirectToRegister(relayed)).toBe("https://relay.example.com/zoom");
  });

  it("tells loopback providers to register the local callback", () => {
    expect(redirectToRegister(info())).toBe(LOOPBACK);
  });
});

describe("developerConsoleUrl", () => {
  it("points at https provider consoles only", () => {
    for (const id of ["google-calendar", "gmail", "notion", "zoom", "slack", "github"]) {
      const url = developerConsoleUrl(id);
      expect(url).not.toBeNull();
      expect(new URL(url!).protocol).toBe("https:");
    }
    expect(developerConsoleUrl("telegram")).toBeNull();
  });
});
