// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe
"use client";

import React, { useState } from "react";
import { Coffee } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/use-toast";
import { useSettings } from "@/lib/hooks/use-settings";
import { commands } from "@/lib/utils/tauri";
import { KEEP_AWAKE_LABEL, applyKeepAwake } from "@/lib/low-disk-guard";

/**
 * The `keepComputerAwake` switch (default off). It applies the hold at once
 * through `set_keep_awake`; the app applies the saved value at startup.
 */
export function KeepAwakeSetting() {
  const { settings, updateSettings } = useSettings();
  const { toast } = useToast();
  const [busy, setBusy] = useState(false);

  const onCheckedChange = async (checked: boolean) => {
    setBusy(true);
    try {
      const result = await applyKeepAwake(checked, commands.setKeepAwake, (enabled) =>
        updateSettings({ keepComputerAwake: enabled }),
      );
      if (!result.ok) {
        toast({
          title: checked ? "could not keep the computer awake" : "could not release keep-awake",
          description: result.error,
          variant: "destructive",
        });
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card className="border-border bg-card">
      <CardContent className="px-3 py-2.5">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center space-x-2.5">
            <Coffee className="h-4 w-4 text-muted-foreground shrink-0" />
            <div>
              <h3 className="text-sm font-medium text-foreground">{KEEP_AWAKE_LABEL}</h3>
              <p className="text-xs text-muted-foreground">
                Stop the computer from sleeping while idle; the screen can still turn off
              </p>
            </div>
          </div>
          <Switch
            id="keepComputerAwake"
            aria-label={KEEP_AWAKE_LABEL}
            checked={settings.keepComputerAwake ?? false}
            disabled={busy}
            onCheckedChange={(checked) => {
              void onCheckedChange(checked);
            }}
          />
        </div>
      </CardContent>
    </Card>
  );
}
