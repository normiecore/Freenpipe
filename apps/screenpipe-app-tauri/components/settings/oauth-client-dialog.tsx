"use client";

// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * "OAuth app" setup for native OAuth connections.
 *
 * Freenpipe ships no OAuth client of its own: the user registers an app with
 * the provider and enters its client ID (and secret, and an HTTPS relay for
 * providers that refuse loopback redirects) here. The values go straight to
 * the SecretStore through the `oauth_client_*` commands; the stored secret is
 * never sent back to this UI, only whether one exists.
 */

import React, { useCallback, useEffect, useState } from "react";
import { Check, Copy, ExternalLink, KeyRound, Loader2 } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { commands, type OAuthClientInfo } from "@/lib/utils/tauri";
import {
  connectBlocker,
  developerConsoleUrl,
  hasFormErrors,
  initialOAuthClientForm,
  redirectToRegister,
  toOAuthClientInput,
  validateOAuthClientForm,
  type OAuthClientFormErrors,
  type OAuthClientFormState,
} from "@/lib/oauth-client-form";

/** Load (and reload) the OAuth app summary for one integration. */
export function useOAuthClient(integrationId: string) {
  const [info, setInfo] = useState<OAuthClientInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const res = await commands.oauthClientGet(integrationId);
      if (res.status === "ok") {
        setInfo(res.data);
        setError(null);
      } else {
        setError(String(res.error));
      }
    } catch (e) {
      setError(String(e));
    }
  }, [integrationId]);

  useEffect(() => {
    setInfo(null);
    refresh();
  }, [refresh]);

  return { info, error, refresh, setInfo };
}

function CopyableValue({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable; the value stays selectable */
    }
  };
  return (
    <div className="flex items-center gap-1.5 rounded-md border border-border bg-muted/40 px-2 py-1.5">
      <code className="flex-1 truncate text-[11px] select-all">{value}</code>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-6 w-6 p-0 shrink-0"
        onClick={copy}
        aria-label="copy redirect URI"
      >
        {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
      </Button>
    </div>
  );
}

