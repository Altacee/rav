use std::sync::Arc;

use crate::db;

/// Create a send-as identity for every alias that does not have one yet.
///
/// Idempotent: existing identities are left exactly as they are, including any
/// signature or display name someone edited. A seeded identity is never made
/// the default — an alias appearing in mailcow must not silently change the
/// address someone sends from.
///
/// Returns how many were added.
pub async fn seed_identities(
    db_pool_manager: &Arc<db::pool::DbPoolManager>,
    user_hash: &str,
    aliases: &[String],
) -> Result<usize, String> {
    if aliases.is_empty() {
        return Ok(0);
    }
    let aliases = aliases.to_vec();
    db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
        let existing = db::identities::list_identities(conn)?;
        let known: Vec<String> = existing.iter().map(|i| i.email.to_lowercase()).collect();
        let mut added = 0usize;
        for alias in &aliases {
            if known.contains(&alias.to_lowercase()) {
                continue;
            }
            let display_name = alias.split('@').next().unwrap_or(alias).to_string();
            let created = db::identities::create_identity(
                conn,
                &db::identities::CreateIdentity {
                    display_name,
                    email: alias.clone(),
                    signature_html: String::new(),
                    is_default: false,
                },
            );
            match created {
                Ok(_) => added += 1,
                Err(e) => tracing::warn!(error = %e, alias = %alias, "could not seed identity"),
            }
        }
        Ok(added)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn pool(dir: &std::path::Path) -> std::sync::Arc<crate::db::pool::DbPoolManager> {
        std::sync::Arc::new(crate::db::pool::DbPoolManager::new(
            dir.to_str().unwrap().to_string(),
            4,
            std::time::Duration::from_secs(600),
            500,
        ))
    }

    #[tokio::test]
    async fn aliases_become_identities_and_a_second_run_adds_nothing() {
        let dir = TempDir::new().unwrap();
        let user_hash = crate::auth::user_data::hash_email("naveen@altacee.com");
        crate::auth::user_data::provision_user_data(dir.path().to_str().unwrap(), &user_hash)
            .unwrap();
        let pool = pool(dir.path()).await;

        let aliases = vec!["sales@altacee.com".to_string(), "billing@altacee.com".to_string()];
        let added = seed_identities(&pool, &user_hash, &aliases).await.unwrap();
        assert_eq!(added, 2);

        let again = seed_identities(&pool, &user_hash, &aliases).await.unwrap();
        assert_eq!(again, 0, "seeding twice must not duplicate an identity");

        let identities = crate::db::pool::with_user_db(&pool, &user_hash, |conn| {
            crate::db::identities::list_identities(conn)
        })
        .await
        .unwrap();
        let emails: Vec<String> = identities.iter().map(|i| i.email.clone()).collect();
        assert!(emails.contains(&"sales@altacee.com".to_string()), "got: {emails:?}");
        assert!(emails.contains(&"billing@altacee.com".to_string()), "got: {emails:?}");
    }

    #[tokio::test]
    async fn a_seeded_identity_is_never_the_default() {
        let dir = TempDir::new().unwrap();
        let user_hash = crate::auth::user_data::hash_email("naveen@altacee.com");
        crate::auth::user_data::provision_user_data(dir.path().to_str().unwrap(), &user_hash)
            .unwrap();
        let pool = pool(dir.path()).await;

        seed_identities(&pool, &user_hash, &["sales@altacee.com".to_string()])
            .await
            .unwrap();

        let identities = crate::db::pool::with_user_db(&pool, &user_hash, |conn| {
            crate::db::identities::list_identities(conn)
        })
        .await
        .unwrap();
        let seeded = identities.iter().find(|i| i.email == "sales@altacee.com").unwrap();
        assert!(
            !seeded.is_default,
            "an alias must not quietly become the address someone sends from"
        );
    }
}
