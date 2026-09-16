// Service worker for Altacee Mail push notifications.
//
// It is deliberately tiny: the payload carries a sender, a subject and a
// folder, never the message body, so there is nothing here worth caching or
// interpreting beyond showing the notification and opening the mailbox.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    // A payload we cannot read still deserves a notification: something arrived.
  }
  const title = data.sender || "New mail";
  const body = data.subject || "";
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      tag: "altacee-mail",
      icon: "/icon.svg",
      badge: "/icon.svg",
      data: { folder: data.folder || "INBOX" },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clients) => {
      // Reuse an open tab when there is one; nobody wants a second copy of
      // their mailbox for every notification.
      for (const client of clients) {
        if ("focus" in client) return client.focus();
      }
      return self.clients.openWindow("/mail");
    }),
  );
});
