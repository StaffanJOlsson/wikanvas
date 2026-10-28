//! Quire's pure domain logic (ARCHITECTURE §4.1): ids, rank, tree rules, time, cursors (D46), the Y text walk,
//! schema v1 as data, document conversion and the board codec. No I/O. This module list is frozen (PLAN W7 "Owns").

/// The API contract (D40): every `quire_core::api::*` path of the design documents is `quire_api::*`.
pub use quire_api as api;

pub mod board;
pub mod convert;
pub mod cursor;
pub mod ids;
pub mod rank;
pub mod schema;
pub mod time;
pub mod tree;
pub mod ytext;
