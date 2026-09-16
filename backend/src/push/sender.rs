use base64::Engine;
use web_push::{
    ContentEncoding, SubscriptionInfo, SubscriptionKeys, VapidSignatureBuilder, WebPushMessageBuilder,
};

use crate::db::push::PushSubscription;

/// What a viewer sees on the lock screen. Sender and subject only — the message
/// body never leaves the server for a push service to store.
pub struct Notification {
    pub sender: String,
    pub subject: String,
    pub folder: String,
}

impl Notification {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "sender": self.sender,
            "subject": self.subject,
            "folder": self.folder,
        })
        .to_string()
    }
}

/// The VAPID identity this server pushes with. `subject` is an https URL rather
/// than a mailto: so no personal address is baked into every notification.
pub struct Vapid {
    private_base64: String,
    subject: String,
}

impl Vapid {
    pub fn new(private_base64: String, subject: String) -> Self {
        Vapid { private_base64, subject }
    }
}

#[derive(Debug)]
pub enum SendError {
    /// The push service says this endpoint is dead: 404 or 410. The caller
    /// deletes the subscription rather than retrying it forever.
    Gone,
    Other(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Gone => write!(f, "subscription is gone"),
            SendError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// web-push asserts the private key's length deep inside `generic-array`, so a
/// key of the wrong size panics rather than erroring. Check it first: bad
/// configuration must disable push, never take down the process.
fn is_valid_private_key(private_base64: &str) -> bool {
    let trimmed = private_base64.trim();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(trimmed));
    matches!(decoded, Ok(bytes) if bytes.len() == 32)
}

/// The application server key the browser needs when it subscribes.
/// `None` when the configured private key does not parse — which is treated as
/// "push is not configured", never as a panic at startup.
pub fn public_key_base64(private_base64: &str) -> Option<String> {
    if !is_valid_private_key(private_base64) {
        return None;
    }
    let builder = VapidSignatureBuilder::from_base64_no_sub(private_base64).ok()?;
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(builder.get_public_key()))
}

