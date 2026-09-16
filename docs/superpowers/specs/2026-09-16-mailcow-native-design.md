# Altacee Mail — mailcow-native design

2026-09-16. Six changes to the fork, each shipped and verified on its own.
Everything here is measured against the live mailcow at `ryuvzdff.altacee.com`
from inside the `rav` pod, not inferred from documentation.

## What the mail server actually offers

| Probe | Result |
|---|---|
| IMAP 993 greeting | `IMAP4rev1 SASL-IR LOGIN-REFERRALS ID ENABLE IDLE LITERAL+ AUTH=PLAIN AUTH=LOGIN`, Dovecot |
| ManageSieve 4190 | open; `Dovecot Pigeonhole`; `vacation`, `vacation-seconds`, `imapsieve`, `regex`, `body`, `editheader`, `duplicate`, `enotify`, `extracttext`; `"SASL" ""` before STARTTLS |
| rspamd `/rspamd/stat`, `/rspamd/auth` | 401 |
| mailcow `/api/v1/...` | answers, empty body without a key |
| Latency from the pod | TCP 31–37 ms, TLS complete 83–146 ms |

The network is not the bottleneck. Connection setup and round trips are.

## Decisions taken before writing this

- Background push holds a **per-user mailcow app password**, encrypted at rest.
  Never the sign-in password, and revocable per device from mailcow.
- Secrets are created by hand in the cluster by the owner. This document names
  keys; it never carries values, and no value reaches Git.
- Fork only — `Altacee/rav`. No upstream PRs for now.
- Staged: one branch, one image and one gitops bump per item, verified live
  before the next starts.

## Order

1 → 4 → 5 → 3 → 6 → 2. The contained fixes land first; the mailcow API comes
before push because it can provision the app password that push needs, which
removes a manual paste from the worst part of that flow.

---

## 1. ManageSieve over STARTTLS

**Problem.** `sieve/client.rs` opens a plain `TcpStream` (line 29) and sends
`AUTHENTICATE "PLAIN"` (line 47). mailcow advertises `"SASL" ""` before
STARTTLS, so authentication is refused — and if it were not, the mailbox
password would cross the public internet in clear. `SIEVE_HOST` is therefore
unusable today, which is why filters and the autoresponder only exist inside
this app.

**Design.** After the greeting, parse the advertised capabilities. If
`STARTTLS` is present, issue it, wrap the stream with the
`async_native_tls::TlsConnector` that `MailTransport` already builds for IMAP
(it carries `TLS_CA_CERT_PATH`), and re-read the capability list, which RFC 5804
requires the server to re-issue. Only then authenticate.

**Fail closed.** If STARTTLS is advertised and the handshake fails, the
connection is dropped — no plaintext fallback. If it is *not* advertised, refuse
to authenticate unless `SIEVE_ALLOW_PLAINTEXT=true` is set explicitly, which
exists only for a loopback test server.

**Config.** `SIEVE_HOST=ryuvzdff.altacee.com`, `SIEVE_PORT=4190`.

**Tests.** A fake ManageSieve server in the test suite asserting: (a) with
STARTTLS advertised, no `AUTHENTICATE` byte is ever written to the plaintext
stream; (b) a script uploads and activates; (c) a refused STARTTLS produces an
error rather than a plaintext retry. Each must fail against today's client.

**Verify live.** Create a filter in the UI, confirm it appears as a Sieve script
in mailcow, send a matching message, confirm it is filed with Altacee Mail
closed.

## 4. More than one pooled IMAP session per account

**Problem.** `SessionCache` holds exactly one reusable session per account
(`imap/session_cache.rs:22-27`). Concurrent requests that miss the single slot
open new connections, capped at 4 by a semaphore added after an OOM. Each miss
pays the measured ~100 ms of TCP plus TLS plus LOGIN.

**Design.** Make the slot a small stack: `HashMap<String, Vec<ImapSession>>`,
capped by `IMAP_POOL_SIZE` (default 3). Acquire pops, release pushes back while
under the cap and otherwise logs out and drops. The existing connect semaphore
stays exactly as it is — it is the thing that prevents the OOM, and widening the
pool must not widen that.

**Tests.** N concurrent requests reuse at most N sessions and open no more than
the cap; a poisoned session is dropped rather than returned to the pool.

**Verify live.** Time a folder switch and a bulk flag change over 50 messages
before and after, from the pod.

## 5. QRESYNC

**Problem.** `sync.rs` tiers STATUS → CONDSTORE → full fetch and infers
deletions by comparing message counts (`sync.rs:350`), which is a heuristic that
cannot distinguish a delete from a concurrent arrival.

**Design.** `async-imap` has no QRESYNC helper, but exposes `run_command`,
`run_command_untagged` and `read_response`. Issue `ENABLE QRESYNC` after login,
`SELECT <folder> (QRESYNC (<uidvalidity> <modseq>))` on resync, and parse
`VANISHED (EARLIER)` into explicit deletions. Every failure — server without the
capability, a parse that does not match — falls back to the current CONDSTORE
tier, which stays in place.

**Tests.** Parser fixtures for `VANISHED` ranges (`1:3,7`), a mismatched
`UIDVALIDITY` forcing a full resync, and a capability-absent path that takes the
old tier.

**Verify live.** Delete a message from mailcow's own UI and confirm it
disappears without a full refetch.

## 3. rspamd training that is actually authenticated

