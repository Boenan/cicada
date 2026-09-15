//! cicada — the binary crate's library half.
//!
//! Everything here touches the world: git, the forge, the filesystem, the CLI.
//! The pure half — commit parsing, routing, bump computation, rendering — lives
//! in `cicada-core`, whose manifest lists no I/O dependency at all (#29).
//!
//! It is a library as well as a binary so integration tests can drive the real
//! `GitBackend` against real repositories rather than a mock.

pub mod config;
pub mod git;
pub mod report;
