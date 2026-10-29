//! The /api/v1 route modules: each returns an OpenApiRouter<AppState> (ARCHITECTURE §7.4). This module list is
//! frozen (PLAN W7 "Owns").

pub mod admin;
pub mod auth;
pub mod boards;
pub mod collections;
pub mod config;
pub mod docs;
pub mod export;
pub mod files;
pub mod invites;
pub mod items;
pub mod lists;
pub mod search;
pub mod setup;
pub mod tokens;
pub mod tree;
pub mod users;
pub mod workspace;
