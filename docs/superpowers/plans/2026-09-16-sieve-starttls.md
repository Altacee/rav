# Item 1: ManageSieve over STARTTLS — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make filters and the out-of-office responder run inside Dovecot 24/7 by talking ManageSieve to mailcow over STARTTLS, without ever putting a mailbox password on the wire in clear.

**Architecture:** Split the ManageSieve protocol from its transport so the handshake can be tested over an in-memory duplex, then upgrade the socket with the `async_native_tls::TlsConnector` that `MailTransport` already builds for IMAP. Filters and vacation become one uploaded script, because Dovecot activates exactly one.

**Tech Stack:** Rust, tokio (`features = ["full"]`, so `tokio::io::duplex` is available), `async-native-tls` 0.6, `base64`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-16-mailcow-native-design.md`

## Global Constraints

- Server: `ryuvzdff.altacee.com:4190`, Dovecot Pigeonhole. Its banner advertises `"SASL" ""` then `"STARTTLS"` — authentication is impossible until TLS is up.
- Never send `AUTHENTICATE` over an unencrypted stream unless `SIEVE_ALLOW_PLAINTEXT=true`, which exists only for a loopback test server. Default false.
- Available Sieve extensions (from the live banner): `fileinto reject envelope vacation subaddress comparator-i;ascii-numeric relational regex imap4flags copy include variables body enotify environment mailbox date index ihave duplicate mime foreverypart extracttext vacation-seconds editheader imapflags notify imapsieve`.
- Dovecot keeps one active script per user, so filters and vacation share the single script named `rav-filters`.
- Tests are inline `#[cfg(test)]` modules, the convention in this codebase (`src/config.rs`, `src/folder_cipher.rs`). Run with `cargo test`.
- Never log a password, a script body, or a base64 SASL blob.

## File Structure

| File | Responsibility |
|---|---|
| `backend/src/sieve/protocol.rs` (create) | The ManageSieve protocol over any `AsyncRead + AsyncWrite`: capabilities, STARTTLS request, authenticate, putscript, setactive, logout. No sockets, no TLS, no config. |
| `backend/src/sieve/client.rs` (rewrite) | Transport: TCP connect, the TLS upgrade decision, and the fail-closed rule. Calls `protocol.rs` for every byte. |
| `backend/src/sieve/generator.rs` (modify) | Adds vacation-script generation next to the existing filter rules. |
| `backend/src/sieve/mod.rs` (modify) | `push_state()` replaces `push_filters()`: one script carrying both. |
| `backend/src/config.rs` (modify) | `sieve_allow_plaintext`. |
| `backend/src/routes/filters.rs`, `backend/src/routes/vacation.rs` (modify) | Both call `push_state` so either edit republishes the whole script. |
| `gitops/apps/rav/rav.yaml` (modify, other repo) | `SIEVE_HOST`, `SIEVE_PORT`. |

---

### Task 1: Protocol over a generic stream, with capability parsing

**Files:**
- Create: `backend/src/sieve/protocol.rs`
- Modify: `backend/src/sieve/mod.rs` (add `mod protocol;`)
- Test: inline `#[cfg(test)]` in `backend/src/sieve/protocol.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct Capabilities { pub starttls: bool, pub sasl: Vec<String> }`; `pub struct Sieve<S>` with `pub fn new(stream: S) -> Self`, `pub async fn read_capabilities(&mut self) -> Result<Capabilities, String>`, `pub async fn request_starttls(&mut self) -> Result<(), String>`, `pub async fn authenticate_plain(&mut self, email: &str, password: &str) -> Result<(), String>`, `pub async fn put_script(&mut self, name: &str, script: &str) -> Result<(), String>`, `pub async fn set_active(&mut self, name: &str) -> Result<(), String>`, `pub async fn logout(&mut self)`, `pub fn into_inner(self) -> S`.

- [ ] **Step 1: Write the failing test**

