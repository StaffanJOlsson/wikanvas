//! The authorization seam: authz::require(actor, action, resource) (owner: W2)
//!
//! §8: every handler that touches a single resource calls [`require`] after `Auth<S>`; list-shaped store functions
//! take a `Viewer` instead. In M1 the owner is always allowed, and it is the only user. For the roles that exist in
//! the schema already (D37) the rule is the obvious one, so multi-user later starts from a safe default: `admin`
//! may do everything, `member` may read and write, `viewer` may only read; token management stays with the user
//! that owns the token (the store checks `user_id`). Multi-user later changes only this function and
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

/// Allows or refuses (403 `forbidden`); see the module docs.
pub fn require(actor: &Actor, action: Action, resource: Resource<'_>) -> Result<(), ApiError> {
    let allowed = matches!(
        (actor.role, action),
        (Role::Owner | Role::Admin, _)
            | (Role::Member, Action::Read | Action::Write)
            | (Role::Viewer, Action::Read)
    );
    if allowed {
        Ok(())
    } else {
        Err(ApiError::new(
            ErrorCode::Forbidden,
            format!("not allowed: {action:?} on {resource:?}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::AuthMethod;

    fn actor(role: Role) -> Actor {
        Actor {
            user_id: "u".into(),
            role,
            method: AuthMethod::Session {
                session_id: "s".into(),
            },
        }
    }

    #[test]
    fn roles() {
        let ok = |role, action| require(&actor(role), action, Resource::Item("i")).is_ok();
        for action in [Action::Read, Action::Write, Action::Admin] {
            assert!(ok(Role::Owner, action) && ok(Role::Admin, action));
        }
        assert!(ok(Role::Member, Action::Write) && !ok(Role::Member, Action::Admin));
        assert!(ok(Role::Viewer, Action::Read) && !ok(Role::Viewer, Action::Write));
        let err = require(&actor(Role::Viewer), Action::Admin, Resource::Workspace).unwrap_err();
        assert_eq!(err.code, ErrorCode::Forbidden);
    }
}
