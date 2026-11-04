//! Application state independent of the windowing backend.
pub mod cli;
pub mod git;
pub mod graph;
pub mod history;
pub mod timing;

#[cfg(test)]
mod test_support;