export function OAuthClientDialog({
  integrationId,
  integrationName,
  info,
  open,
  onOpenChange,
  onChanged,
}: {
  integrationId: string;
  integrationName: string;
  info: OAuthClientInfo | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged: (info: OAuthClientInfo) => void;
}) {
  const [form, setForm] = useState<OAuthClientFormState>(() => initialOAuthClientForm(info));
  const [errors, setErrors] = useState<OAuthClientFormErrors>({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"save" | "remove" | null>(null);

  // Start from the stored values each time the dialog opens.
  useEffect(() => {
    if (open) {
      setForm(initialOAuthClientForm(info));
      setErrors({});
      setSaveError(null);
    }
  }, [open, info]);

  const update = (patch: Partial<OAuthClientFormState>) => {
    setForm((f) => ({ ...f, ...patch }));
    setSaveError(null);
  };

  const save = async () => {
    const found = validateOAuthClientForm(form, info);
    setErrors(found);
    if (hasFormErrors(found)) return;
    setBusy("save");
    try {
      const res = await commands.oauthClientSave(integrationId, toOAuthClientInput(form));
      if (res.status === "ok") {
        onChanged(res.data);
        onOpenChange(false);
      } else {
        setSaveError(String(res.error));
      }
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    setBusy("remove");
    try {
      const res = await commands.oauthClientDelete(integrationId);
      if (res.status === "ok") {
        onChanged(res.data);
        onOpenChange(false);
      } else {
        setSaveError(String(res.error));
      }
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const consoleUrl = developerConsoleUrl(integrationId);
  const registerUri = info ? redirectToRegister(info) : null;
  const secretPlaceholder = info?.has_client_secret
    ? "stored; leave empty to keep it"
    : info?.uses_pkce
      ? "optional for public (PKCE) apps"
      : "client secret";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="text-sm">OAuth app for {integrationName}</DialogTitle>
          <DialogDescription className="text-xs leading-relaxed">
            Register your own OAuth app with {integrationName} and paste its credentials here.
            They are kept in this computer&apos;s secret store and are only sent to{" "}
            {integrationName}.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 text-xs">
          {consoleUrl && (
            <button
              type="button"
              onClick={() => openUrl(consoleUrl)}
              className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground underline"
            >
              <ExternalLink className="h-3 w-3" />
              open the {integrationName} developer console
            </button>
          )}

          <div className="space-y-1">
            <Label className="text-xs">Redirect URI to register</Label>
            {registerUri ? (
              <CopyableValue value={registerUri} />
            ) : (
              <p className="text-muted-foreground leading-relaxed">
                {integrationName} only accepts HTTPS redirects. Run a relay you control that
                forwards the full query string to{" "}
                <code className="text-[11px]">{info?.default_redirect_uri}</code>, register the
                relay&apos;s URL with {integrationName}, and enter it below.
              </p>
            )}
          </div>

          <div className="space-y-1">
            <Label htmlFor={`${integrationId}-client-id`} className="text-xs">
              Client ID
            </Label>
            <Input
              id={`${integrationId}-client-id`}
              value={form.clientId}
              onChange={(e) => update({ clientId: e.target.value })}
              autoComplete="off"
              spellCheck={false}
              className="h-8 text-xs"
            />
            {errors.clientId && <p className="text-destructive">{errors.clientId}</p>}
          </div>

          <div className="space-y-1">
            <Label htmlFor={`${integrationId}-client-secret`} className="text-xs">
              Client secret
            </Label>
            <Input
              id={`${integrationId}-client-secret`}
              type="password"
              value={form.clientSecret}
              onChange={(e) => update({ clientSecret: e.target.value, clearSecret: false })}
              placeholder={form.clearSecret ? "will be removed on save" : secretPlaceholder}
              autoComplete="new-password"
              spellCheck={false}
              disabled={form.clearSecret}
              className="h-8 text-xs"
            />
            {info?.has_client_secret && (
              <button
                type="button"
                onClick={() => update({ clearSecret: !form.clearSecret, clientSecret: "" })}
                className="text-[10px] text-muted-foreground hover:text-foreground underline"
              >
                {form.clearSecret ? "keep the stored secret" : "remove the stored secret"}
              </button>
            )}
            {errors.clientSecret && <p className="text-destructive">{errors.clientSecret}</p>}
          </div>

          <div className="space-y-1">
            <Label htmlFor={`${integrationId}-redirect`} className="text-xs">
              Redirect URI {info?.requires_https_redirect ? "(HTTPS relay, required)" : "(optional)"}
            </Label>
            <Input
              id={`${integrationId}-redirect`}
              value={form.redirectUri}
              onChange={(e) => update({ redirectUri: e.target.value })}
              placeholder={
                info?.requires_https_redirect
                  ? "https://your-relay.example.com/oauth"
                  : info?.default_redirect_uri
              }
              autoComplete="off"
              spellCheck={false}
              className="h-8 text-xs"
            />
            {errors.redirectUri && <p className="text-destructive">{errors.redirectUri}</p>}
          </div>

          {saveError && (
            <p className="rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-destructive">
              {saveError}
            </p>
          )}
        </div>

        <DialogFooter className="gap-2 sm:justify-between">
          {info?.configured ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={remove}
              disabled={busy !== null}
              className="h-7 text-xs text-muted-foreground hover:text-destructive"
            >
              {busy === "remove" && <Loader2 className="h-3 w-3 mr-1 animate-spin" />}
              remove app
            </Button>
          ) : (
            <span />
          )}
          <div className="flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
              className="h-7 text-xs"
            >
              cancel
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={save}
              disabled={busy !== null}
              className="h-7 text-xs"
            >
              {busy === "save" && <Loader2 className="h-3 w-3 mr-1 animate-spin" />}
              save
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Compact status line plus the dialog. Mount it next to any OAuth connect
 * button; `onChange` reports the latest summary so the caller can gate the
 * button with `connectBlocker`.
 */
export function OAuthClientSetup({
  integrationId,
  integrationName,
  onChange,
}: {
  integrationId: string;
  integrationName: string;
  onChange?: (info: OAuthClientInfo | null) => void;
}) {
  const { info, error, setInfo } = useOAuthClient(integrationId);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    onChange?.(info);
  }, [info, onChange]);

  if (error) {
    return <p className="text-xs text-destructive">could not read the OAuth app settings: {error}</p>;
  }
  if (!info) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        loading OAuth app settings...
      </div>
    );
  }

  const blocker = connectBlocker(info);
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border px-2.5 py-2 text-xs">
      <span className="flex min-w-0 items-center gap-1.5 text-muted-foreground">
        <KeyRound className="h-3 w-3 shrink-0" />
        {info.configured ? (
          <span className="truncate">
            OAuth app <code className="text-[11px]">{info.client_id}</code>
            {blocker ? ` (${blocker})` : ""}
          </span>
        ) : (
          <span>Connecting needs your own {integrationName} OAuth app.</span>
        )}
      </span>
      <Button
        type="button"
        variant={info.configured ? "ghost" : "outline"}
        size="sm"
        onClick={() => setOpen(true)}
        className="h-6 px-2 text-xs shrink-0"
      >
        {info.configured ? "edit" : "set up OAuth app"}
      </Button>
      <OAuthClientDialog
        integrationId={integrationId}
        integrationName={integrationName}
        info={info}
        open={open}
        onOpenChange={setOpen}
        onChanged={setInfo}
      />
    </div>
  );
}
