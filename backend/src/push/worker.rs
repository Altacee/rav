use std::sync::Arc;

use tokio::sync::Notify;

use crate::config::AppConfig;
use crate::db;
use crate::imap::types::ImapCredentials;
use crate::mail_transport::MailTransport;
use crate::realtime::events::{EventBus, MailEvent};
use crate::realtime::idle::IdleManager;
use crate::realtime::worker::SyncWorkerManager;

use super::credential;
use super::sender::{self, Notification, Vapid};

/// How often the data directory is rescanned for credentials added or removed
/// while the process was busy. The notify handle makes the common case
/// immediate; this is the safety net.
const RESCAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

/// Everything the push worker needs. All of it already exists in `main`: push
/// adds no new IMAP machinery, it just keeps the existing IDLE alive for users
/// who asked for notifications and turns `NewMail` into a push.
pub struct PushContext {
    pub config: Arc<AppConfig>,
    pub imap_client: Arc<dyn crate::imap::client::ImapClient>,
    pub db_pool_manager: Arc<db::pool::DbPoolManager>,
    pub transport: Arc<MailTransport>,
    pub event_bus: Arc<EventBus>,
    pub idle_manager: Arc<IdleManager>,
    pub sync_worker_manager: Arc<SyncWorkerManager>,
    pub http: Arc<reqwest::Client>,
}

/// The user hashes whose mailbox has a usable stored credential.
///
/// A credential marked invalid is skipped: a password the server rejected must
/// not be retried in a loop against the mail server.
pub async fn users_with_push(
    data_dir: &str,
    db_pool_manager: &Arc<db::pool::DbPoolManager>,
) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // User directories are the hex SHA-256 of an address; anything else in
        // the data directory is not a mailbox.
        if name.len() != 64 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if !entry.path().join("db.sqlite").exists() {
            continue;
        }
        let has_credential = db::pool::with_user_db(db_pool_manager, &name, |conn| {
            Ok(db::push::get_credential(conn)?
                .filter(|c| c.invalid_since.is_none())
                .is_some())
        })
        .await
        .unwrap_or(false);
        if has_credential {
            found.push(name);
        }
    }
    found.sort();
    found
}

/// Start the push worker. The returned handle makes a credential change take
/// effect at once rather than at the next rescan.
pub fn spawn(ctx: PushContext) -> Arc<Notify> {
    let wake = Arc::new(Notify::new());
    let Some(key) = ctx
        .config
        .push_credential_key
        .as_deref()
        .and_then(credential::parse_key)
    else {
        tracing::info!("push is not configured (PUSH_CREDENTIAL_KEY unset or not 32 bytes)");
        return wake;
    };
    let Some(vapid_private) = ctx.config.push_vapid_key.clone() else {
        tracing::info!("push is not configured (PUSH_VAPID_KEY unset)");
        return wake;
    };
    if sender::public_key_base64(&vapid_private).is_none() {
        tracing::warn!("PUSH_VAPID_KEY is not a 32-byte base64 key - push stays off");
        return wake;
    }

    let subject = ctx
        .config
        .push_vapid_subject
        .clone()
        .or_else(|| ctx.config.webauthn_rp_origin.clone())
        .unwrap_or_else(|| "https://localhost".to_string());

    let woken = Arc::clone(&wake);
    tokio::spawn(async move {
        let vapid = Arc::new(Vapid::new(vapid_private, subject));
        let running: Arc<dashmap::DashMap<String, ()>> = Arc::new(dashmap::DashMap::new());
        loop {
            for user_hash in users_with_push(&ctx.config.data_dir, &ctx.db_pool_manager).await {
                if running.contains_key(&user_hash) {
                    continue;
                }
                if let Some(creds) = decrypt_credentials(&ctx, &key, &user_hash).await
                    && credential_still_works(
                        &ctx.imap_client,
                        &ctx.db_pool_manager,
                        &user_hash,
                        &creds,
                    )
                    .await
                {
                    running.insert(user_hash.clone(), ());
                    start_for_user(&ctx, Arc::clone(&vapid), Arc::clone(&running), user_hash, creds)
                        .await;
                }
            }
            tokio::select! {
                _ = woken.notified() => {}
                _ = tokio::time::sleep(RESCAN_INTERVAL) => {}
            }
        }
    });

    wake
}

