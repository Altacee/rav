//! The assistant panel: a bounded tool loop over the signed-in mailbox.
//! Drafts and actions are proposals; nothing here writes to the mailbox.
// removed in Task 10 once the routes use everything
#![allow(dead_code)]
pub mod actions;
pub mod events;
pub mod model;
pub mod refs;
pub mod session;
pub mod text;
pub mod tools;
