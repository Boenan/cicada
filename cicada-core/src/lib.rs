//! `cicada-core` — everything that is a pure function of its inputs.
//!
//! **No I/O dependencies appear in this crate's manifest** (decision #29), and
//! that is the entire reason the crate exists: purity is enforced by Cargo rather
//! than by discipline. If code here wants `std::fs`, a socket or a subprocess, it
//! belongs in the `cicada` binary instead.

pub mod branch;
pub mod bump;
pub mod changelog;
pub mod check;
pub mod commit;
pub mod config;
pub mod edit;
pub mod identity;
pub mod notes;
pub mod plan;
pub mod render;
pub mod route;
pub mod template;
pub mod updater;
pub mod versions;

pub use check::{Finding, Severity};
pub use commit::{Footer, ParseError, ParsedCommit, RawCommit};
pub use config::{Bump, CommitSource, Config, PackageId, Target};
pub use plan::{DerivedFrom, Plan, PlanError, PlanInput, Reason, Release, render_tag};
pub use edit::{EditError, EditSource, FileEdit};
pub use identity::Identity;
pub use versions::VersionsFile;
