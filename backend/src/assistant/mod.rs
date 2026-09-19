//! The assistant panel: a bounded tool loop over the signed-in mailbox.
//! Drafts and actions are proposals; nothing here writes to the mailbox.
pub mod actions;
pub mod events;
pub mod mail;
pub mod model;
pub mod refs;
pub mod session;
pub mod text;
pub mod tools;
