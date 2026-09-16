use serde::Deserialize;

/// Where mailcow is and how to authenticate to it.
pub struct MailcowApi {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl MailcowApi {
    pub fn new(http: reqwest::Client, base_url: String, api_key: String) -> Self {
        MailcowApi { http, base_url: base_url.trim_end_matches('/').to_string(), api_key }
    }
}

/// One row of `GET /api/v1/get/alias/all`. Only the three fields we use;
/// serde ignores the rest, so a mailcow upgrade that adds fields is harmless.
#[derive(Debug, Deserialize)]
struct Alias {
    address: String,
    /// Delivery targets, comma-separated when an alias fans out.
    goto: String,
    #[serde(default)]
    active: i32,
}

/// The active aliases that deliver to `mailbox`, sorted, as send-as candidates.
///
/// Returns an empty list for every failure — no key, a rejected key, mailcow
/// down, a body that will not parse. Someone reading their mail must never be
/// blocked by the admin API being unavailable.
pub async fn aliases_for(api: &MailcowApi, mailbox: &str) -> Vec<String> {
    let url = format!("{}/api/v1/get/alias/all", api.base_url);
    let response = match api
        .http
        .get(&url)
        .header("X-API-Key", &api.api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), "mailcow alias lookup refused");
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!(error = %e, "mailcow unreachable");
            return Vec::new();
        }
    };

    let aliases: Vec<Alias> = match response.json().await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "mailcow alias response did not parse");
            return Vec::new();
        }
    };

    let mailbox = mailbox.to_lowercase();
    let mut out: Vec<String> = aliases
        .into_iter()
        .filter(|a| a.active != 0)
        .filter(|a| {
            a.goto
                .split(',')
                .any(|target| target.trim().eq_ignore_ascii_case(&mailbox))
        })
        .map(|a| a.address)
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A stub mailcow that records the request and answers with `body`.
    async fn stub(status: &'static str, body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let seen = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = sock
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
            seen
        });
        (format!("http://{addr}"), handle)
    }

    const ALIASES: &str = r#"[
        {"address":"sales@altacee.com","goto":"naveen@altacee.com","active":1},
        {"address":"old@altacee.com","goto":"naveen@altacee.com","active":0},
        {"address":"someone@altacee.com","goto":"other@altacee.com","active":1},
        {"address":"billing@altacee.com","goto":"naveen@altacee.com, ops@altacee.com","active":1}
    ]"#;

    #[tokio::test]
    async fn sends_the_api_key_and_returns_matching_active_aliases() {
        let (url, handle) = stub("200 OK", ALIASES).await;
        let api = MailcowApi::new(reqwest::Client::new(), url, "k3y".to_string());
        let aliases = aliases_for(&api, "naveen@altacee.com").await;
        let seen = handle.await.unwrap();
        assert!(seen.to_lowercase().contains("x-api-key: k3y"), "got: {seen}");
        assert!(seen.starts_with("GET /api/v1/get/alias/all"), "got: {seen}");
        assert_eq!(
            aliases,
            vec!["billing@altacee.com".to_string(), "sales@altacee.com".to_string()],
            "inactive aliases and other mailboxes' aliases must be left out, and a \
             multi-target goto still counts"
        );
    }

    #[tokio::test]
    async fn an_unauthorised_key_yields_nothing_rather_than_an_error() {
        let (url, handle) = stub("401 Unauthorized", "{}").await;
        let api = MailcowApi::new(reqwest::Client::new(), url, "wrong".to_string());
        let aliases = aliases_for(&api, "naveen@altacee.com").await;
        let _ = handle.await;
        assert!(aliases.is_empty(), "a bad key must not break sign-in");
    }

    #[tokio::test]
    async fn an_unreachable_mailcow_yields_nothing() {
        let api = MailcowApi::new(
            reqwest::Client::new(),
            "http://127.0.0.1:1".to_string(),
            "k".to_string(),
        );
        assert!(aliases_for(&api, "naveen@altacee.com").await.is_empty());
    }

    #[tokio::test]
    async fn matching_is_case_insensitive() {
        let (url, handle) = stub("200 OK", ALIASES).await;
        let api = MailcowApi::new(reqwest::Client::new(), url, "k".to_string());
        let aliases = aliases_for(&api, "NAVEEN@Altacee.com").await;
        let _ = handle.await;
        assert_eq!(aliases.len(), 2, "a mailbox is the same mailbox in any case");
    }
}