/// Read a user's stored credential and unseal it into IMAP credentials.
async fn decrypt_credentials(
    ctx: &PushContext,
    key: &[u8; 32],
    user_hash: &str,
) -> Option<ImapCredentials> {
    let stored = db::pool::with_user_db(&ctx.db_pool_manager, user_hash, |conn| {
        db::push::get_credential(conn)
    })
    .await
    .ok()??;

    match credential::open(key, &stored.nonce, &stored.ciphertext) {
        Ok(password) => Some(ImapCredentials {
            host: stored.imap_host,
            port: stored.imap_port,
            tls: true,
            email: stored.email,
            password,
        }),
        Err(e) => {
            // Almost always a rotated PUSH_CREDENTIAL_KEY. Mark it so the user
            // is asked for the app password again instead of silently never
            // being notified.
            tracing::warn!(error = %e, "stored push credential did not decrypt");
            let _ = db::pool::with_user_db(&ctx.db_pool_manager, user_hash, db::push::mark_invalid)
                .await;
            None
        }
    }
}

/// Probe the stored credential once before putting it to work.
///
/// A password the server rejects is marked invalid and never retried: without
/// this, a revoked app password would have the worker reconnecting against
/// mailcow forever, which is both useless and rude. Anything other than an
/// authentication failure — mailcow down, a network blip — is left alone, so a
/// transient outage does not disable someone's notifications.
async fn credential_still_works(
    imap_client: &Arc<dyn crate::imap::client::ImapClient>,
    db_pool_manager: &Arc<db::pool::DbPoolManager>,
    user_hash: &str,
    creds: &ImapCredentials,
) -> bool {
    match imap_client.list_folders(creds).await {
        Ok(_) => true,
        Err(crate::imap::error::ImapError::AuthenticationFailed) => {
            tracing::info!("stored push credential was rejected - marking it invalid");
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, db::push::mark_invalid).await;
            false
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not verify stored push credential yet");
            false
        }
    }
}

/// Keep one user's INBOX under IDLE and turn `NewMail` into a push.
///
/// `NewMail` is the right event: it fires after filter rules and the vacation
/// responder have run, so a message a rule filed elsewhere never becomes a
/// notification.
async fn start_for_user(
    ctx: &PushContext,
    vapid: Arc<Vapid>,
    running: Arc<dashmap::DashMap<String, ()>>,
    user_hash: String,
    creds: ImapCredentials,
) {
    ctx.sync_worker_manager
        .ensure_worker(user_hash.clone(), creds.clone());
    ctx.idle_manager
        .start_idle(
            user_hash.clone(),
            "INBOX".to_string(),
            creds,
            Arc::clone(&ctx.transport),
            Arc::clone(&ctx.sync_worker_manager),
        )
        .await;

    let mut rx = ctx.event_bus.subscribe(&user_hash).await;
    let db_pool_manager = Arc::clone(&ctx.db_pool_manager);
    let http = Arc::clone(&ctx.http);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(MailEvent::NewMail { folder, latest_sender, latest_subject, .. }) => {
                    let notification = Notification {
                        sender: latest_sender.unwrap_or_else(|| "New message".to_string()),
                        subject: latest_subject.unwrap_or_default(),
                        folder,
                    };
                    deliver(&db_pool_manager, &http, &vapid, &user_hash, &notification).await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "push subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        running.remove(&user_hash);
    });
}

