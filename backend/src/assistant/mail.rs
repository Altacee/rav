//! `SessionMail`: the real `MailAccess` over the per-user SQLite cache, the
//! Tantivy index and IMAP. Read-only — never writes the cache or the mailbox.

use super::refs::MessageLoc;
use super::text::{readable_body, BODY_CAP, THREAD_CAP};
use super::tools::{MailAccess, MessageContent, MessageSummary, SearchArgs};
use crate::db;
use crate::db::messages::CachedMessage;
use crate::db::pool::DbPoolManager;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::search::engine::{SearchEngine, SearchQuery};
use async_trait::async_trait;
use std::sync::Arc;

const MAX_THREAD: usize = 8;

pub struct SessionMail {
    pub user_hash: String,
    pub creds: ImapCredentials,
    pub db: Arc<DbPoolManager>,
    pub search: Arc<SearchEngine>,
    pub imap: Arc<dyn ImapClient>,
}

/// Parse `YYYY-MM-DD` to the UTC epoch second at the start of that day.
pub fn day_start_epoch(s: &str) -> Option<i64> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
}

fn sender(m: &CachedMessage) -> String {
    if m.from_name.is_empty() {
        m.from_address.clone()
    } else {
        format!("{} <{}>", m.from_name, m.from_address)
    }
}

fn summary(m: CachedMessage) -> MessageSummary {
    MessageSummary {
        from: sender(&m),
        loc: MessageLoc { folder: m.folder, uid: m.uid },
        subject: m.subject,
        date: m.date,
        snippet: m.snippet,
    }
}

impl SessionMail {
    async fn header(&self, loc: &MessageLoc) -> Result<CachedMessage, String> {
        let (folder, uid) = (loc.folder.clone(), loc.uid);
        db::pool::with_user_db(&self.db, &self.user_hash, move |conn| {
            db::messages::get_single_message(conn, &folder, uid)
        })
        .await?
        .ok_or_else(|| "that email is no longer in the mailbox".to_string())
    }

    /// Cached body if present, else IMAP. Does not write the cache.
    async fn body(&self, loc: &MessageLoc, cap: usize) -> Result<String, String> {
        let (folder, uid) = (loc.folder.clone(), loc.uid);
        let cached = db::pool::with_user_db(&self.db, &self.user_hash, move |conn| {
            db::messages::get_cached_body(conn, &folder, uid)
        })
        .await?;
        if let Some(c) = cached {
            return Ok(readable_body(c.html.as_deref(), c.text.as_deref(), cap));
        }
        // BODY.PEEK[] (via fetch_raw_bytes), never fetch_body's BODY[] — reading
        // a message from the assistant must not mark it \Seen on the server.
        let raw = self
            .imap
            .fetch_raw_bytes(&self.creds, &loc.folder, loc.uid)
            .await
            .map_err(|_| "could not fetch that email from the server".to_string())?;
        let (text_plain, text_html) =
            crate::imap::client::parse_peeked_bodies(&raw).map_err(|_| "could not read that email".to_string())?;
        Ok(readable_body(text_html.as_deref(), text_plain.as_deref(), cap))
    }

    async fn content(&self, m: CachedMessage, cap: usize) -> Result<MessageContent, String> {
        let loc = MessageLoc { folder: m.folder.clone(), uid: m.uid };
        let body = self.body(&loc, cap).await?;
        Ok(MessageContent {
            from: sender(&m),
            to: m.to_addresses.clone(),
            loc,
            subject: m.subject,
            date: m.date,
            body,
        })
    }
}

#[async_trait]
impl MailAccess for SessionMail {
    async fn search(&self, args: &SearchArgs) -> Result<Vec<MessageSummary>, String> {
        let a = args.clone();
        let date_from = a.after.as_deref().and_then(day_start_epoch);
        let date_to = a.before.as_deref().and_then(day_start_epoch);
        let q = a.query.clone();
        let mut found: Vec<MessageSummary> = db::pool::with_user_db(&self.db, &self.user_hash, move |conn| {
            db::messages::search_messages_sqlite(
                conn,
                &q,
                db::messages::SearchFilters {
                    folder: a.folder.as_deref(),
                    from: a.from.as_deref(),
                    to: None,
                    date_from,
                    date_to,
                    has_attachment: None,
                    is_read: None,
                    is_flagged: None,
                },
                10,
            )
        })
        .await?
        .into_iter()
        .map(summary)
        .collect();

        // Body matches the SQLite pass cannot see, as the search route does.
        if found.len() < 10
            && !args.query.trim().is_empty()
            && let Ok(index) = self.search.open_user_index(&self.user_hash)
            && let Ok((hits, _)) = index.search(&SearchQuery {
                text: args.query.clone(),
                folder: args.folder.clone(),
                from: args.from.clone(),
                date_from,
                date_to,
                limit: 10,
                ..Default::default()
            })
        {
            for hit in hits {
                if found.len() >= 10 {
                    break;
                }
                if found.iter().any(|m| m.loc.folder == hit.folder && m.loc.uid == hit.uid) {
                    continue;
                }
                if let Ok(m) = self.header(&MessageLoc { folder: hit.folder.clone(), uid: hit.uid }).await {
                    found.push(summary(m));
                }
            }
        }
        Ok(found)
    }

    async fn read(&self, loc: &MessageLoc) -> Result<MessageContent, String> {
        let m = self.header(loc).await?;
        self.content(m, BODY_CAP).await
    }

