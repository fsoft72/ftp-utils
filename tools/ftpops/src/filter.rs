//! The `--filter` CLI value and its mapping to ftpdiff CSV status strings.

use clap::ValueEnum;

/// Which diff status to operate on. clap's default kebab-case casing
/// gives `--filter remote-only` / `--filter local-only`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    RemoteOnly,
    LocalOnly,
}

impl Filter {
    /// The exact string this filter matches in the CSV `status` column
    /// (matches `ftp_utils_core::DiffStatus`'s `Debug` output, which is
    /// what `ftpdiff --csv` writes).
    pub fn status_str(self) -> &'static str {
        match self {
            Filter::RemoteOnly => "RemoteOnly",
            Filter::LocalOnly => "LocalOnly",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_only_maps_to_csv_status() {
        assert_eq!(Filter::RemoteOnly.status_str(), "RemoteOnly");
    }

    #[test]
    fn local_only_maps_to_csv_status() {
        assert_eq!(Filter::LocalOnly.status_str(), "LocalOnly");
    }
}
