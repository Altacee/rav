# Item 4: More than one pooled IMAP session per account — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop paying a fresh TCP + TLS + LOGIN (~100 ms, measured from the pod to mailcow) every time two requests for the same account overlap.

**Architecture:** Extract the slot map into a small generic `Pool<T>`, so its capacity behaviour is testable without an IMAP server, and let `SessionCache` hold up to `IMAP_POOL_SIZE` idle sessions per account instead of exactly one. The connect semaphore is untouched — that is the thing that stopped an earlier OOM.

**Tech Stack:** Rust, `std::sync::Mutex` (never held across an `.await`), tokio `Semaphore`.

**Spec:** `docs/superpowers/specs/2026-09-16-mailcow-native-design.md`

## Global Constraints

- `MAX_CONCURRENT_CONNECTS_PER_ACCOUNT = 4` stays exactly as it is: widening the idle pool must not widen how many connections can be opened at once.
- Never hold a `std::sync::MutexGuard` across an `.await`.
- A session that errored is dropped by its caller and never returned to the pool — that contract does not change.
- Default `IMAP_POOL_SIZE` is 3. mailcow is a shared host; this is a number to be kind with, not to maximise.

## File Structure

| File | Responsibility |
|---|---|
| `backend/src/imap/pool.rs` (create) | A generic, synchronous `Pool<T>`: take, put, capacity, per-key isolation. No IMAP, no async, fully unit-testable. |
| `backend/src/imap/session_cache.rs` (modify) | Keeps the IMAP-specific parts — key derivation, the connect semaphore, `connect()` — and delegates storage to `Pool<ImapSession>`. |
| `backend/src/imap/client.rs:266` (modify) | Passes the configured pool size when building `RealImapClient`. |
| `backend/src/config.rs` (modify) | `imap_pool_size`, default 3. |

---

### Task 1: A generic pool with a capacity

**Files:**
- Create: `backend/src/imap/pool.rs`
- Modify: `backend/src/imap/mod.rs` (add `pub(crate) mod pool;`)
- Test: inline `#[cfg(test)]` in `backend/src/imap/pool.rs`

