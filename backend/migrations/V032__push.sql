-- Background push: one stored credential per mailbox, plus the browser
-- subscriptions it notifies.
--
-- The credential is a mailcow app password, encrypted with AES-256-GCM under a
-- key that lives only in a cluster Secret. It is stored at all so that IDLE can
-- outlive the browser tab — that is the whole point of push — and it is a
-- per-device app password rather than a login, so mailcow can revoke it alone.
CREATE TABLE IF NOT EXISTS push_credential (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    email         TEXT NOT NULL,
    imap_host     TEXT NOT NULL,
    imap_port     INTEGER NOT NULL,
    nonce         BLOB NOT NULL,
    ciphertext    BLOB NOT NULL,
    -- Set when the stored credential stops authenticating. A bad password must
    -- never become a retry loop against the mail server.
    invalid_since TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS push_subscription (
    endpoint   TEXT PRIMARY KEY,
    p256dh     TEXT NOT NULL,
    auth       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
