//! Library root of the `tui` crate for the Terminal Day Organizer.
//!
//! This crate exposes both a library (used by tests and by future modules)
//! and the `tui` binary. For now, the library surface is limited to the
//! shared IPC envelope and payload types that travel between the TUI and
//! the Python Agent over stdio (see `Canal_IPC` in `design.md`).
//!
//! Additional modules (app state, storage tool handlers, renderers, …) are
//! added by later spec tasks.

pub mod ipc;