Add to `backend/src/sieve/protocol.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds canned server output and records everything the client wrote.
    async fn exchange(server_says: &str, f: impl AsyncFnOnce(&mut Sieve<tokio::io::DuplexStream>)) -> String {
        let (client_side, mut server_side) = tokio::io::duplex(8192);
        let script = server_says.to_string();
        let handle = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            server_side.write_all(script.as_bytes()).await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            // Read until the client hangs up.
            while let Ok(n) = server_side.read(&mut buf).await {
                if n == 0 { break; }
                seen.extend_from_slice(&buf[..n]);
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        let mut sieve = Sieve::new(client_side);
        f(&mut sieve).await;
        drop(sieve);
        handle.await.unwrap()
    }

    #[tokio::test]
    async fn parses_the_mailcow_banner() {
        let banner = "\"IMPLEMENTATION\" \"Dovecot Pigeonhole\"\r\n\
                      \"SASL\" \"\"\r\n\
                      \"STARTTLS\"\r\n\
                      \"VERSION\" \"1.0\"\r\n\
                      OK \"Dovecot ready.\"\r\n";
        let mut caps = Capabilities::default();
        exchange(banner, async |s| { caps = s.read_capabilities().await.unwrap(); }).await;
        assert!(caps.starttls, "STARTTLS must be detected in mailcow's banner");
        assert!(caps.sasl.is_empty(), "mailcow offers no SASL mechanism before TLS");
    }

    #[tokio::test]
    async fn parses_post_tls_sasl_list() {
        let banner = "\"SASL\" \"PLAIN LOGIN\"\r\nOK \"Dovecot ready.\"\r\n";
        let mut caps = Capabilities::default();
        exchange(banner, async |s| { caps = s.read_capabilities().await.unwrap(); }).await;
        assert_eq!(caps.sasl, vec!["PLAIN".to_string(), "LOGIN".to_string()]);
        assert!(!caps.starttls);
    }

    #[tokio::test]
    async fn authenticate_sends_sasl_plain_and_never_logs_it() {
        let written = exchange("OK \"ready\"\r\nOK \"Logged in.\"\r\n", async |s| {
            s.read_capabilities().await.unwrap();
            s.authenticate_plain("a@b.com", "hunter2").await.unwrap();
        }).await;
        // \0a@b.com\0hunter2 base64-encoded
        assert!(written.contains("AUTHENTICATE \"PLAIN\" \"AGFAYi5jb20AaHVudGVyMg==\""), "got: {written}");
    }

    #[tokio::test]
    async fn a_no_response_is_an_error() {
        let mut result = Ok(());
        exchange("OK \"ready\"\r\nNO \"Authentication failed.\"\r\n", async |s| {
            s.read_capabilities().await.unwrap();
            result = s.authenticate_plain("a@b.com", "wrong").await;
        }).await;
        assert!(result.unwrap_err().contains("Authentication failed"));
    }

    #[tokio::test]
    async fn put_script_uses_a_literal_and_then_activates() {
        let written = exchange("OK \"ready\"\r\nOK \"Putscript ok.\"\r\nOK \"Setactive ok.\"\r\n", async |s| {
            s.read_capabilities().await.unwrap();
            s.put_script("rav-filters", "keep;\r\n").await.unwrap();
            s.set_active("rav-filters").await.unwrap();
        }).await;
        assert!(written.contains("PUTSCRIPT \"rav-filters\" {9+}"), "got: {written}");
        assert!(written.contains("SETACTIVE \"rav-filters\""), "got: {written}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test sieve::protocol -- --nocapture`
Expected: FAIL — `protocol.rs` does not exist yet, so the module fails to compile.

- [ ] **Step 3: Write the implementation**

`backend/src/sieve/protocol.rs`, above the test module:

