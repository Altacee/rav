//! Notifications that survive the browser tab being closed.
//!
//! Realtime in this client has always meant "while a WebSocket is open": the
//! IDLE manager starts when one connects and the worker reaps when the last one
//! goes. Push needs an IMAP connection that outlives every tab, which means
//! holding a credential — see `credential` for what is held and how.

pub mod credential;