**Problem.** `routes/spam.rs` POSTs the raw message to `{RSPAMD_URL}/learnspam`
with no credential. mailcow's controller answers 401, so "mark as spam" today
only moves the message.

**Design.** Add `RSPAMD_PASSWORD`, sent as rspamd's `Password:` header. A 401 is
surfaced as a configuration error in logs once per process, not per click, and
the UI still moves the message so the user action never appears to fail.

**Secret (owner-created).** `rav/rspamd` with key `password`, mounted as
`RSPAMD_PASSWORD`. Config: `RSPAMD_URL=https://ryuvzdff.altacee.com/rspamd`.

**Tests.** A mock rspamd asserting the header is present and the body is
`message/rfc822`; a 401 maps to the config error, not a 500.

**Verify live.** Train one message, then confirm the count moved in mailcow's
rspamd UI.

## 6. mailcow API

**Problem.** Send-as identities and aliases are typed in by hand, and there is no
way to mint the app password item 2 needs.

**Design.** A thin `mailcow` client behind `MAILCOW_API_URL` and an
`X-API-Key` header. Three uses, in order of value:

- **Identities and aliases** — `GET /api/v1/get/alias/all` and the mailbox
  object seed the send-as list, refreshed on login rather than stored twice.
- **App password provisioning** — `POST /api/v1/add/app-passwd` mints a
  protocol-scoped credential for item 2, so the user never pastes one.
- **Quota** — IMAP `GETQUOTA` already answers this; the API is not used for it.

**Scope.** A mailcow API key is workspace-wide, so provisioning needs a
read-write key. If the owner prefers a read-only key, item 6 ships without
provisioning and item 2 keeps the manual paste; this is a switch, not a rewrite.

**Secret (owner-created).** `rav/mailcow-api` with key `api-key`, mounted as
`MAILCOW_API_KEY`. Only this pod mounts it.

**Tests.** A mock mailcow asserting the key header, alias mapping, and that a
failed API call degrades to the hand-typed identity list rather than erroring.

## 2. Background IDLE and Web Push

**Problem.** Realtime exists only while a tab is open. `IdleManager` starts IDLE
per (user, folder) when the WebSocket connects (`realtime/idle.rs:14-16`), the
worker reaps when nobody is connected, notifications are a foreground
`new Notification()` (`useNotifications.ts:81`), and there is no service worker
anywhere in the frontend. Sessions live in an in-memory `DashMap`
(`auth/session.rs:62-74`), so a deploy ends every one of them.

**Design, in four pieces.**

*Credential.* Settings gets "Enable push on this device". The credential is a
mailcow app password — minted through item 6 where the key allows it, pasted
otherwise — encrypted with AES-256-GCM under a 32-byte server key from
`PUSH_CREDENTIAL_KEY`, and stored in that user's SQLite. `aes-gcm` is already a
dependency and `folder_cipher.rs` is the precedent for handling. A new refinery
migration adds `push_credentials(email, nonce, ciphertext, created_at)` and
`push_subscriptions(endpoint, p256dh, auth, created_at)`.

*Worker.* A push worker independent of WebSocket sessions holds one IDLE
connection per stored credential, reconnecting with backoff. An authentication
failure marks the credential invalid, stops that task and asks the user to
re-enable — a bad password must never become a retry loop against the mail
server. Concurrency is capped, and the cap is a config value because it is the
number that decides whether this is kind to mailcow.

*Delivery.* The `web-push` crate with VAPID keys from `PUSH_VAPID_KEY`; the
public key is served by `/api/push/config`. A service worker in
`frontend/public` shows the notification and opens the message on click. The
payload carries sender and subject only — never the body.

*Revocation.* Disabling push deletes both rows; the app password is revoked in
mailcow by the user. Sign-out does not delete it, because surviving sign-out is
the entire point.

**Secret (owner-created).** `rav/push` with keys `credential-key` and
`vapid-private-key`; I generate the VAPID pair locally and hand over the values
for the owner to load, or the owner generates both.

**Tests.** Round-trip encrypt/decrypt with a wrong key failing closed; a
credential that fails authentication stops its task and is marked invalid; the
payload contains no body text; a push subscription that returns 410 Gone is
deleted rather than retried forever.

**Verify live.** Close every tab, send mail from another account, and confirm the
notification arrives; then restart the pod and confirm it still arrives.

**Risk, stated plainly.** This is the only item that stores a mailbox credential
at rest. Its blast radius is bounded by three things: the credential is a
per-device app password rather than a login, the encryption key lives only in a
cluster Secret, and mailcow can revoke any of them without touching this app.
If the owner would rather not hold credentials at all, the alternative is push
that dies at every deploy, which is not worth building.

## What I need from the owner

| When | What |
|---|---|
| Before item 3 | Secret `rav/rspamd`, key `password` — the rspamd controller password |
| Before item 6 | Secret `rav/mailcow-api`, key `api-key`, and whether it is read-only or read-write |
| Before item 2 | Secret `rav/push`, keys `credential-key` (32 random bytes) and `vapid-private-key` |

## Out of scope

SOGo, ActiveSync, CalDAV/CardDAV against mailcow, and OIDC sign-in against our
own identity stack. The IMAP server advertises only `AUTH=PLAIN` and
`AUTH=LOGIN`, so single sign-on would need mailcow-side work first, and it is a
separate decision from anything above.