```rust
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// What the server advertised. RFC 5804 has the server re-issue this list
/// after a successful STARTTLS, which is the only time mailcow names a SASL
/// mechanism at all.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub starttls: bool,
    pub sasl: Vec<String>,
}

/// The ManageSieve protocol over any stream. Deliberately knows nothing about
/// sockets, TLS or config, so the handshake can be tested over an in-memory
/// duplex — including the part that must NOT happen (authenticating before
/// the stream is encrypted).
pub struct Sieve<S> {
    io: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Sieve<S> {
    pub fn new(stream: S) -> Self {
        Sieve { io: BufReader::new(stream) }
    }

    /// Hands the raw stream back so the caller can wrap it in TLS.
    pub fn into_inner(self) -> S {
        self.io.into_inner()
    }

    pub async fn read_capabilities(&mut self) -> Result<Capabilities, String> {
        let mut caps = Capabilities::default();
        loop {
            let line = self.read_line().await?;
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("OK") {
                let _ = rest;
                return Ok(caps);
            }
            if trimmed.starts_with("NO") || trimmed.starts_with("BYE") {
                return Err(format!("ManageSieve: server error: {trimmed}"));
            }
            let mut parts = trimmed.split('"').filter(|p| !p.trim().is_empty());
            match parts.next().map(str::trim) {
                Some("STARTTLS") => caps.starttls = true,
                Some("SASL") => {
                    caps.sasl = parts
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .map(str::to_string)
                        .collect();
                }
                _ => {}
            }
        }
    }

    pub async fn request_starttls(&mut self) -> Result<(), String> {
        self.write_all(b"STARTTLS\r\n").await?;
        self.expect_ok().await
    }

    pub async fn authenticate_plain(&mut self, email: &str, password: &str) -> Result<(), String> {
        let mut creds = vec![0u8];
        creds.extend_from_slice(email.as_bytes());
        creds.push(0);
        creds.extend_from_slice(password.as_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(&creds);
        // The blob carries the password: it is never logged, at any level.
        self.write_all(format!("AUTHENTICATE \"PLAIN\" \"{encoded}\"\r\n").as_bytes()).await?;
        self.expect_ok().await
    }

    pub async fn put_script(&mut self, name: &str, script: &str) -> Result<(), String> {
        let bytes = script.as_bytes();
        self.write_all(format!("PUTSCRIPT \"{name}\" {{{}+}}\r\n", bytes.len()).as_bytes()).await?;
        self.write_all(bytes).await?;
        self.write_all(b"\r\n").await?;
        self.expect_ok().await
    }

    pub async fn set_active(&mut self, name: &str) -> Result<(), String> {
        self.write_all(format!("SETACTIVE \"{name}\"\r\n").as_bytes()).await?;
        self.expect_ok().await
    }

    pub async fn logout(&mut self) {
        let _ = self.write_all(b"LOGOUT\r\n").await;
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.io
            .write_all(bytes)
            .await
            .map_err(|e| format!("ManageSieve: write failed: {e}"))?;
        self.io
            .flush()
            .await
            .map_err(|e| format!("ManageSieve: flush failed: {e}"))
    }

    async fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let n = self
            .io
            .read_line(&mut line)
            .await
            .map_err(|e| format!("ManageSieve: read error: {e}"))?;
        if n == 0 {
            return Err("ManageSieve: connection closed unexpectedly".to_string());
        }
        Ok(line)
    }

    /// Reads until a tagged OK, treating NO/BYE as the error they are and
    /// skipping the untagged lines Dovecot sends in between.
    async fn expect_ok(&mut self) -> Result<(), String> {
        loop {
            let line = self.read_line().await?;
            let trimmed = line.trim();
            if trimmed.starts_with("OK") {
                return Ok(());
            }
            if trimmed.starts_with("NO") || trimmed.starts_with("BYE") {
                return Err(format!("ManageSieve: server error: {trimmed}"));
            }
        }
    }
}
```

Add to `backend/src/sieve/mod.rs`, next to the existing `mod client;`:

```rust
mod protocol;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test sieve::protocol`
Expected: 5 passed. If `AsyncFnOnce` is unavailable on the toolchain, change the helper to take `fn(&mut Sieve<..>) -> BoxFuture<'_, ()>` rather than weakening any assertion.

- [ ] **Step 5: Commit**

```bash
git add backend/src/sieve/protocol.rs backend/src/sieve/mod.rs
git commit -m "feat(sieve): ManageSieve protocol over a generic stream"
```

---

### Task 2: STARTTLS upgrade, and refusing to authenticate without it

**Files:**
- Modify: `backend/src/sieve/client.rs` (full rewrite of `do_push`)
- Modify: `backend/src/config.rs` (add `sieve_allow_plaintext`)
- Modify: `backend/src/sieve/mod.rs` (pass the connector through)
- Test: inline `#[cfg(test)]` in `backend/src/sieve/client.rs`

**Interfaces:**
- Consumes: `Sieve`, `Capabilities` from Task 1.
- Produces: `pub async fn push_script(host: &str, port: u16, tls: &async_native_tls::TlsConnector, allow_plaintext: bool, email: &str, password: &str, script_name: &str, script: &str) -> Result<(), String>`; `AppConfig.sieve_allow_plaintext: bool`.

- [ ] **Step 1: Write the failing test**

