//! `cicada-core` — everything that is a pure function of its inputs.
//!
//! **No I/O dependencies appear in this crate's manifest** (decision #29), and
//! that is the entire reason the crate exists: purity is enforced by Cargo rather
//! than by discipline. If code here wants `std::fs`, a socket or a subprocess, it
//! belongs in the `cicada` binary instead.

pub mod branch;
pub mod check;
pub mod commit;
pub mod config;

pub use check::{Finding, Severity};
pub use commit::{Footer, ParseError, ParsedCommit, RawCommit};
pub use config::{Bump, CommitSource, Config, PackageId, Target};
