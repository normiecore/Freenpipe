// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OAuthClientSetup } from "../oauth-client-dialog";

const LOOPBACK = "http://localhost:3035/connections/oauth/callback";

const mocks = vi.hoisted(() => ({
  oauthClientGet: vi.fn(),
  oauthClientSave: vi.fn(),
  oauthClientDelete: vi.fn(),
  openUrl: vi.fn(),
}));

vi.mock("@/lib/utils/tauri", () => ({
  commands: {
    oauthClientGet: mocks.oauthClientGet,
    oauthClientSave: mocks.oauthClientSave,
    oauthClientDelete: mocks.oauthClientDelete,
  },
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

function info(over: Record<string, unknown> = {}) {
  return {
    integration_id: "notion",
    configured: false,
    client_id: null,
    has_client_secret: false,
    redirect_uri: null,
    default_redirect_uri: LOOPBACK,
    requires_https_redirect: false,
    uses_pkce: false,
    auth_url: "https://api.notion.com/v1/oauth/authorize",
    token_url: "https://api.notion.com/v1/oauth/token",
    ...over,
  };
}

describe("OAuthClientSetup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("explains that an OAuth app is needed and saves one", async () => {
    mocks.oauthClientGet.mockResolvedValue({ status: "ok", data: info() });
    const saved = info({ configured: true, client_id: "my-client", has_client_secret: true });
    mocks.oauthClientSave.mockResolvedValue({ status: "ok", data: saved });
    const onChange = vi.fn();

    render(<OAuthClientSetup integrationId="notion" integrationName="Notion" onChange={onChange} />);

    expect(await screen.findByText(/needs your own Notion OAuth app/)).toBeInTheDocument();
    await waitFor(() => expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ configured: false })));

    fireEvent.click(screen.getByRole("button", { name: "set up OAuth app" }));
    // The loopback URI to register is shown for providers that accept it.
    expect(await screen.findByText(LOOPBACK)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Client ID"), { target: { value: " my-client " } });
    fireEvent.change(screen.getByLabelText("Client secret"), { target: { value: "top-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));

    await waitFor(() =>
      expect(mocks.oauthClientSave).toHaveBeenCalledWith("notion", {
        client_id: "my-client",
        client_secret: "top-secret",
        redirect_uri: null,
      }),
    );
    await waitFor(() => expect(onChange).toHaveBeenLastCalledWith(saved));
    expect(await screen.findByText("my-client")).toBeInTheDocument();
  });

  it("refuses to save an https-only provider without a relay", async () => {
    mocks.oauthClientGet.mockResolvedValue({
      status: "ok",
      data: info({ integration_id: "zoom", requires_https_redirect: true }),
    });

    render(<OAuthClientSetup integrationId="zoom" integrationName="Zoom" />);
    fireEvent.click(await screen.findByRole("button", { name: "set up OAuth app" }));
    expect(await screen.findByText(/only accepts HTTPS redirects/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Client ID"), { target: { value: "zoom-client" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));

    expect(await screen.findByText(/needs an HTTPS redirect/)).toBeInTheDocument();
    expect(mocks.oauthClientSave).not.toHaveBeenCalled();
  });

  it("shows backend validation errors", async () => {
    mocks.oauthClientGet.mockResolvedValue({ status: "ok", data: info() });
    mocks.oauthClientSave.mockResolvedValue({ status: "error", error: "the secret store is unavailable" });

    render(<OAuthClientSetup integrationId="notion" integrationName="Notion" />);
    fireEvent.click(await screen.findByRole("button", { name: "set up OAuth app" }));
    fireEvent.change(screen.getByLabelText("Client ID"), { target: { value: "c" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));

    expect(await screen.findByText("the secret store is unavailable")).toBeInTheDocument();
  });
});