Add to `backend/src/sieve/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A server that advertises STARTTLS and then records whatever arrives.
    /// It never completes a TLS handshake — the point is what the client does
    /// BEFORE encryption exists.
    async fn starttls_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(b"\"SASL\" \"\"\r\n\"STARTTLS\"\r\nOK \"ready\"\r\n").await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        seen.extend_from_slice(&buf[..n]);
                        if seen.windows(8).any(|w| w == b"STARTTLS") {
                            sock.write_all(b"OK \"Begin TLS negotiation now.\"\r\n").await.unwrap();
                        }
                    }
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        (addr, handle)
    }

    fn test_connector() -> async_native_tls::TlsConnector {
        async_native_tls::TlsConnector::from(native_tls::TlsConnector::builder())
    }

    #[tokio::test]
    async fn never_authenticates_before_tls() {
        let (addr, handle) = starttls_server().await;
        let result = push_script(
            &addr.ip().to_string(), addr.port(), &test_connector(), false,
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        ).await;
        let written = handle.await.unwrap();
        assert!(written.contains("STARTTLS"), "client must request STARTTLS: {written}");
        assert!(!written.contains("AUTHENTICATE"), "password must never precede TLS: {written}");
        assert!(result.is_err(), "a failed TLS handshake must not fall back to plaintext");
    }

    #[tokio::test]
    async fn refuses_a_server_that_offers_no_starttls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(b"\"SASL\" \"PLAIN\"\r\nOK \"ready\"\r\n").await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = sock.read(&mut buf).await {
                if n == 0 { break; }
                seen.extend_from_slice(&buf[..n]);
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        let result = push_script(
            &addr.ip().to_string(), addr.port(), &test_connector(), false,
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        ).await;
        let written = handle.await.unwrap();
        assert!(!written.contains("AUTHENTICATE"), "no plaintext auth without opt-in: {written}");
        assert!(result.unwrap_err().contains("plaintext"));
    }

    #[tokio::test]
    async fn allows_plaintext_only_when_explicitly_opted_in() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(b"\"SASL\" \"PLAIN\"\r\nOK \"ready\"\r\n").await.unwrap();
            let mut buf = [0u8; 1024];
            let mut seen = Vec::new();
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        seen.extend_from_slice(&buf[..n]);
                        sock.write_all(b"OK \"fine\"\r\n").await.unwrap();
                        if seen.windows(9).any(|w| w == b"SETACTIVE") { break; }
                    }
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        let result = push_script(
            &addr.ip().to_string(), addr.port(), &test_connector(), true,
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        ).await;
        let written = handle.await.unwrap();
        assert!(written.contains("AUTHENTICATE"), "opt-in should authenticate: {written}");
        assert!(result.is_ok(), "{result:?}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test sieve::client`
Expected: FAIL to compile — `push_script` still has the old signature and no TLS parameter.

- [ ] **Step 3: Write the implementation**

Replace the whole body of `backend/src/sieve/client.rs` above the tests:

```rust
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

use super::protocol::Sieve;

/// Upload and activate a Sieve script over ManageSieve (RFC 5804).
///
/// The password is only ever written to an encrypted stream. mailcow
/// advertises `"SASL" ""` until STARTTLS succeeds, so a plaintext attempt
/// would both fail and put a mailbox credential on the public internet;
/// `allow_plaintext` exists solely for a loopback test server.
pub async fn push_script(
    host: &str,
    port: u16,
    tls: &async_native_tls::TlsConnector,
    allow_plaintext: bool,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    timeout(
        Duration::from_secs(15),
        do_push(host, port, tls, allow_plaintext, email, password, script_name, script),
    )
    .await
    .map_err(|_| "ManageSieve: connection timed out".to_string())?
}

async fn do_push(
    host: &str,
    port: u16,
    tls: &async_native_tls::TlsConnector,
    allow_plaintext: bool,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("ManageSieve: connect failed: {e}"))?;

    let mut plain = Sieve::new(tcp);
    let caps = plain.read_capabilities().await?;

    if caps.starttls {
        plain.request_starttls().await?;
        let stream = tls
            .connect(host, plain.into_inner())
            .await
            .map_err(|e| format!("ManageSieve: TLS handshake failed: {e}"))?;
        let mut secure = Sieve::new(stream);
        // RFC 5804: the server re-issues its capabilities after TLS. Read them
        // (mailcow only names a SASL mechanism here) before authenticating.
        secure.read_capabilities().await?;
        return run_session(&mut secure, email, password, script_name, script).await;
    }

    if !allow_plaintext {
        return Err(
            "ManageSieve: server offers no STARTTLS; refusing to authenticate over plaintext"
                .to_string(),
        );
    }
    run_session(&mut plain, email, password, script_name, script).await
}

async fn run_session<S>(
    sieve: &mut Sieve<S>,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    sieve.authenticate_plain(email, password).await?;
    sieve.put_script(script_name, script).await?;
    sieve.set_active(script_name).await?;
    sieve.logout().await;
    Ok(())
}
```