    async fn thread(&self, loc: &MessageLoc) -> Result<Vec<MessageContent>, String> {
        let m = self.header(loc).await?;
        let Some(mid) = m.message_id.clone().filter(|s| !s.is_empty()) else {
            return Ok(vec![self.content(m, THREAD_CAP).await?]);
        };
        let references = m.references_header.clone();
        let rows = db::pool::with_user_db(&self.db, &self.user_hash, move |conn| {
            db::messages::get_full_thread(conn, &mid, references.as_deref())
        })
        .await?;
        let start = rows.len().saturating_sub(MAX_THREAD); // keep the latest
        let mut out = Vec::new();
        for row in rows.into_iter().skip(start) {
            out.push(self.content(row, THREAD_CAP).await?);
        }
        Ok(out)
    }

    async fn folders(&self) -> Result<Vec<String>, String> {
        let rows = db::pool::with_user_db(&self.db, &self.user_hash, db::folders::get_all_folders).await?;
        Ok(rows.into_iter().map(|f| f.name).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_to_utc_day_start() {
        assert_eq!(day_start_epoch("2026-09-19"), Some(1_789_776_000));
        assert_eq!(day_start_epoch("19/09/2026"), None);
        assert_eq!(day_start_epoch(""), None);
    }

    use crate::db::messages::UpsertMessageParams;
    use crate::imap::client::mock::MockImapClient;
    use crate::search::engine::SearchEngine;
    use std::time::Duration;

    /// A fresh, migrated per-user db plus a `SessionMail` over it and `imap`.
    async fn test_mail(tmp: &std::path::Path, imap: MockImapClient) -> SessionMail {
        let data_dir = tmp.to_str().unwrap().to_string();
        crate::auth::user_data::provision_user_data(&data_dir, "user1").unwrap();
        let db = Arc::new(DbPoolManager::new(data_dir, 4, Duration::from_secs(600), 10));
        db::pool::with_user_db(&db, "user1", |conn| {
            db::folders::upsert_folder(
                conn,
                db::folders::UpsertFolderParams {
                    name: "INBOX",
                    delimiter: None,
                    parent: None,
                    flags_csv: "",
                    is_subscribed: true,
                    total_count: 1,
                    unread_count: 0,
                    uid_validity: 1,
                    highest_modseq: 0,
                },
            )
        })
        .await
        .unwrap();

        SessionMail {
            user_hash: "user1".to_string(),
            creds: ImapCredentials { host: "x".into(), port: 993, tls: true, email: "a@x.com".into(), password: "".into() },
            db,
            search: Arc::new(SearchEngine::new(tmp.join("search"))),
            imap: Arc::new(imap),
        }
    }

    fn msg<'a>(
        message_id: Option<&'a str>,
        in_reply_to: Option<&'a str>,
        references_header: Option<&'a str>,
        date_epoch: i64,
    ) -> UpsertMessageParams<'a> {
        UpsertMessageParams {
            message_id,
            in_reply_to,
            references_header,
            subject: "Hi",
            from_address: "a@x.com",
            from_name: "A",
            to_json: "me@x.com",
            cc_json: "",
            date: "2026-09-17",
            date_epoch,
            flags_csv: "",
            size: 10,
            has_attachments: false,
            snippet: "hi there",
            reaction: None,
        }
    }

    #[tokio::test]
    async fn reading_an_uncached_body_peeks_and_never_calls_fetch_body() {
        let tmp = tempfile::tempdir().unwrap();
        let mail = test_mail(
            tmp.path(),
            // fetch_body errors, so a success here proves the read used
            // fetch_raw_bytes (BODY.PEEK[]) instead — never marking \Seen.
            MockImapClient::new()
                .with_fetch_body_disabled()
                .with_raw_bytes(b"From: a@x.com\r\nSubject: Hi\r\n\r\nHello there".to_vec()),
        )
        .await;
        db::pool::with_user_db(&mail.db, "user1", |conn| {
            db::messages::upsert_message(conn, "INBOX", 1, msg(Some("<abc@x>"), None, None, 1_789_000_000))
        })
        .await
        .unwrap();

        let content = mail.read(&MessageLoc { folder: "INBOX".to_string(), uid: 1 }).await.unwrap();
        assert!(content.body.contains("Hello there"));
    }

    #[tokio::test]
    async fn thread_walks_ancestors_not_just_descendants() {
        let tmp = tempfile::tempdir().unwrap();
        let mail = test_mail(tmp.path(), MockImapClient::new()).await;
        db::pool::with_user_db(&mail.db, "user1", |conn| {
            // m1 is the ancestor; m2 replies to it and references it.
            db::messages::upsert_message(conn, "INBOX", 1, msg(Some("<m1@x>"), None, None, 1_789_000_000))?;
            db::messages::upsert_message(conn, "INBOX", 2, msg(Some("<m2@x>"), Some("<m1@x>"), Some("<m1@x>"), 1_789_000_100))
        })
        .await
        .unwrap();

        // Reading the thread from the *reply* (m2) must still surface its
        // ancestor (m1) — get_thread_messages(m2's id) alone would not, since
        // it only finds messages that reference m2, not what m2 references.
        let thread = mail.thread(&MessageLoc { folder: "INBOX".to_string(), uid: 2 }).await.unwrap();
        let uids: Vec<u32> = thread.iter().map(|c| c.loc.uid).collect();
        assert_eq!(uids, vec![1, 2]);
    }
}
