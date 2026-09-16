use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;
use crate::push::credential;
use crate::push::sender;

/// Handle the push worker watches for credential changes, so enabling push
/// takes effect now rather than at the next rescan.
pub type PushWake = Arc<tokio::sync::Notify>;

#[derive(Deserialize)]
pub struct StoreCredentialBody {
    /// A mailcow app password, not the sign-in password: it is stored (sealed)
    /// so IDLE can outlive the tab, and mailcow can revoke it on its own.
    pub app_password: String,
}

#[derive(Deserialize)]
pub struct SubscriptionBody {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

#[derive(Deserialize)]
pub struct UnsubscribeBody {
    pub endpoint: String,
}

/// `GET /api/push/config` — what the browser needs to decide whether to offer
/// push, and the application server key it must subscribe with.
pub async fn get_config_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let public_key = config
        .push_vapid_key
        .as_deref()
        .and_then(sender::public_key_base64);
    let configured = public_key.is_some()
        && config
            .push_credential_key
            .as_deref()
            .and_then(credential::parse_key)
            .is_some();

    let stored = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::push::get_credential(conn)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(serde_json::json!({
        "enabled": configured,
        "public_key": public_key,
        "credential_stored": stored.is_some(),
        "credential_invalid": stored.map(|c| c.invalid_since.is_some()).unwrap_or(false),
    }))
    .into_response())
}

/// `PUT /api/push/credential` — store the app password that keeps IDLE running.
pub async fn store_credential_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(wake): Extension<PushWake>,
    Json(body): Json<StoreCredentialBody>,
) -> Result<Response, AppError> {
    if body.app_password.is_empty() {
        return Err(AppError::BadRequest("app_password is required".to_string()));
    }
    let key = config
        .push_credential_key
        .as_deref()
        .and_then(credential::parse_key)
        .ok_or_else(|| {
            AppError::ServiceUnavailable("push is not configured on this server".to_string())
        })?;

    let sealed = credential::seal(&key, &body.app_password).map_err(AppError::InternalError)?;
    let email = session.email.clone();
    let imap_host = session.imap_host.clone();
    let imap_port = session.imap_port;
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::push::store_credential(
            conn,
            &db::push::StoreCredential {
                email,
                imap_host,
                imap_port,
                nonce: sealed.nonce,
                ciphertext: sealed.ciphertext,
            },
        )
    })
    .await
    .map_err(AppError::InternalError)?;

    wake.notify_waiters();
    Ok(Json(serde_json::json!({ "stored": true })).into_response())
}

/// `DELETE /api/push/credential` — stop notifications and forget everything
/// stored for them. The app password itself is revoked in mailcow by its owner.
pub async fn delete_credential_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(wake): Extension<PushWake>,
) -> Result<Response, AppError> {
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, db::push::delete_credential)
        .await
        .map_err(AppError::InternalError)?;
    wake.notify_waiters();
    Ok(Json(serde_json::json!({ "stored": false })).into_response())
}

/// `POST /api/push/subscription` — register this browser.
pub async fn subscribe_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<SubscriptionBody>,
) -> Result<Response, AppError> {
    if body.endpoint.is_empty() || body.p256dh.is_empty() || body.auth.is_empty() {
        return Err(AppError::BadRequest(
            "endpoint, p256dh and auth are required".to_string(),
        ));
    }
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::push::add_subscription(conn, &body.endpoint, &body.p256dh, &body.auth)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(serde_json::json!({ "subscribed": true })).into_response())
}

/// `DELETE /api/push/subscription` — this browser only; other devices keep theirs.
pub async fn unsubscribe_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<UnsubscribeBody>,
) -> Result<Response, AppError> {
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::push::delete_subscription(conn, &body.endpoint)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(serde_json::json!({ "subscribed": false })).into_response())
}
