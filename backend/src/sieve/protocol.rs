use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// What the server advertised. RFC 5804 has the server re-issue this list after
/// a successful STARTTLS, which is the only point at which mailcow names a SASL
/// mechanism at all — before TLS it sends `"SASL" ""`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub starttls: bool,
    pub sasl: Vec<String>,
}

/// The ManageSieve protocol over any stream.
///
/// It deliberately knows nothing about sockets, TLS or config, so the handshake
/// can be driven over an in-memory duplex in tests — including the part that
/// must *not* happen, authenticating before the stream is encrypted.
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
            if trimmed.starts_with("OK") {
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
        // This blob carries the password. It is never logged, at any level.
        self.write_all(format!("AUTHENTICATE \"PLAIN\" \"{encoded}\"\r\n").as_bytes())
            .await?;
        self.expect_ok().await
    }

    pub async fn put_script(&mut self, name: &str, script: &str) -> Result<(), String> {
        let bytes = script.as_bytes();
        self.write_all(format!("PUTSCRIPT \"{name}\" {{{}+}}\r\n", bytes.len()).as_bytes())
            .await?;
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

    /// Reads until a tagged OK, treating NO/BYE as the error it is and skipping
    /// the untagged lines Dovecot sends in between.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Spawns a fake server that says `says` and records everything written to
    /// it, so a test can assert on what the client did — including what it must
    /// never do before the stream is encrypted.
    fn fake_server(
        mut sock: tokio::io::DuplexStream,
        says: &str,
    ) -> tokio::task::JoinHandle<String> {
        let says = says.to_string();
        tokio::spawn(async move {
            sock.write_all(says.as_bytes()).await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        })
    }

    #[tokio::test]
    async fn parses_the_mailcow_banner() {
        let (client, server) = tokio::io::duplex(8192);
        let banner = "\"IMPLEMENTATION\" \"Dovecot Pigeonhole\"\r\n\
                      \"SASL\" \"\"\r\n\
                      \"STARTTLS\"\r\n\
                      \"VERSION\" \"1.0\"\r\n\
                      OK \"Dovecot ready.\"\r\n";
        let _server = fake_server(server, banner);
        let mut sieve = Sieve::new(client);
        let caps = sieve.read_capabilities().await.unwrap();
        assert!(caps.starttls, "STARTTLS must be detected in mailcow's banner");
        assert!(caps.sasl.is_empty(), "mailcow offers no SASL mechanism before TLS");
    }

    #[tokio::test]
    async fn parses_post_tls_sasl_list() {
        let (client, server) = tokio::io::duplex(8192);
        let _server = fake_server(server, "\"SASL\" \"PLAIN LOGIN\"\r\nOK \"Dovecot ready.\"\r\n");
        let mut sieve = Sieve::new(client);
        let caps = sieve.read_capabilities().await.unwrap();
        assert_eq!(caps.sasl, vec!["PLAIN".to_string(), "LOGIN".to_string()]);
        assert!(!caps.starttls);
    }

    #[tokio::test]
    async fn authenticate_sends_sasl_plain() {
        let (client, server) = tokio::io::duplex(8192);
        let handle = fake_server(server, "OK \"ready\"\r\nOK \"Logged in.\"\r\n");
        let mut sieve = Sieve::new(client);
        sieve.read_capabilities().await.unwrap();
        sieve.authenticate_plain("a@b.com", "hunter2").await.unwrap();
        drop(sieve);
        let written = handle.await.unwrap();
        // base64 of \0a@b.com\0hunter2
        assert!(
            written.contains("AUTHENTICATE \"PLAIN\" \"AGFAYi5jb20AaHVudGVyMg==\""),
            "got: {written}"
        );
    }

    #[tokio::test]
    async fn a_no_response_is_an_error() {
        let (client, server) = tokio::io::duplex(8192);
        let _server = fake_server(server, "OK \"ready\"\r\nNO \"Authentication failed.\"\r\n");
        let mut sieve = Sieve::new(client);
        sieve.read_capabilities().await.unwrap();
        let err = sieve.authenticate_plain("a@b.com", "wrong").await.unwrap_err();
        assert!(err.contains("Authentication failed"), "got: {err}");
    }

    #[tokio::test]
    async fn put_script_uses_a_literal_and_then_activates() {
        let (client, server) = tokio::io::duplex(8192);
        let handle = fake_server(
            server,
            "OK \"ready\"\r\nOK \"Putscript ok.\"\r\nOK \"Setactive ok.\"\r\n",
        );
        let mut sieve = Sieve::new(client);
        sieve.read_capabilities().await.unwrap();
        sieve.put_script("rav-filters", "keep;\r\n").await.unwrap();
        sieve.set_active("rav-filters").await.unwrap();
        drop(sieve);
        let written = handle.await.unwrap();
        assert!(written.contains("PUTSCRIPT \"rav-filters\" {7+}"), "got: {written}");
        assert!(written.contains("SETACTIVE \"rav-filters\""), "got: {written}");
    }
}
