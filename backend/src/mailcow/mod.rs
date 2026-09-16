//! A thin client for mailcow's admin API.
//!
//! Only what Altacee Mail actually needs: the aliases that deliver to a
//! mailbox, so send-as identities do not have to be typed in by hand. Every
//! call degrades to "no information" rather than an error — mailcow being
//! unreachable must never stop someone reading their mail.

mod client;
mod identities;

pub use client::{aliases_for, MailcowApi};
pub use identities::seed_identities;
