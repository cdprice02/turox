//! How much depth a move or a node is worth.
//!
//! Pure functions of where the loop currently stands, with no state of their
//! own, which is what separates them from `ordering`: ordering learns across a
//! search, selectivity only decides. Reductions, pruning and extensions all
//! land here because they answer one question in two directions: an extension
//! is worth spending as a reduction exemption rather than as extra depth.

pub(in crate::search) mod lmr;
