//! Library root of the `tui` crate for the Terminal Day Organizer.
//!
//! Modules added as tasks progress:
//! - `ipc`: Canal_IPC envelope and payload types (task 2.1)
//! - `app`: pure AppState and focus state machine (tasks 5.1–5.6)
//! - `color`: terminal colour detection and style resolution (tasks 6.1–6.3)

pub mod app;
pub mod calendario;
pub mod color;
pub mod ipc;
pub mod ipc_handler;
pub mod proximos;
