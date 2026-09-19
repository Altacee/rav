use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use super::connection::{connect, ImapStream};
use super::error::ImapError;
use super::pool::Pool;
use super::types::ImapCredentials;

/// An authenticated IMAP session over our stream wrapper.
pub type ImapSession = async_imap::Session<ImapStream>;

/// Max number of brand-new IMAP connections (TCP+TLS+LOGIN) allowed to be
/// opened concurrently for a single account. Only a few idle slots exist per
/// account beyond the pooled ones, so a request that finds the pool empty
/// falls through to `connect()`;
/// without a cap, a burst of concurrent requests (e.g. an unbatched bulk
/// action over hundreds of messages) opens one connection per request all at
/// once, which is what actually OOM-killed the process.
const MAX_CONCURRENT_CONNECTS_PER_ACCOUNT: usize = 4;

/// A pooled session idle this long is discarded rather than reused. Dovecot
/// logs out a non-IDLE connection after 30 minutes of no input (a compile-time
/// constant, not configurable in mailcow); reusing one after that hands a
/// closed socket to the next command. On 2026-09-19 that made LIST read EOF as
/// "zero folders" and the webmail showed an empty mailbox. Stay under 30.
const SESSION_MAX_IDLE: std::time::Duration = std::time::Duration::from_secs(25 * 60);

/// Up to `max_idle` reusable sessions per account (email@host).
///
/// Acquiring takes a session out of the pool; releasing puts it back. On error
/// paths, callers simply drop the session rather than calling `release`,
/// ensuring a broken connection is never reused.
pub struct SessionCache {
    pool: Pool<ImapSession>,
    connect_limits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl SessionCache {
    /// `max_idle` is how many authenticated sessions per account are kept for
    /// reuse. It is not a limit on live connections — that is still
    /// `MAX_CONCURRENT_CONNECTS_PER_ACCOUNT`, which must not move.
    pub fn new(max_idle: usize) -> Self {
        SessionCache {
            pool: Pool::with_max_age(max_idle, SESSION_MAX_IDLE),
            connect_limits: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn max_idle(&self) -> usize {
        self.pool.capacity()
    }

    fn key(creds: &ImapCredentials) -> String {
        format!("{}@{}", creds.email, creds.host)
    }

    fn connect_limit(&self, key: &str) -> Arc<Semaphore> {
        let mut limits = self.connect_limits.lock().unwrap();
        limits
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTS_PER_ACCOUNT)))
            .clone()
    }

    /// Take an existing session from the cache, or create a new one.
    ///
    /// The lock is released before the async connect so we never hold a
    /// `std::sync::MutexGuard` across an `.await`.
    pub async fn acquire(
        &self,
        creds: &ImapCredentials,
        connect_host: &str,
        tls_connector: &async_native_tls::TlsConnector,
    ) -> Result<ImapSession, ImapError> {
        let key = Self::key(creds);
        if let Some(session) = self.pool.take(&key) {
            return Ok(session);
        }
        // Bound how many callers can open a fresh connection for this
        // account at once; excess callers queue here instead of all
        // connecting simultaneously.
        let limiter = self.connect_limit(&key);
        let _permit = limiter
            .acquire_owned()
            .await
            .expect("connect semaphore is never closed");
        connect(creds, connect_host, tls_connector).await
    }

    /// Return a healthy session for reuse after a successful operation. Beyond
    /// the pool's capacity the session is dropped, which closes the socket.
    pub fn release(&self, creds: &ImapCredentials, session: ImapSession) {
        self.pool.put(&Self::key(creds), session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> ImapCredentials {
        ImapCredentials {
            host: "mail.example.com".into(),
            port: 993,
            tls: true,
            email: "a@example.com".into(),
            password: "x".into(),
        }
    }

    #[test]
    fn the_key_separates_accounts_and_hosts() {
        let a = creds();
        let mut b = creds();
        b.email = "b@example.com".into();
        let mut c = creds();
        c.host = "other.example.com".into();
        assert_ne!(SessionCache::key(&a), SessionCache::key(&b));
        assert_ne!(SessionCache::key(&a), SessionCache::key(&c));
    }

    #[test]
    fn the_configured_size_is_what_the_pool_holds() {
        let cache = SessionCache::new(3);
        assert_eq!(cache.max_idle(), 3);
    }
}
