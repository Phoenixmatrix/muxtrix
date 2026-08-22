//! The GitHub panel: pull requests, changed files, and diffs.
//!
//! Split by surface — the panel frame, the two virtualised lists, and the diff
//! reader — because they change for unrelated reasons.

pub(crate) mod diff;
pub(crate) mod lists;
pub(crate) mod panel;
