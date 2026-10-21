// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe
"use client";

import React, { useEffect, useState } from "react";
import { HardDrive } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useSettings } from "@/lib/hooks/use-settings";
import { commands } from "@/lib/utils/tauri";
import { LOW_DISK_GUARD_LABEL, lowDiskGuardDescription } from "@/lib/low-disk-guard";

/**
 * The `stopRecordingOnLowDisk` switch (default on). The guard in the app
 * reads the setting at its next check, so no restart is needed.
 */
export function LowDiskGuardSetting() {
  const { settings, updateSettings } = useSettings();
  const [thresholdBytes, setThresholdBytes] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    commands
      .getLowDiskGuardConfig()
      .then((result) => {
        if (cancelled) return;
        setThresholdBytes(result.status === "ok" ? Number(result.data.thresholdBytes) : null);
      })
      .catch(() => {
        if (!cancelled) setThresholdBytes(null);
      });
    return () => {
      cancelled = true;
    };
  }, [settings.dataDir]);

  return (
    <Card className="border-border bg-card">
      <CardContent className="px-3 py-2.5">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center space-x-2.5">
            <HardDrive className="h-4 w-4 text-muted-foreground shrink-0" />
            <div>
              <h3 className="text-sm font-medium text-foreground">{LOW_DISK_GUARD_LABEL}</h3>
              <p className="text-xs text-muted-foreground">
                {lowDiskGuardDescription(thresholdBytes)}
              </p>
            </div>
          </div>
          <Switch
            id="stop-recording-on-low-disk"
            data-testid="low-disk-recording-guard-toggle"
            aria-label={LOW_DISK_GUARD_LABEL}
            checked={settings.stopRecordingOnLowDisk ?? true}
            onCheckedChange={(checked) => {
              void updateSettings({ stopRecordingOnLowDisk: checked });
            }}
          />
        </div>
      </CardContent>
    </Card>
  );
}
