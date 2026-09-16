use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

use super::protocol::Sieve;

/// Upload and activate a Sieve script over ManageSieve (RFC 5804).
///
/// The password is only ever written to an encrypted stream. mailcow advertises
/// `"SASL" ""` until STARTTLS succeeds, so a plaintext attempt would both fail
/// and put a mailbox credential on the public internet. `allow_plaintext`
/// exists solely for a loopback test server.
/// Where to push, and on what terms. Bundled because the crate forbids
/// long argument lists, and these four always travel together.
pub struct SieveTarget<'a> {
    pub host: &'a str,
    pub port: u16,
    pub tls: &'a async_native_tls::TlsConnector,
    /// Only ever true for a loopback test server. See `push_script`.
    pub allow_plaintext: bool,
}

pub async fn push_script(
    target: SieveTarget<'_>,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    timeout(
        Duration::from_secs(15),
        do_push(target, email, password, script_name, script),
    )
    .await
    .map_err(|_| "ManageSieve: connection timed out".to_string())?
}

async fn do_push(
    target: SieveTarget<'_>,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    let SieveTarget { host, port, tls, allow_plaintext } = target;
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
        // RFC 5804: the server re-issues its capabilities after TLS, and that is
        // the first time mailcow names a SASL mechanism. Read them, then log in.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A loopback ManageSieve server that answers OK to everything and records
    /// what arrived. `banner` decides whether it advertises STARTTLS; the TLS
    /// handshake itself is never completed, because these tests are about what
    /// the client does *before* encryption exists.
    async fn fake_server(banner: &'static str) -> (String, u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(banner.as_bytes()).await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        seen.extend_from_slice(&buf[..n]);
                        if sock.write_all(b"OK \"fine\"\r\n").await.is_err() {
                            break;
                        }
                        // Hang up right after acknowledging STARTTLS: the client
                        // then fails its handshake immediately instead of sitting
                        // out the 15s timeout, and the assertion is the same.
                        if seen.windows(8).any(|w| w == b"STARTTLS")
                            || seen.windows(9).any(|w| w == b"SETACTIVE")
                        {
                            break;
                        }
                    }
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        (addr.ip().to_string(), addr.port(), handle)
    }

    fn test_connector() -> async_native_tls::TlsConnector {
        async_native_tls::TlsConnector::from(native_tls::TlsConnector::builder())
    }

    const OFFERS_STARTTLS: &str = "\"SASL\" \"\"\r\n\"STARTTLS\"\r\nOK \"ready\"\r\n";
    const NO_STARTTLS: &str = "\"SASL\" \"PLAIN\"\r\nOK \"ready\"\r\n";

    #[tokio::test]
    async fn never_authenticates_before_tls() {
        let (host, port, handle) = fake_server(OFFERS_STARTTLS).await;
        let result = push_script(
            SieveTarget { host: &host, port, tls: &test_connector(), allow_plaintext: false },
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        )
        .await;
        let written = handle.await.unwrap();
        assert!(written.contains("STARTTLS"), "client must request STARTTLS: {written}");
        assert!(!written.contains("AUTHENTICATE"), "password must never precede TLS: {written}");
        assert!(result.is_err(), "a failed TLS handshake must not fall back to plaintext");
    }

    #[tokio::test]
    async fn refuses_a_server_that_offers_no_starttls() {
        let (host, port, handle) = fake_server(NO_STARTTLS).await;
        let result = push_script(
            SieveTarget { host: &host, port, tls: &test_connector(), allow_plaintext: false },
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        )
        .await;
        let err = result.unwrap_err();
        let written = handle.await.unwrap();
        assert!(!written.contains("AUTHENTICATE"), "no plaintext auth without opt-in: {written}");
        assert!(err.contains("plaintext"), "got: {err}");
    }

    #[tokio::test]
    async fn allows_plaintext_only_when_explicitly_opted_in() {
        let (host, port, handle) = fake_server(NO_STARTTLS).await;
        let result = push_script(
            SieveTarget { host: &host, port, tls: &test_connector(), allow_plaintext: true },
            "a@b.com", "hunter2", "rav-filters", "keep;\r\n",
        )
        .await;
        let written = handle.await.unwrap();
        assert!(written.contains("AUTHENTICATE"), "opt-in should authenticate: {written}");
        assert!(written.contains("SETACTIVE"), "opt-in should activate the script: {written}");
        assert!(result.is_ok(), "{result:?}");
    }
}
