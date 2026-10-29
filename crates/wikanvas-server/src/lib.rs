//! Quire's HTTP server (docs/ARCHITECTURE.md §4.1, §7, §8). This module list is frozen (PLAN W7 "Owns").

pub mod admin;
pub mod app;
pub mod authz;
pub mod board_previews;
pub mod cli;
pub mod config;
pub mod cors;
pub mod csrf;
pub mod error;
pub mod extract;
pub mod headers;
pub mod jobs;
pub mod openapi;
pub mod ratelimit;
pub mod routes;
pub mod state;
pub mod static_files;
#[cfg(feature = "testing")]
pub mod testing;

pub use app::app;
pub use error::ApiError;
pub use extract::{Actor, Auth, AuthMethod, Viewer};
pub use state::AppState;
