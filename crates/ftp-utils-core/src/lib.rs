//! Shared FTP/FTPS client, directory comparison, and diff logic used by
//! all tools in the ftp-utils suite.
//!
//! See `docs/superpowers/specs/2026-09-21-ftp-utils-monorepo-design.md`
//! for the design this crate implements.

pub mod diff;
pub mod exclude;
pub mod local;
pub mod remote;
