//! Application state independent of the windowing backend.

/// Base package version with the short Git revision, or `unknown` for source archives.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("GITK_RS_REVISION"));
pub mod cli;
pub mod diff;
pub mod git;
pub mod graph;
pub mod history;
pub mod search;
pub mod syntax;
pub mod timing;

#[cfg(test)]
mod test_support;

pub mod preferences;
pub mod workspace;

mod settings_file;
