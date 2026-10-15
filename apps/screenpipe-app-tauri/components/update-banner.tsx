"use client";

import { Button } from "@/components/ui/button";
import { Sparkles, X } from "lucide-react";
import { create } from "zustand";
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/lib/utils/tauri";
import { useToast } from "@/components/ui/use-toast";
import { cn } from "@/lib/utils";

interface UpdateInfo {
  version: string;
  body: string;
}

interface UpdateBannerState {
  isVisible: boolean;
  updateInfo: UpdateInfo | null;
  isInstalling: boolean;
  // Version the user dismissed in this session. Periodic re-checks and
  // providers-remount hydration would otherwise re-show the same banner
  // immediately after the user clicked X.
  dismissedVersion: string | null;
  setIsVisible: (visible: boolean) => void;
  setUpdateInfo: (info: UpdateInfo | null) => void;
  setIsInstalling: (installing: boolean) => void;
  dismiss: (version: string) => void;
  resetDismissed: () => void;
}

export const useUpdateBanner = create<UpdateBannerState>((set) => ({
  isVisible: false,
  updateInfo: null,
  isInstalling: false,
  dismissedVersion: null,
  setIsVisible: (visible) => set({ isVisible: visible }),
  setUpdateInfo: (info) => set({ updateInfo: info }),
  setIsInstalling: (installing) => set({ isInstalling: installing }),
  dismiss: (version) => set({ isVisible: false, dismissedVersion: version }),
  resetDismissed: () => set({ dismissedVersion: null }),
}));

interface UpdateBannerProps {
  className?: string;
  compact?: boolean;
  /** "sidebar" renders a vertical card sized for the app sidebar. */
  variant?: "default" | "sidebar";
}

export function UpdateBanner({ className, compact = false, variant = "default" }: UpdateBannerProps) {
  const { isVisible, updateInfo, isInstalling, setIsInstalling, dismiss } = useUpdateBanner();
  const { toast } = useToast();

  const handleUpdate = async () => {
    setIsInstalling(true);
    const progress = toast({
      title: "installing update...",
      description: "the app restarts by itself once the update is in place",
      duration: Infinity,
    });

    // restart_for_update waits (up to 60s) until startup is far enough
    // along to exit safely (#3622), then downloads if needed, installs and
    // restarts, so on success this call never returns. Any other answer
    // means nothing was applied; an error result counts as "errored".
    let outcome = "errored";
    let detail: string | null = null;
    try {
      const res = await commands.restartForUpdate(60);
      if (res.status === "ok") {
        outcome = res.data;
      } else {
        detail = res.error;
      }
    } catch (error) {
      detail = String(error);
    }
    if (outcome === "proceed") return;

    console.error("update not applied:", outcome, detail ?? "");
    progress.dismiss();
    setIsInstalling(false);
    toast({
      title: detail ? "update failed" : "still starting up",
      description:
        detail ??
        (outcome === "pending"
          ? "finish startup first, then click update again"
          : "startup error — open settings to see details before restarting"),
      variant: "destructive",
    });
  };

  if (!isVisible || !updateInfo) return null;

  if (variant === "sidebar") {
    return (
      <button
        type="button"
        data-testid="update-banner"
        onClick={handleUpdate}
        disabled={isInstalling}
        className={cn(
          "w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg border border-border bg-card/50 hover:bg-card transition-colors text-left disabled:opacity-60",
          className,
        )}
      >
        <Sparkles className="h-4 w-4 text-primary shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="text-xs font-medium text-foreground truncate">
            {isInstalling ? "Restarting…" : "Restart to update"}
          </div>
          <div className="text-[10px] text-muted-foreground truncate">v{updateInfo.version}</div>
        </div>
      </button>
    );
  }

  if (compact) {
    return (
      <div className={cn(
        "flex items-center gap-2 text-xs text-muted-foreground",
        className
      )}>
        <Sparkles className="h-3 w-3 text-primary" />
        <span>v{updateInfo.version} ready</span>
        <Button
          variant="ghost"
          size="sm"
          className="h-5 px-2 text-xs"
          onClick={handleUpdate}
          disabled={isInstalling}
        >
          {isInstalling ? "restarting..." : "restart to update"}
        </Button>
      </div>
    );
  }

  return (
    <div className={cn(
      "flex items-center justify-between gap-3 px-3 py-2 bg-muted/50 border-b text-sm",
      className
    )}>
      <div className="flex items-center gap-2 flex-1">
        <Sparkles className="h-4 w-4 text-primary" />
        <span>
          update <span className="font-medium">v{updateInfo.version}</span> is ready
        </span>
      </div>
      <div className="flex items-center gap-2">
        <Button
          variant="default"
          size="sm"
          className="h-7 px-3 text-xs"
          onClick={handleUpdate}
          disabled={isInstalling}
        >
          {isInstalling ? "restarting..." : "restart to update"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 w-7 p-0"
          onClick={() => dismiss(updateInfo.version)}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

// Hook to listen for update events from Rust.
// Mounted globally in app/providers.tsx so it survives route changes and
// catches the `update-available` event regardless of which page is open
// when the download finishes. On mount, it also pulls the current pending
// state from Rust so it can recover if the event fired before this hook
// registered (boot-time webview race).
export function useUpdateListener() {
  const { setIsVisible, setUpdateInfo, resetDismissed } = useUpdateBanner();

  useEffect(() => {
    let unlistenAvailable: (() => void) | undefined;
    let unlistenClick: (() => void) | undefined;

    // Rust re-emits update-available on every periodic check, and providers
    // hydration runs on every remount — both would otherwise resurrect a
    // banner the user just dismissed. Read dismissedVersion fresh inside the
    // callback so a newer version still shows even if an older one is dismissed.
    const showIfNotDismissed = (info: UpdateInfo) => {
      setUpdateInfo(info);
      if (useUpdateBanner.getState().dismissedVersion !== info.version) {
        setIsVisible(true);
      }
    };

    const setupListeners = async () => {
      // Download happens silently in the background. Banner only appears
      // when the update is ready to install.
      unlistenAvailable = await listen<UpdateInfo>("update-available", (event) => {
        showIfNotDismissed(event.payload);
      });

      // Tray click is an explicit user request — clear any prior dismissal
      // so the banner reappears even if they X'd it earlier this session.
      unlistenClick = await listen("update-now-clicked", () => {
        resetDismissed();
        setIsVisible(true);
      });

      // Hydrate from Rust in case the event fired before we mounted.
      try {
        const resPending = await commands.getPendingUpdate();
        const pending = resPending.status === "ok" ? resPending.data : null;
        if (pending?.downloaded) {
          showIfNotDismissed({ version: pending.version, body: pending.body });
        }
      } catch (e) {
        // Command not registered yet (older Rust side) or app not ready.
        // Fall back to event-driven path silently.
      }
    };

    setupListeners();

    return () => {
      unlistenAvailable?.();
      unlistenClick?.();
    };
  }, [setIsVisible, setUpdateInfo, resetDismissed]);
}