In `backend/src/config.rs`, beside `sieve_port`:

```rust
    /// Allow `AUTHENTICATE` on an unencrypted ManageSieve stream. Only ever
    /// true for a loopback test server: mailcow requires STARTTLS, and a
    /// plaintext login would put a mailbox password on the public internet.
    #[serde(default)]
    pub sieve_allow_plaintext: bool,
```

In `backend/src/sieve/mod.rs`, thread the connector and flag through (the
transport lives in `MailTransport`, which already carries the CA from
`TLS_CA_CERT_PATH`):

```rust
pub async fn push_filters(
    config: &Arc<AppConfig>,
    transport: &crate::mail_transport::MailTransport,
    email: &str,
    password: &str,
    rules: &[FilterRule],
) {
    let Some(ref host) = config.sieve_host else { return; };
    let script = generator::generate_sieve_script(rules);
    if let Err(e) = client::push_script(
        host,
        config.sieve_port,
        &transport.imap_connector,
        config.sieve_allow_plaintext,
        email,
        password,
        "rav-filters",
        &script,
    )
    .await
    {
        tracing::warn!(error = %e, "ManageSieve push failed - filters will apply via IDLE only");
    }
}
```

Update the one caller, `backend/src/routes/filters.rs:41`, to pass the
`MailTransport` extension the handler already has access to; add the
`Extension(transport): Extension<Arc<MailTransport>>` argument if it is not
there yet, matching how `routes/spam.rs` takes its extensions.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test sieve && cargo clippy -- -D warnings`
Expected: all sieve tests pass, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add backend/src/sieve backend/src/config.rs backend/src/routes/filters.rs
git commit -m "feat(sieve): STARTTLS, and never authenticate without it"
```

---

### Task 3: Out-of-office as a server-side Sieve script

**Files:**
- Modify: `backend/src/sieve/generator.rs`
- Modify: `backend/src/sieve/mod.rs`
- Modify: `backend/src/routes/vacation.rs`
- Test: inline `#[cfg(test)]` in `backend/src/sieve/generator.rs`

**Interfaces:**
- Consumes: `push_script` from Task 2; `VacationResponder { enabled, subject, body, start_date: Option<String>, end_date: Option<String>, reply_interval_hours }` from `backend/src/db/vacation.rs:6`.
- Produces: `pub fn generate_script(rules: &[FilterRule], vacation: Option<&VacationResponder>) -> String`; `pub async fn push_state(config, transport, email, password, rules, vacation)`.

- [ ] **Step 1: Write the failing test**

Add to `backend/src/sieve/generator.rs`:

```rust
#[cfg(test)]
mod vacation_tests {
    use super::*;
    use crate::db::vacation::VacationResponder;

    fn responder() -> VacationResponder {
        VacationResponder {
            enabled: true,
            subject: "Out of office".to_string(),
            body: "Back on Monday.".to_string(),
            start_date: None,
            end_date: None,
            reply_interval_hours: 24,
        }
    }

    #[test]
    fn disabled_responder_emits_no_vacation_rule() {
        let mut v = responder();
        v.enabled = false;
        let script = generate_script(&[], Some(&v));
        assert!(!script.contains("vacation"), "got: {script}");
    }

    #[test]
    fn enabled_responder_requires_and_emits_vacation() {
        let script = generate_script(&[], Some(&responder()));
        assert!(script.contains("require"), "got: {script}");
        assert!(script.contains("\"vacation\""), "got: {script}");
        assert!(script.contains(":days 1"), "got: {script}");
        assert!(script.contains(":subject \"Out of office\""), "got: {script}");
        assert!(script.contains("Back on Monday."), "got: {script}");
    }

    #[test]
    fn dates_become_currentdate_guards() {
        let mut v = responder();
        v.start_date = Some("2026-09-20".to_string());
        v.end_date = Some("2026-09-30".to_string());
        let script = generate_script(&[], Some(&v));
        assert!(script.contains("currentdate :value \"ge\" \"date\" \"2026-09-20\""), "got: {script}");
        assert!(script.contains("currentdate :value \"le\" \"date\" \"2026-09-30\""), "got: {script}");
    }

    #[test]
    fn hours_round_up_to_whole_days_and_never_below_one() {
        let mut v = responder();
        v.reply_interval_hours = 30;
        assert!(generate_script(&[], Some(&v)).contains(":days 2"));
        v.reply_interval_hours = 1;
        assert!(generate_script(&[], Some(&v)).contains(":days 1"));
    }

    #[test]
    fn quotes_in_the_subject_are_escaped() {
        let mut v = responder();
        v.subject = "Re: \"urgent\"".to_string();
        let script = generate_script(&[], Some(&v));
        assert!(script.contains(":subject \"Re: \\\"urgent\\\"\""), "got: {script}");
    }

    #[test]
    fn a_body_line_that_is_a_lone_dot_is_stuffed() {
        let mut v = responder();
        v.body = "first\n.\nlast".to_string();
        let script = generate_script(&[], Some(&v));
        assert!(script.contains("\n..\n"), "a lone dot would end the literal early: {script}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test sieve::generator`