/// Send one notification to every browser this mailbox registered, dropping
/// endpoints the push service says are gone.
async fn deliver(
    db_pool_manager: &Arc<db::pool::DbPoolManager>,
    http: &reqwest::Client,
    vapid: &Vapid,
    user_hash: &str,
    notification: &Notification,
) {
    let subscriptions = db::pool::with_user_db(db_pool_manager, user_hash, |conn| {
        db::push::list_subscriptions(conn)
    })
    .await
    .unwrap_or_default();

    for subscription in subscriptions {
        match sender::send(http, vapid, &subscription, notification).await {
            Ok(()) => {}
            Err(sender::SendError::Gone) => {
                let endpoint = subscription.endpoint.clone();
                let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                    db::push::delete_subscription(conn, &endpoint)
                })
                .await;
            }
            Err(e) => tracing::warn!(error = %e, "push delivery failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn pool(dir: &std::path::Path) -> Arc<crate::db::pool::DbPoolManager> {
        Arc::new(crate::db::pool::DbPoolManager::new(
            dir.to_str().unwrap().to_string(),
            4,
            std::time::Duration::from_secs(600),
            500,
        ))
    }

    async fn user_with_credential(dir: &std::path::Path, invalid: bool) -> String {
        let user_hash = crate::auth::user_data::hash_email("naveen@altacee.com");
        crate::auth::user_data::provision_user_data(dir.to_str().unwrap(), &user_hash).unwrap();
        let pool = pool(dir);
        crate::db::pool::with_user_db(&pool, &user_hash, move |conn| {
            crate::db::push::store_credential(
                conn,
                &crate::db::push::StoreCredential {
                    email: "naveen@altacee.com".to_string(),
                    imap_host: "ryuvzdff.altacee.com".to_string(),
                    imap_port: 993,
                    nonce: vec![0; 12],
                    ciphertext: vec![0; 32],
                },
            )?;
            if invalid {
                crate::db::push::mark_invalid(conn)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        user_hash
    }

    #[tokio::test]
    async fn finds_users_that_have_a_credential() {
        let dir = TempDir::new().unwrap();
        let user_hash = user_with_credential(dir.path(), false).await;
        let found = users_with_push(dir.path().to_str().unwrap(), &pool(dir.path())).await;
        assert_eq!(found, vec![user_hash]);
    }

    #[tokio::test]
    async fn a_credential_marked_invalid_is_not_retried() {
        let dir = TempDir::new().unwrap();
        let _ = user_with_credential(dir.path(), true).await;
        let found = users_with_push(dir.path().to_str().unwrap(), &pool(dir.path())).await;
        assert!(
            found.is_empty(),
            "a password the server rejected must not be tried again until someone re-enters it"
        );
    }

    #[tokio::test]
    async fn a_user_without_a_credential_is_ignored() {
        let dir = TempDir::new().unwrap();
        let user_hash = crate::auth::user_data::hash_email("someone@altacee.com");
        crate::auth::user_data::provision_user_data(dir.path().to_str().unwrap(), &user_hash)
            .unwrap();
        assert!(users_with_push(dir.path().to_str().unwrap(), &pool(dir.path())).await.is_empty());
    }

    #[tokio::test]
    async fn a_rejected_password_is_marked_invalid_and_not_retried() {
        let dir = TempDir::new().unwrap();
        let user_hash = user_with_credential(dir.path(), false).await;
        let pool = pool(dir.path());
        let imap_client: Arc<dyn crate::imap::client::ImapClient> = Arc::new(
            crate::imap::client::mock::MockImapClient::new()
                .with_error(crate::imap::error::ImapError::AuthenticationFailed),
        );
        let creds = ImapCredentials {
            host: "ryuvzdff.altacee.com".to_string(),
            port: 993,
            tls: true,
            email: "naveen@altacee.com".to_string(),
            password: "revoked".to_string(),
        };

        assert!(!credential_still_works(&imap_client, &pool, &user_hash, &creds).await);
        assert!(
            users_with_push(dir.path().to_str().unwrap(), &pool).await.is_empty(),
            "a rejected credential must drop out of the scan, not be retried"
        );
    }

    #[tokio::test]
    async fn a_stray_file_in_the_data_directory_is_not_mistaken_for_a_user() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README"), b"not a user").unwrap();
        std::fs::create_dir(dir.path().join("tantivy-scratch")).unwrap();
        assert!(users_with_push(dir.path().to_str().unwrap(), &pool(dir.path())).await.is_empty());
    }
}
