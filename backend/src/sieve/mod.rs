mod client;
mod generator;
mod protocol;

pub use generator::is_sieve_capable;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::filters::FilterRule;

/// Publish the whole Sieve state — filter rules and the out-of-office
/// responder — if sieve_host is configured. Dovecot activates one script per
/// user, so both always travel together. Best-effort: logs on failure.
pub async fn push_state(
    config: &Arc<AppConfig>,
    transport: &crate::mail_transport::MailTransport,
    email: &str,
    password: &str,
    rules: &[FilterRule],
    vacation: Option<&crate::db::vacation::VacationResponder>,
) {
    let Some(ref host) = config.sieve_host else { return; };
    let script = generator::generate_script(rules, vacation);
    if let Err(e) = client::push_script(
        client::SieveTarget {
            host,
            port: config.sieve_port,
            tls: &transport.imap_connector,
            allow_plaintext: config.sieve_allow_plaintext,
        },
        email,
        password,
        "rav-filters",
        &script,
    )
    .await
    {
        tracing::warn!(error = %e, "ManageSieve push failed - filters and vacation apply in-app only");
    }
}
