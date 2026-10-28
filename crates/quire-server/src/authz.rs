//! The authorization seam: authz::require(actor, action, resource) (owner: W2)
//!
//! §8: every handler that touches a single resource calls [`require`] after `Auth<S>`; list-shaped store functions
//! take a `Viewer` instead. In M1 the owner is always allowed; multi-user later changes only this function and
//! `shared::visible_items`.

use quire_api::{ErrorCode, Role};

use crate::error::ApiError;
use crate::extract::Actor;

/// What the actor wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
    /// Permanent deletion, settings, tokens, export, backup.
    Admin,
}

/// What it wants to do it to (ids as stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource<'a> {
    Workspace,
    Collection(&'a str),
    Item(&'a str),
    Attachment(&'a str),
    Token(&'a str),
}

/// Allows or refuses (403 `forbidden`). M1: the owner may do everything; no other role exists yet.
pub fn require(actor: &Actor, action: Action, resource: Resource<'_>) -> Result<(), ApiError> {
    match actor.role {
        Role::Owner => Ok(()),
        _ => Err(ApiError::new(
            ErrorCode::Forbidden,
            format!("not allowed: {action:?} on {resource:?}"),
        )),
    }
}