/// Encrypt and deliver one notification to one browser endpoint.
pub async fn send(
    http: &reqwest::Client,
    vapid: &Vapid,
    subscription: &PushSubscription,
    notification: &Notification,
) -> Result<(), SendError> {
    let info = SubscriptionInfo {
        endpoint: subscription.endpoint.clone(),
        keys: SubscriptionKeys {
            p256dh: subscription.p256dh.clone(),
            auth: subscription.auth.clone(),
        },
    };

    if !is_valid_private_key(&vapid.private_base64) {
        return Err(SendError::Other(
            "PUSH_VAPID_KEY is not 32 bytes of base64 - refusing to sign".to_string(),
        ));
    }
    let mut signature = VapidSignatureBuilder::from_base64(&vapid.private_base64, &info)
        .map_err(|e| SendError::Other(format!("VAPID key: {e}")))?;
    signature.add_claim("sub", vapid.subject.clone());
    let signature = signature
        .build()
        .map_err(|e| SendError::Other(format!("VAPID signature: {e}")))?;

    let payload = notification.to_json();
    let mut message = WebPushMessageBuilder::new(&info);
    message.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
    message.set_vapid_signature(signature);
    // A notification about new mail is worthless an hour later.
    message.set_ttl(600);
    let message = message
        .build()
        .map_err(|e| SendError::Other(format!("build push message: {e}")))?;

    let mut request = http.post(message.endpoint.to_string()).header("TTL", message.ttl);
    let body = match message.payload {
        Some(payload) => {
            for (name, value) in payload.crypto_headers.into_iter() {
                request = request.header(name, value);
            }
            request = request
                .header("Content-Encoding", payload.content_encoding.to_str())
                .header("Content-Type", "application/octet-stream");
            payload.content
        }
        None => Vec::new(),
    };

    let response = request
        .body(body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| SendError::Other(format!("push service unreachable: {e}")))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
        return Err(SendError::Gone);
    }
    if !status.is_success() {
        return Err(SendError::Other(format!("push service returned {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Any 32 bytes below the curve order is a valid P-256 private key; this
    /// one exists only in this test file.
    const TEST_VAPID_KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";

    fn subscription(endpoint: &str) -> crate::db::push::PushSubscription {
        crate::db::push::PushSubscription {
            endpoint: endpoint.to_string(),
            // A throwaway P-256 point and auth secret generated for this test.
            p256dh: "BP3ayfxj36KQOPimdVxaWQCwFCf6RdZnospyE0lsc90INP29IDBS0QY21uAG2-9PR2xQHHRUeLfC7FBXvKWMuxw".to_string(),
            auth: "kbjpiK6xdetaM8P6QOBPEA".to_string(),
        }
    }

    async fn stub(status: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            // Bounded: when send() fails before connecting, accept() would
            // otherwise never return and the test would deadlock on its own
            // join handle rather than reporting the real failure.
            let accepted = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept()).await;
            let Ok(Ok((mut sock, _))) = accepted else {
                return String::from("<no connection>");
            };
            let mut buf = vec![0u8; 16384];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let seen = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = sock
                .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
                .await;
            seen
        });
        (format!("http://{addr}/push"), handle)
    }

    #[test]
    fn the_payload_never_carries_the_message_body() {
        let json = Notification {
            sender: "Dave".to_string(),
            subject: "Lunch?".to_string(),
            folder: "INBOX".to_string(),
        }
        .to_json();
        assert!(json.contains("Lunch?"), "subject is wanted: {json}");
        assert!(json.contains("Dave"));
        assert!(!json.to_lowercase().contains("body"), "no message body leaves the server: {json}");
    }

    #[test]
    fn a_public_key_can_be_derived_for_the_browser() {
        let public = public_key_base64(TEST_VAPID_KEY).expect("test key should parse");
        assert!(public.len() > 80, "an uncompressed P-256 point, base64url: {public}");
        assert!(!public.contains('+') && !public.contains('/'), "must be url-safe: {public}");
    }

    #[test]
    fn a_nonsense_key_is_not_configured_rather_than_a_panic() {
        assert!(public_key_base64("not-a-key").is_none());
    }

    #[test]
    fn a_wrong_length_key_is_rejected_before_the_crate_panics() {
        // web-push asserts the scalar length inside generic-array, so a 31- or
        // 37-byte key would abort the worker rather than return an error.
        let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 31]);
        let long = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 37]);
        assert!(public_key_base64(&short).is_none());
        assert!(public_key_base64(&long).is_none());
    }

    #[tokio::test]
    async fn a_410_means_the_subscription_is_gone() {
        let (endpoint, handle) = stub("410 Gone").await;
        let vapid = Vapid::new(TEST_VAPID_KEY.to_string(), "https://webmail.altacee.com".to_string());
        let err = send(&reqwest::Client::new(), &vapid, &subscription(&endpoint), &Notification {
            sender: "Dave".to_string(),
            subject: "Lunch?".to_string(),
            folder: "INBOX".to_string(),
        })
        .await
        .unwrap_err();
        let _ = handle.await;
        assert!(matches!(err, SendError::Gone), "got: {err:?}");
    }

    #[tokio::test]
    async fn a_successful_send_carries_an_encrypted_body_and_a_vapid_header() {
        let (endpoint, handle) = stub("201 Created").await;
        let vapid = Vapid::new(TEST_VAPID_KEY.to_string(), "https://webmail.altacee.com".to_string());
        let result = send(&reqwest::Client::new(), &vapid, &subscription(&endpoint), &Notification {
            sender: "Dave".to_string(),
            subject: "Lunch?".to_string(),
            folder: "INBOX".to_string(),
        })
        .await;
        let seen = handle.await.unwrap();
        assert!(result.is_ok(), "{result:?}");
        let lower = seen.to_lowercase();
        assert!(lower.contains("authorization: vapid"), "got: {seen}");
        assert!(lower.contains("content-encoding: aes128gcm"), "got: {seen}");
        assert!(!seen.contains("Lunch?"), "the payload must be encrypted, not plaintext");
    }
}