Expected: FAIL — `generate_script` does not exist.

- [ ] **Step 3: Write the implementation**

In `backend/src/sieve/generator.rs`:

```rust
use crate::db::vacation::VacationResponder;

/// One script carries everything, because Dovecot activates exactly one per
/// user: the filter rules first, then the responder.
pub fn generate_script(rules: &[FilterRule], vacation: Option<&VacationResponder>) -> String {
    let rules_part = generate_sieve_script(rules);
    let Some(v) = vacation.filter(|v| v.enabled) else {
        return rules_part;
    };

    let days = ((v.reply_interval_hours + 23) / 24).max(1);
    let mut guards = Vec::new();
    if let Some(ref start) = v.start_date {
        guards.push(format!("currentdate :value \"ge\" \"date\" \"{}\"", escape(start)));
    }
    if let Some(ref end) = v.end_date {
        guards.push(format!("currentdate :value \"le\" \"date\" \"{}\"", escape(end)));
    }

    let mut extensions = vec!["\"vacation\""];
    if !guards.is_empty() {
        extensions.push("\"date\"");
        extensions.push("\"relational\"");
        extensions.push("\"comparator-i;ascii-numeric\"");
    }

    let action = format!(
        "vacation :days {days} :subject \"{}\" text:\r\n{}\r\n.\r\n;",
        escape(&v.subject),
        stuff_dots(&v.body),
    );

    let block = if guards.is_empty() {
        action
    } else {
        format!("if allof ({}) {{\r\n  {action}\r\n}}", guards.join(",\r\n          "))
    };

    format!(
        "{rules_part}\r\n# --- out of office (generated by Altacee Mail) ---\r\nrequire [{}];\r\n{block}\r\n",
        extensions.join(", ")
    )
}

/// Sieve quoted strings escape backslash and double quote, nothing else.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// In a `text:` literal a line consisting of a single dot terminates it, so
/// any leading dot is doubled — the same rule SMTP uses.
fn stuff_dots(body: &str) -> String {
    body.lines()
        .map(|l| if l.starts_with('.') { format!(".{l}") } else { l.to_string() })
        .collect::<Vec<_>>()
        .join("\r\n")
}
```

In `backend/src/sieve/mod.rs`, rename the entry point so either edit republishes
the whole script:

```rust
pub async fn push_state(
    config: &Arc<AppConfig>,
    transport: &crate::mail_transport::MailTransport,
    email: &str,
    password: &str,
    rules: &[FilterRule],
    vacation: Option<&crate::db::vacation::VacationResponder>,
) {
    let Some(ref host) = config.sieve_host else { return; };
    let script = generator::generate_script(rules, vacation);
    if let Err(e) = client::push_script(
        host, config.sieve_port, &transport.imap_connector, config.sieve_allow_plaintext,
        email, password, "rav-filters", &script,
    ).await {
        tracing::warn!(error = %e, "ManageSieve push failed - filters and vacation apply in-app only");
    }
}
```

