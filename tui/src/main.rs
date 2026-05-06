//! `tui` binary entry point for the Terminal Day Organizer.
//!
//! The real startup sequence (resolve Almacén path, spawn the Agent as a
//! child process, enter the Ratatui render loop, …) is implemented in
//! task 18 of the spec. For now this is a placeholder that keeps the
//! workspace building and exercises a symbol from the `tui` library so
//! the `[lib]` and `[[bin]]` targets stay in sync.

fn main() {
    // Keep a reference to the library so changes that break the public
    // IPC surface fail fast at compile time.
    let _ = tui::ipc::PROTOCOL_VERSION;
}
