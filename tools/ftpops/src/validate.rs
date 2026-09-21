//! Validates that `--filter` is compatible with the chosen `--to`/`--on`
//! direction, per the design spec's combination table: `RemoteOnly`
//! entries only make sense downloading to local or deleting from remote;
//! `LocalOnly` entries only make sense uploading to remote or deleting
//! from local.

use clap::ValueEnum;

use crate::filter::Filter;

/// Which side `copy` writes to.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyTarget {
    Local,
    Remote,
}

/// Which side `delete` removes from.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Local,
    Remote,
}

#[derive(Debug)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_copy(to: CopyTarget, filter: Filter) -> Result<(), ValidationError> {
    match (to, filter) {
        (CopyTarget::Local, Filter::RemoteOnly) => Ok(()),
        (CopyTarget::Remote, Filter::LocalOnly) => Ok(()),
        (CopyTarget::Local, Filter::LocalOnly) => Err(ValidationError(
            "--filter local-only doesn't apply to 'copy --to local': LocalOnly entries have no remote file to download from - did you mean --filter remote-only, or copy --to remote?".to_string(),
        )),
        (CopyTarget::Remote, Filter::RemoteOnly) => Err(ValidationError(
            "--filter remote-only doesn't apply to 'copy --to remote': RemoteOnly entries have no local file to upload from - did you mean --filter local-only, or copy --to local?".to_string(),
        )),
    }
}

pub fn validate_delete(on: DeleteTarget, filter: Filter) -> Result<(), ValidationError> {
    match (on, filter) {
        (DeleteTarget::Remote, Filter::RemoteOnly) => Ok(()),
        (DeleteTarget::Local, Filter::LocalOnly) => Ok(()),
        (DeleteTarget::Remote, Filter::LocalOnly) => Err(ValidationError(
            "--filter local-only doesn't apply to 'delete --on remote': LocalOnly entries don't exist on the remote side - did you mean --filter remote-only, or delete --on local?".to_string(),
        )),
        (DeleteTarget::Local, Filter::RemoteOnly) => Err(ValidationError(
            "--filter remote-only doesn't apply to 'delete --on local': RemoteOnly entries don't exist locally - did you mean --filter local-only, or delete --on remote?".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_to_local_with_remote_only_is_valid() {
        assert!(validate_copy(CopyTarget::Local, Filter::RemoteOnly).is_ok());
    }

    #[test]
    fn copy_to_remote_with_local_only_is_valid() {
        assert!(validate_copy(CopyTarget::Remote, Filter::LocalOnly).is_ok());
    }

    #[test]
    fn copy_to_local_with_local_only_is_invalid() {
        assert!(validate_copy(CopyTarget::Local, Filter::LocalOnly).is_err());
    }

    #[test]
    fn copy_to_remote_with_remote_only_is_invalid() {
        assert!(validate_copy(CopyTarget::Remote, Filter::RemoteOnly).is_err());
    }

    #[test]
    fn delete_on_remote_with_remote_only_is_valid() {
        assert!(validate_delete(DeleteTarget::Remote, Filter::RemoteOnly).is_ok());
    }

    #[test]
    fn delete_on_local_with_local_only_is_valid() {
        assert!(validate_delete(DeleteTarget::Local, Filter::LocalOnly).is_ok());
    }

    #[test]
    fn delete_on_remote_with_local_only_is_invalid() {
        assert!(validate_delete(DeleteTarget::Remote, Filter::LocalOnly).is_err());
    }

    #[test]
    fn delete_on_local_with_remote_only_is_invalid() {
        assert!(validate_delete(DeleteTarget::Local, Filter::RemoteOnly).is_err());
    }
}
