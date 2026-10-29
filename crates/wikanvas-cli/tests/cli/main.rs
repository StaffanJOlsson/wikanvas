//! The CLI test binary (PLAN R7, W8): one module per area, run against `wikanvas_server::testing::TestServer` or a
//! scripted mock listener (owner: W8). `flows::` needs the W1, W2, W4 and W6 routes and runs in W7's gate.

mod admin;
mod api;
mod auth;
mod common;
mod config;
mod coverage;
mod exit_codes;
mod flows;
mod mocked;
mod output;
mod tokens;
mod transport;
mod users;
mod version;
