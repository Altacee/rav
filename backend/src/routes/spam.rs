use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::folder_cipher::FolderId;
use crate::imap::client::{ImapClient, ImapCredentials};

/// `POST /api/messages/{folder}/{uid}/report-spam`
///
/// Fetches the raw RFC 822 bytes of the message and POSTs them to rspamd's
/// /learnspam endpoint. The frontend is responsible for moving the message to
/// Junk separately via the existing move API.
///
/// Returns 200 with `{ "trained": true }` when rspamd was called successfully,
/// or `{ "trained": false }` when RSPAMD_URL is not configured (not an error).
pub async fn report_spam_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(http_client): Extension<Arc<reqwest::Client>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let trained = learn_message(&session, &config, &imap_client, &http_client, &folder, uid, "learnspam").await?;
    Ok(Json(serde_json::json!({ "trained": trained })).into_response())
}

/// `POST /api/messages/{folder}/{uid}/report-ham`
///
/// Same as report-spam but teaches rspamd that this message is legitimate.
/// The frontend is responsible for moving the message to Inbox separately.
pub async fn report_ham_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(http_client): Extension<Arc<reqwest::Client>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let trained = learn_message(&session, &config, &imap_client, &http_client, &folder, uid, "learnham").await?;
    Ok(Json(serde_json::json!({ "trained": trained })).into_response())
}

/// Shared implementation: fetch raw bytes then POST to rspamd.
/// Returns true if rspamd was contacted, false if RSPAMD_URL is not set.
async fn learn_message(
    session: &SessionState,
    config: &AppConfig,
    imap_client: &Arc<dyn ImapClient>,
    http_client: &reqwest::Client,
    folder: &str,
    uid: u32,
    endpoint: &str,
) -> Result<bool, AppError> {
    let Some(ref rspamd_url) = config.rspamd_url else {
        return Ok(false);
    };

    let creds = ImapCredentials {
        host: session.imap_host.clone(),
        port: session.imap_port,
        tls: session.imap_tls,
        email: session.email.clone(),
        password: session.password.clone(),
    };

    let raw = imap_client
        .fetch_raw_bytes(&creds, folder, uid)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    post_learn(
        http_client,
        rspamd_url,
        config.rspamd_password.as_deref(),
        endpoint,
        raw,
    )
    .await?;
    Ok(true)
}

/// POST a raw message to one of rspamd's learn endpoints.
///
/// mailcow's rspamd controller is behind authentication — `/rspamd/stat` and
/// `/rspamd/auth` both answer 401 — so without `RSPAMD_PASSWORD` the training
/// silently does nothing useful. The 401 case says so by name rather than
/// surfacing as a bare status code.
async fn post_learn(
    http_client: &reqwest::Client,
    rspamd_url: &str,
    password: Option<&str>,
    endpoint: &str,
    raw: Vec<u8>,
) -> Result<(), AppError> {
    let url = format!("{}/{endpoint}", rspamd_url.trim_end_matches('/'));
    let mut request = http_client
        .post(&url)
        .header("Content-Type", "message/rfc822");
    if let Some(password) = password {
        // rspamd's own header name. Never logged.
        request = request.header("Password", password);
    }
    let resp = request
        .body(raw)
        .send()
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("rspamd unreachable: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AppError::ServiceUnavailable(format!(
            "rspamd rejected the credential ({status}) - check RSPAMD_PASSWORD"
        )));
    }
    if !status.is_success() {
        return Err(AppError::ServiceUnavailable(format!(
            "rspamd returned {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A stub rspamd controller: records the request it received and answers
    /// with `status`.
    async fn stub_rspamd(status: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let seen = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = sock
                .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
                .await;
            seen
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn the_controller_password_is_sent() {
        let (url, handle) = stub_rspamd("200 OK").await;
        let http = reqwest::Client::new();
        let result = post_learn(&http, &url, Some("s3cret"), "learnspam", b"From: a\r\n\r\nhi".to_vec()).await;
        let seen = handle.await.unwrap();
        assert!(result.is_ok(), "{result:?}");
        assert!(seen.contains("password: s3cret") || seen.contains("Password: s3cret"), "got: {seen}");
        assert!(seen.contains("content-type: message/rfc822") || seen.contains("Content-Type: message/rfc822"), "got: {seen}");
        assert!(seen.starts_with("POST /learnspam"), "got: {seen}");
    }

    #[tokio::test]
    async fn no_password_configured_means_no_header() {
        let (url, handle) = stub_rspamd("200 OK").await;
        let http = reqwest::Client::new();
        let _ = post_learn(&http, &url, None, "learnham", b"x".to_vec()).await;
        let seen = handle.await.unwrap();
        assert!(!seen.to_lowercase().contains("password:"), "got: {seen}");
    }

    #[tokio::test]
    async fn a_401_says_the_password_is_the_problem() {
        let (url, handle) = stub_rspamd("401 Unauthorized").await;
        let http = reqwest::Client::new();
        let err = post_learn(&http, &url, Some("wrong"), "learnspam", b"x".to_vec())
            .await
            .unwrap_err();
        let _ = handle.await;
        let msg = format!("{err:?}");
        assert!(msg.contains("RSPAMD_PASSWORD"), "the error must name the fix, got: {msg}");
    }

    #[tokio::test]
    async fn a_trailing_slash_on_the_url_does_not_double_up() {
        let (url, handle) = stub_rspamd("200 OK").await;
        let http = reqwest::Client::new();
        let _ = post_learn(&http, &format!("{url}/"), None, "learnspam", b"x".to_vec()).await;
        let seen = handle.await.unwrap();
        assert!(seen.starts_with("POST /learnspam"), "got: {seen}");
    }
}
