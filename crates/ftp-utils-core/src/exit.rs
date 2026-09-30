//! Process exit codes shared by every tool in the suite.

/// The run succeeded (for ftpdiff: everything matched).
pub const EXIT_OK: i32 = 0;

/// The run completed but found differences (ftpdiff) or some operations
/// failed (ftpops).
pub const EXIT_FAILURES: i32 = 1;

/// The run could not complete: bad arguments or config, unreadable input,
/// connection failure, or a user abort.
pub const EXIT_ERROR: i32 = 2;
