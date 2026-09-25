//! Deciding which move to try first, and measuring whether that worked.
//!
//! Alpha-beta prunes in proportion to how early the best move is tried, so
//! ordering is what makes the search cheap rather than what makes it correct.
//! [`stats`] is how that claim is checked.

pub mod stats;
