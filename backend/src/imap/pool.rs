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

    pub fn capacity(&self) -> usize {
        self.max_idle
    }

    pub fn take(&self, key: &str) -> Option<T> {
        let mut slots = self.slots.lock().unwrap();
        slots.get_mut(key).and_then(Vec::pop)
    }

    /// Returns false when this key is already at capacity, in which case
    /// `value` is dropped — for an IMAP session that closes the socket, which
    /// is the intended outcome.
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

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn idle_count(&self, key: &str) -> usize {
        self.slots.lock().unwrap().get(key).map_or(0, Vec::len)
    }
}

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
