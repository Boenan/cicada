//! `FileEdit` — the boundary between computing a release and performing one (#30).
//!
//! Core returns **path plus final content**; the I/O layer's whole job is writing
//! it. That is what makes `--dry-run` trustworthy: it is not a parallel branch of
//! logic describing what *would* happen, it is the identical computation with the
//! write step skipped. A dry-run implemented any other way drifts from reality,
//! and the drift is invisible until the day it matters.
//!
//! Paths are **relative to the repo root**, always (#44). Packages already declare
//! their own `path`, and resolving from the root removes the ambiguity about which
//! one a given updater meant.

/// One file, and everything it should contain afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    /// Relative to the repo root.
    pub path: String,
    pub content: String,
    /// What produced it. Carried for the dry-run diff header and for error
    /// messages — a splice that fails its re-parse (#52) must say which updater
    /// wrote the file, not only which file.
    pub source: EditSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditSource {
    /// A package's updater, named by package and updater kind.
    Updater { package: String, kind: &'static str },
    Changelog { package: String },
    VersionsFile,
}

impl std::fmt::Display for EditSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditSource::Updater { package, kind } => write!(f, "{package} ({kind} updater)"),
            EditSource::Changelog { package } => write!(f, "{package} (changelog)"),
            EditSource::VersionsFile => f.write_str("versions file"),
        }
    }
}

/// Why an edit could not be produced. Every variant aborts the run **before
/// anything is written** (#52) — a partially updated tree is worse than none.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("{path}: {message}")]
    Parse { path: String, message: String },

    #[error("{path}: no value at `{address}`")]
    NotFound { path: String, address: String },

    #[error("{path}: `{address}` {message}")]
    Unaddressable { path: String, address: String, message: &'static str },

    /// #52. The splice produced something that will not parse, which means the
    /// transform is wrong — and finding out now costs a failed run, where finding
    /// out later costs a broken release.
    #[error(
        "{path}: the edit produced a file that no longer parses ({message})\n\
         this is a bug in cicada's {kind} updater — nothing has been written"
    )]
    Reparse { path: String, kind: &'static str, message: String },

    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
}
