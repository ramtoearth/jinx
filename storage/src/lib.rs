//! `storage` crate for the Terminal Day Organizer.
//!
//! This crate owns the SQLite-backed Almacén: schema, migrations, the
//! `Storage` trait and its `SqliteStorage` implementation, plus the
//! export/import routines used by the Exportador.
//!
//! The actual implementation is added in later tasks of the spec; this
//! file currently acts as the crate root so the workspace builds.
