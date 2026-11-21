//! Fork-owned code. Upstream files should only reference this module through
//! small hooks; see `docs/FORK.md` (the register) and `docs/WATCH-PAGE.md` (the
//! Watch page's design). `seams` is the machine-checked list of those hooks.

#[cfg(test)]
pub mod seams;
pub mod watch;
