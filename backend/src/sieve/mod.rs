mod client;
mod generator;
mod protocol;

pub use generator::is_sieve_capable;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::filters::FilterRule;

/// Push all rules to ManageSieve if sieve_host is configured. Best-effort: logs on failure.
pub async fn push_filters(
    config: &Arc<AppConfig>,
    transport: &crate::mail_transport::MailTransport,
    email: &str,
    password: &str,
    rules: &[FilterRule],
) {
    let Some(ref host) = config.sieve_host else { return; };
    let script = generator::generate_sieve_script(rules);
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
        tracing::warn!(error = %e, "ManageSieve push failed - filters will apply via IDLE only");
    }
}
