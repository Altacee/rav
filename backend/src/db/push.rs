use rusqlite::{params, Connection};

/// A stored push credential. `invalid_since` is set when it stops
/// authenticating, so a bad password stops being retried against the mail
/// server until someone enters a new one.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub invalid_since: Option<String>,
}

/// What a caller supplies to store one.
#[derive(Debug, Clone)]
pub struct StoreCredential {
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

pub fn get_credential(conn: &Connection) -> Result<Option<StoredCredential>, String> {
    let mut stmt = conn
        .prepare("SELECT email, imap_host, imap_port, nonce, ciphertext, invalid_since FROM push_credential WHERE id = 1")
        .map_err(|e| format!("prepare get_credential: {e}"))?;
    let mut rows = stmt
        .query_map([], |row| {
            Ok(StoredCredential {
                email: row.get(0)?,
                imap_host: row.get(1)?,
                imap_port: row.get::<_, i64>(2)? as u16,
                nonce: row.get(3)?,
                ciphertext: row.get(4)?,
                invalid_since: row.get(5)?,
            })
        })
        .map_err(|e| format!("query get_credential: {e}"))?;
    match rows.next() {
        Some(row) => Ok(Some(row.map_err(|e| format!("read credential: {e}"))?)),
        None => Ok(None),
    }
}

/// Store (or replace) the one credential. Replacing clears `invalid_since`: a
/// freshly entered password deserves a fresh attempt.
pub fn store_credential(conn: &Connection, data: &StoreCredential) -> Result<(), String> {
    conn.execute(
        "INSERT INTO push_credential (id, email, imap_host, imap_port, nonce, ciphertext, invalid_since)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT(id) DO UPDATE SET
            email = excluded.email,
            imap_host = excluded.imap_host,
            imap_port = excluded.imap_port,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext,
            invalid_since = NULL",
        params![
            data.email,
            data.imap_host,
            data.imap_port as i64,
            data.nonce,
            data.ciphertext
        ],
    )
    .map_err(|e| format!("store_credential: {e}"))?;
    Ok(())
}

/// Record that the stored credential no longer authenticates.
pub fn mark_invalid(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE push_credential SET invalid_since = datetime('now') WHERE id = 1",
        [],
    )
    .map_err(|e| format!("mark_invalid: {e}"))?;
    Ok(())
}

/// Forget the credential and every subscription it fed: with nothing able to
/// deliver, keeping browser endpoints around serves no one.
pub fn delete_credential(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM push_credential", [])
        .map_err(|e| format!("delete_credential: {e}"))?;
    conn.execute("DELETE FROM push_subscription", [])
        .map_err(|e| format!("delete subscriptions: {e}"))?;
    Ok(())
}

pub fn add_subscription(
    conn: &Connection,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO push_subscription (endpoint, p256dh, auth) VALUES (?1, ?2, ?3)
         ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth",
        params![endpoint, p256dh, auth],
    )
    .map_err(|e| format!("add_subscription: {e}"))?;
    Ok(())
}

pub fn delete_subscription(conn: &Connection, endpoint: &str) -> Result<(), String> {
    conn.execute("DELETE FROM push_subscription WHERE endpoint = ?1", params![endpoint])
        .map_err(|e| format!("delete_subscription: {e}"))?;
    Ok(())
}

pub fn list_subscriptions(conn: &Connection) -> Result<Vec<PushSubscription>, String> {
    let mut stmt = conn
        .prepare("SELECT endpoint, p256dh, auth FROM push_subscription ORDER BY created_at")
        .map_err(|e| format!("prepare list_subscriptions: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PushSubscription {
                endpoint: row.get(0)?,
                p256dh: row.get(1)?,
                auth: row.get(2)?,
            })
        })
        .map_err(|e| format!("query list_subscriptions: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read subscriptions: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn params() -> StoreCredential {
        StoreCredential {
            email: "naveen@altacee.com".to_string(),
            imap_host: "ryuvzdff.altacee.com".to_string(),
            imap_port: 993,
            nonce: vec![1; 12],
            ciphertext: vec![2; 32],
        }
    }

    #[test]
    fn no_credential_by_default() {
        let conn = open_test_db();
        assert!(get_credential(&conn).unwrap().is_none());
    }

    #[test]
    fn a_stored_credential_comes_back() {
        let conn = open_test_db();
        store_credential(&conn, &params()).unwrap();
        let got = get_credential(&conn).unwrap().unwrap();
        assert_eq!(got.email, "naveen@altacee.com");
        assert_eq!(got.imap_port, 993);
        assert_eq!(got.ciphertext, vec![2; 32]);
        assert!(got.invalid_since.is_none());
    }

    #[test]
    fn storing_again_replaces_rather_than_accumulates() {
        let conn = open_test_db();
        store_credential(&conn, &params()).unwrap();
        let mut second = params();
        second.ciphertext = vec![3; 32];
        store_credential(&conn, &second).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM push_credential", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "one mailbox, one stored credential");
        assert_eq!(get_credential(&conn).unwrap().unwrap().ciphertext, vec![3; 32]);
    }

    #[test]
    fn re_storing_clears_a_previous_invalid_mark() {
        let conn = open_test_db();
        store_credential(&conn, &params()).unwrap();
        mark_invalid(&conn).unwrap();
        assert!(get_credential(&conn).unwrap().unwrap().invalid_since.is_some());
        store_credential(&conn, &params()).unwrap();
        assert!(
            get_credential(&conn).unwrap().unwrap().invalid_since.is_none(),
            "a freshly entered password deserves a fresh attempt"
        );
    }

    #[test]
    fn deleting_the_credential_takes_the_subscriptions_with_it() {
        let conn = open_test_db();
        store_credential(&conn, &params()).unwrap();
        add_subscription(&conn, "https://push.example/1", "p", "a").unwrap();
        add_subscription(&conn, "https://push.example/2", "p", "a").unwrap();
        delete_credential(&conn).unwrap();
        assert!(get_credential(&conn).unwrap().is_none());
        assert!(
            list_subscriptions(&conn).unwrap().is_empty(),
            "no credential means nothing can be delivered, so nothing should be kept"
        );
    }

    #[test]
    fn the_same_endpoint_subscribing_twice_is_one_subscription() {
        let conn = open_test_db();
        add_subscription(&conn, "https://push.example/1", "p", "a").unwrap();
        add_subscription(&conn, "https://push.example/1", "p2", "a2").unwrap();
        let subs = list_subscriptions(&conn).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].p256dh, "p2", "re-subscribing refreshes the keys");
    }

    #[test]
    fn a_gone_endpoint_can_be_dropped() {
        let conn = open_test_db();
        add_subscription(&conn, "https://push.example/1", "p", "a").unwrap();
        delete_subscription(&conn, "https://push.example/1").unwrap();
        assert!(list_subscriptions(&conn).unwrap().is_empty());
    }
}
