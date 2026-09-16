"use client";

import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost, apiPut } from "@/lib/api";

const BASE_PATH = process.env.NEXT_PUBLIC_BASE_PATH || "";

export type PushConfig = {
  /** The server has both keys configured; without this, push cannot work at all. */
  enabled: boolean;
  public_key: string | null;
  credential_stored: boolean;
  /** The stored app password stopped authenticating and needs replacing. */
  credential_invalid: boolean;
};

/**
 * base64url → bytes, the form PushManager wants for its VAPID key.
 *
 * Returns an ArrayBuffer rather than a Uint8Array: TypeScript's DOM types
 * insist the view be backed by a plain ArrayBuffer, which `Uint8Array` alone
 * does not promise.
 */
function decodeKey(base64: string): ArrayBuffer {
  const padded = base64.padEnd(base64.length + ((4 - (base64.length % 4)) % 4), "=");
  const raw = atob(padded.replace(/-/g, "+").replace(/_/g, "/"));
  const bytes = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i += 1) bytes[i] = raw.charCodeAt(i);
  return bytes.buffer;
}

function encodeKey(buffer: ArrayBuffer | null): string {
  if (!buffer) return "";
  const bytes = new Uint8Array(buffer);
  let binary = "";
  bytes.forEach((b) => (binary += String.fromCharCode(b)));
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function pushSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

/**
 * Notifications that survive the tab being closed.
 *
 * Two separate things have to be true: the server holds an app password so it
 * can keep an IMAP connection open, and this browser has a push subscription.
 * The hook exposes both so the UI can say which one is missing.
 */
export function usePush() {
  const queryClient = useQueryClient();

  // The server's half: are both keys configured, and is an app password stored?
  const { data: config } = useQuery({
    queryKey: ["push", "config"],
    queryFn: () => apiGet<PushConfig>("/push/config"),
  });

  // This browser's half. Kept as its own query rather than effect state so the
  // two can be refetched together after enabling or disabling.
  const { data: subscribed = false } = useQuery({
    queryKey: ["push", "subscription"],
    enabled: pushSupported(),
    queryFn: async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      const existing = await registration?.pushManager.getSubscription();
      return !!existing;
    },
  });

  const refresh = useCallback(
    () => queryClient.invalidateQueries({ queryKey: ["push"] }),
    [queryClient],
  );

  /** Store the app password that keeps IDLE alive, then subscribe this browser. */
  const enableMutation = useMutation({
    mutationFn: async (appPassword: string) => {
      if (!pushSupported()) throw new Error("This browser cannot receive push notifications");
      const current = config ?? (await apiGet<PushConfig>("/push/config"));
      if (!current.enabled || !current.public_key) {
        throw new Error("Push is not configured on this server");
      }
      if (Notification.permission !== "granted") {
        const permission = await Notification.requestPermission();
        if (permission !== "granted") throw new Error("Notifications were blocked");
      }

      await apiPut("/push/credential", { app_password: appPassword });

      const registration = await navigator.serviceWorker.register(`${BASE_PATH}/sw.js`);
      const subscription =
        (await registration.pushManager.getSubscription()) ??
        (await registration.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: decodeKey(current.public_key),
        }));

      await apiPost("/push/subscription", {
        endpoint: subscription.endpoint,
        p256dh: encodeKey(subscription.getKey("p256dh")),
        auth: encodeKey(subscription.getKey("auth")),
      });
    },
    onSuccess: refresh,
  });

  /** Stop notifications everywhere and forget the stored credential. */
  const disableMutation = useMutation({
    mutationFn: async () => {
      if (pushSupported()) {
        const registration = await navigator.serviceWorker.getRegistration();
        const subscription = await registration?.pushManager.getSubscription();
        if (subscription) {
          await apiDelete("/push/subscription", { endpoint: subscription.endpoint });
          await subscription.unsubscribe();
        }
      }
      await apiDelete("/push/credential");
    },
    onSuccess: refresh,
  });

  return {
    config: config ?? null,
    subscribed,
    busy: enableMutation.isPending || disableMutation.isPending,
    enable: (appPassword: string) => enableMutation.mutateAsync(appPassword),
    disable: () => disableMutation.mutateAsync(),
    refresh,
  };
}