Update both callers to read the other half of the state and pass it:
`routes/filters.rs` reads the vacation row with `db::vacation::get_vacation`,
and `routes/vacation.rs` reads the rules with the same call
`routes/filters.rs` already uses, then both call `push_state`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test sieve && cargo clippy -- -D warnings`
Expected: all pass, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add backend/src/sieve backend/src/routes/filters.rs backend/src/routes/vacation.rs
git commit -m "feat(sieve): out-of-office as a server-side vacation rule"
```

---

### Task 4: Ship it and prove it on the live server

**Files:**
- Modify: `gitops/apps/rav/rav.yaml` (other repo: `~/development/altacee/gitops`)
- Modify: `gitops/apps/rav/README.md`

**Interfaces:**
- Consumes: the image built from this branch.
- Produces: a deployment whose filters and vacation live in Dovecot.

- [ ] **Step 1: Run the whole suite before building anything**

Run: `cd backend && cargo test && cargo clippy -- -D warnings`
Expected: green. A red suite stops the task — do not build an image from it.

- [ ] **Step 2: Build and push the image**

```bash
cd ~/development/altacee/rav
SHA=$(git rev-parse --short HEAD)
git archive --format=tar HEAD | ssh altacee@192.168.13.40 "set -e
  T=\$(mktemp -d); tar -x -C \$T; cd \$T
  docker build -t 100.114.251.94:5000/altacee/rav:$SHA .
  docker push 100.114.251.94:5000/altacee/rav:$SHA
  docker image inspect --format '{{index .RepoDigests 0}}' 100.114.251.94:5000/altacee/rav:$SHA
  cd /; rm -rf \$T"
```

Expected: a `sha256:` digest on the last line. Keep it.

- [ ] **Step 3: Point the cluster at it**

In `gitops/apps/rav/rav.yaml`, bump the image to the new tag and digest, and add
beside the other mail settings:

```yaml
            # Filters and the out-of-office responder are uploaded to Dovecot
            # over ManageSieve, so they run at delivery time whether or not
            # anyone has the client open. STARTTLS is mandatory: mailcow
            # advertises no SASL mechanism before it.
            - { name: SIEVE_HOST, value: ryuvzdff.altacee.com }
            - { name: SIEVE_PORT, value: "4190" }
```

Commit on a branch, open a PR, merge it, then:

```bash
kubectl -n argocd annotate application rav argocd.argoproj.io/refresh=hard --overwrite
kubectl -n rav get pods -l app=rav -w
```

Expected: a new pod Running 1/1, serving the digest from Step 2 (check with
`kubectl -n rav get pod -l app=rav -o jsonpath='{.items[0].status.containerStatuses[0].imageID}'`).

- [ ] **Step 4: Verify against mailcow, not against the app**

1. Sign in to `https://webmail.altacee.com`, create a filter, save it.
2. In mailcow's own UI for that mailbox, confirm a `rav-filters` script exists and is active.
3. Enable the out-of-office responder with a subject you can recognise.
4. Re-check the script in mailcow: it must now contain the `vacation` rule.
5. **Close every Altacee Mail tab.** From another account, send a message that
   matches the filter. Confirm it is filed into the target folder and that the
   auto-reply arrives — with the client shut, which is the whole point.
6. Check the pod logs for `ManageSieve push failed`; there should be none.

If step 5 files nothing, the script is present but not active — check
`SETACTIVE` succeeded rather than assuming the filter is wrong.

- [ ] **Step 5: Record it**

Append to `~/development/altacee/infra/lab-cluster/RUNLOG.md`: what shipped, the
image digest, and what step 5 actually showed.

```bash
cd ~/development/altacee/gitops && git add apps/rav && git commit -m "deploy: rav <sha> (server-side Sieve)"
```

## Self-Review

- **Spec coverage.** Item 1 of the spec is covered by Tasks 1–3 (STARTTLS, fail-closed, vacation) and Task 4 (config, live proof). Items 2–6 are out of scope for this plan by design; each gets its own.
- **Placeholders.** None: every step carries the code or command it needs.
- **Type consistency.** `Sieve<S>`, `Capabilities`, `push_script`, `generate_script` and `push_state` keep the same signatures across Tasks 1–3, and `VacationResponder`'s field names match `backend/src/db/vacation.rs:6-16`.
- **Known risk.** `run_session` is generic over the stream, so the TLS path and the loopback path share one code path; the TLS handshake itself has no unit test and is proved in Task 4, step 5. That is deliberate — a self-signed test CA would test our own fixture, not mailcow.