**Interfaces:**
- Produces: `pub struct Pool<T>` with `pub fn new(max_idle: usize) -> Self`, `pub fn take(&self, key: &str) -> Option<T>`, `pub fn put(&self, key: &str, value: T) -> bool` (false = at capacity, value dropped), `pub fn idle_count(&self, key: &str) -> usize`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_returns_none_when_empty() {
        let pool: Pool<u32> = Pool::new(3);
        assert_eq!(pool.take("a@example.com"), None);
    }

    #[test]
    fn a_put_value_comes_back_out_once() {
        let pool = Pool::new(3);
        assert!(pool.put("a@example.com", 7));
        assert_eq!(pool.take("a@example.com"), Some(7));
        assert_eq!(pool.take("a@example.com"), None, "taking removes it");
    }

    #[test]
    fn capacity_is_per_key_and_enforced() {
        let pool = Pool::new(2);
        assert!(pool.put("a", 1));
        assert!(pool.put("a", 2));
        assert!(!pool.put("a", 3), "the third idle session must be refused, not stored");
        assert_eq!(pool.idle_count("a"), 2);
        assert!(pool.put("b", 9), "a different account has its own capacity");
        assert_eq!(pool.idle_count("b"), 1);
    }

    #[test]
    fn zero_capacity_keeps_nothing() {
        let pool = Pool::new(0);
        assert!(!pool.put("a", 1));
        assert_eq!(pool.take("a"), None);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd backend && cargo test imap::pool`
Expected: FAIL — `Pool` does not exist.

- [ ] **Step 3: Write the implementation**

```rust
use std::collections::HashMap;
use std::sync::Mutex;

/// A bounded set of idle values per key.
///
/// Deliberately synchronous and generic: the capacity rule is the part worth
/// testing, and it can be tested without an IMAP server. `SessionCache` adds
/// everything IMAP-shaped on top.
pub struct Pool<T> {
    slots: Mutex<HashMap<String, Vec<T>>>,
    max_idle: usize,
}

impl<T> Pool<T> {
    pub fn new(max_idle: usize) -> Self {
        Pool { slots: Mutex::new(HashMap::new()), max_idle }
    }

    pub fn take(&self, key: &str) -> Option<T> {
        let mut slots = self.slots.lock().unwrap();
        slots.get_mut(key).and_then(Vec::pop)
    }

    /// Returns false when the pool is already at capacity for this key, in
    /// which case `value` is dropped — for an IMAP session that closes the
    /// socket, which is the intended outcome.
    pub fn put(&self, key: &str, value: T) -> bool {
        if self.max_idle == 0 {
            return false;
        }
        let mut slots = self.slots.lock().unwrap();
        let entry = slots.entry(key.to_string()).or_default();
        if entry.len() >= self.max_idle {
            return false;
        }
        entry.push(value);
        true
    }

    pub fn idle_count(&self, key: &str) -> usize {
        self.slots.lock().unwrap().get(key).map_or(0, Vec::len)
    }
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cd backend && cargo test imap::pool`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add backend/src/imap/pool.rs backend/src/imap/mod.rs
git commit -m "feat(imap): a bounded idle pool, testable without a server"
```

---

### Task 2: SessionCache keeps N idle sessions, not one

**Files:**
- Modify: `backend/src/imap/session_cache.rs`
- Modify: `backend/src/imap/client.rs:263-270`
- Modify: `backend/src/config.rs`
- Test: inline `#[cfg(test)]` in `backend/src/imap/session_cache.rs`

**Interfaces:**
- Consumes: `Pool<T>` from Task 1.
- Produces: `SessionCache::new(max_idle: usize)`; `AppConfig.imap_pool_size: usize`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_separates_accounts_and_hosts() {
        let a = ImapCredentials {
            host: "mail.example.com".into(), port: 993, tls: true,
            email: "a@example.com".into(), password: "x".into(),
        };
        let mut b = a.clone();
        b.email = "b@example.com".into();
        let mut c = a.clone();
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd backend && cargo test imap::session_cache`
Expected: FAIL — `SessionCache::new` takes no argument and `max_idle` does not exist.

- [ ] **Step 3: Write the implementation**

In `session_cache.rs`, replace the `slots` field and the two methods:

```rust
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
            pool: Pool::new(max_idle),
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
```

`acquire` changes only its first block:

```rust
        let key = Self::key(creds);
        if let Some(session) = self.pool.take(&key) {
            return Ok(session);
        }
```

and `release` becomes:

```rust
    /// Return a healthy session for reuse. Beyond the pool's capacity the
    /// session is dropped, which closes the socket.
    pub fn release(&self, creds: &ImapCredentials, session: ImapSession) {
        self.pool.put(&Self::key(creds), session);
    }
```

Add `pub fn capacity(&self) -> usize { self.max_idle }` to `Pool`.

In `config.rs`:

```rust
    /// How many authenticated IMAP sessions per account are kept for reuse.
    /// mailcow is a shared host: this is a number to be kind with.
    #[serde(default = "default_imap_pool_size")]
    pub imap_pool_size: usize,
```

with `fn default_imap_pool_size() -> usize { 3 }`, and every `AppConfig`
literal in tests gains `imap_pool_size: 3`.

In `client.rs`, `RealImapClient::new` takes the size and passes it:

```rust
    pub fn new(transport: Arc<MailTransport>, pool_size: usize) -> Self {
        RealImapClient { cache: SessionCache::new(pool_size), transport }
    }
```

Update its call site in `main.rs` to pass `config.imap_pool_size`.

- [ ] **Step 4: Run the suite**

Run: `cd backend && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: everything green, including the pre-existing suite.

- [ ] **Step 5: Commit**

```bash
git add backend/src
git commit -m "feat(imap): keep IMAP_POOL_SIZE idle sessions per account"
```

---

### Task 3: Ship it

- [ ] **Step 1: Build and push the image** — same recipe as item 1, from this branch's HEAD.
- [ ] **Step 2: Bump the digest** in `gitops/apps/rav/rav.yaml`, PR, merge, hard-refresh Argo, confirm the pod serves the new digest.
- [ ] **Step 3: Confirm nothing regressed** — sign in, switch folders, open messages, and check the pod logs for IMAP errors or reconnect storms.
- [ ] **Step 4: Record** the image digest and what step 3 showed in `~/development/altacee/infra/lab-cluster/RUNLOG.md`.

**Note on measurement:** the honest before/after for this item is a signed-in
session, which needs a mailbox password. The unit tests prove the capacity rule;
the latency claim stays unproven until someone with credentials compares folder
switches before and after.

## Self-Review

- **Spec coverage:** item 4 of the spec is Tasks 1–2; Task 3 ships it.
- **Placeholders:** none.
- **Type consistency:** `Pool::new/take/put/capacity/idle_count` and `SessionCache::new(max_idle)` are used identically in both tasks; `RealImapClient::new` gains exactly one parameter, updated at its one call site.
