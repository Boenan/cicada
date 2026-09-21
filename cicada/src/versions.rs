//! Reading `.cicada-versions.toml` from disk.
//!
//! The shape lives in `cicada-core` (#29); this is the three lines that open the
//! file. The only judgement here is what an **absent** file means, and #4 answers
//! it: nothing has been released. Absent and empty are the same state, so a first
//! run in a virgin repo is the ordinary path rather than a special case.

use std::path::{Path, PathBuf};

use cicada_core::versions::VersionsFile;

pub const FILE_NAME: &str = ".cicada-versions.toml";

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not read {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Exit `1`, never a finding: cicada cannot plan without knowing where each
    /// line stands (#79).
    #[error("{}: {message}", .path.display())]
    Parse { path: PathBuf, message: String },
}

pub struct Loaded {
    pub versions: VersionsFile,
    pub path: PathBuf,
    /// False when the file does not exist yet — nothing has ever been released
    /// here (#4). Kept so callers can say so rather than showing an empty table.
    pub exists: bool,
}

/// Its path is fixed *relative to the config* — beside `cicada.toml`, which is
/// the repo root unless `--config` moved it. Tooling that reads this file should
/// not have to hunt for it (#1).
pub fn path_beside(config_path: &Path) -> PathBuf {
    config_path.parent().unwrap_or(Path::new(".")).join(FILE_NAME)
}

pub fn load(config_path: &Path) -> Result<Loaded, LoadError> {
    let path = path_beside(config_path);
    if !path.exists() {
        return Ok(Loaded { versions: VersionsFile::default(), path, exists: false });
    }
    let src = std::fs::read_to_string(&path)
        .map_err(|source| LoadError::Io { path: path.clone(), source })?;
    let versions = VersionsFile::from_toml(&src)
        .map_err(|e| LoadError::Parse { path: path.clone(), message: e.to_string() })?;
    Ok(Loaded { versions, path, exists: true })
}
