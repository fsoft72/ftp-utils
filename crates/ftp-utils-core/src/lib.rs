//! Shared FTP/FTPS client, directory comparison, and diff logic used by
//! all tools in the ftp-utils suite. The tools compose the building blocks
//! exported here (`local::walk_local_dir`, `remote::walk_remote`,
//! `compare::compare_entries`, `hash::apply_hash_comparison`) themselves,
//! since each side can come from a live source or a CSV report.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this crate implements.

pub mod compare;
pub mod connection;
pub mod csv_source;
pub mod diff;
pub mod error;
pub mod exclude;
pub mod exit;
pub mod ftp_client;
pub mod hash;
pub mod local;
pub mod paths;
pub mod remote;

#[cfg(any(test, feature = "test-utils"))]
pub mod testing;

pub use diff::{DiffEntry, DiffStatus};
pub use remote::{FtpConnection, FtpConnectionError, RawRemoteEntry};
