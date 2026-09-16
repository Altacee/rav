"use client";

import { useState } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { usePush, pushSupported } from "@/hooks/usePush";

/**
 * Notifications that arrive with Altacee Mail closed.
 *
 * This is the one place the app asks for a credential it will keep: an app
 * password created in mailcow, held encrypted so the server can hold an IMAP
 * connection open when no tab is. The copy says so plainly rather than hiding
 * it behind a toggle.
 */
export function PushSettings() {
  const { config, subscribed, busy, enable, disable } = usePush();
  const [appPassword, setAppPassword] = useState("");

  if (!config) return null;

  if (!pushSupported()) {
    return (
      <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">
        This browser cannot receive background notifications.
      </div>
    );
  }

  if (!config.enabled) {
    return (
      <div className="rounded-lg border border-border p-4">
        <div className="text-sm font-medium">Background notifications</div>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Not configured on this server.
        </p>
      </div>
    );
  }

  const active = config.credential_stored && subscribed && !config.credential_invalid;

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-border p-4">
        <div className="flex items-start justify-between gap-4">
          <div>
            <div className="text-sm font-medium">Background notifications</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {active
                ? "New mail reaches this device with Altacee Mail closed."
                : "Paste an app password from mailcow. It is stored encrypted so the server can watch your inbox while nothing is open, and you can revoke it in mailcow at any time."}
            </p>
          </div>
          {active && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                disable()
                  .then(() => toast.success("Background notifications turned off"))
                  .catch((e: Error) => toast.error(e.message))
              }
              className="shrink-0 rounded-md border border-border px-3 py-1.5 text-xs font-medium transition-colors hover:bg-accent"
            >
              {busy ? <Loader2 className="size-3.5 animate-spin" /> : "Turn off"}
            </button>
          )}
        </div>

        {config.credential_invalid && (
          <p className="mt-3 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-400">
            The stored app password stopped working — it was probably revoked.
            Enter a new one to start receiving notifications again.
          </p>
        )}

        {!active && (
          <form
            className="mt-3 flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              enable(appPassword)
                .then(() => {
                  setAppPassword("");
                  toast.success("Background notifications are on");
                })
                .catch((err: Error) => toast.error(err.message));
            }}
          >
            <input
              type="password"
              value={appPassword}
              onChange={(e) => setAppPassword(e.target.value)}
              placeholder="mailcow app password"
              autoComplete="off"
              className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm"
            />
            <button
              type="submit"
              disabled={busy || appPassword.length === 0}
              className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
            >
              {busy ? <Loader2 className="size-3.5 animate-spin" /> : "Turn on"}
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
