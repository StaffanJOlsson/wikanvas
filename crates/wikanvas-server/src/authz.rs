//! The authorization seam: authz::require(actor, action, resource) (owner: W2)
//!
//! §8: every handler with an `Auth<S>` calls [`require`] right after it, before it reads its body or touches the
//! database; list-shaped store functions also take a `Viewer` (`shared::visible_items`). The token scope (`Auth<S>`)
//! and the account's role are separate layers, and a request needs both: a session has every scope, so for a
//! session the role is the only limit. The roles of the schema (D37) get the obvious rule, so multi-user starts
//! from a safe default: the owner and `admin` may do everything, `member` may read and write, `viewer` may only
//! read. The caller's own account ([`Resource::OwnAccount`]: profile, password, sessions, tokens) is the exception:
//! every role may read and manage its own, and the store scopes each of those queries to the caller's `user_id`.
//!
//! The action per route group (§8 has the same table):
//! - [`Action::Read`]: every GET of content and structure (items, documents, boards, attachments, collections, the
//!   tree and the sidebar, the lists, the trash list, search, the workspace, the document schema), and starring,
//!   unstarring and recording a view, which change only the caller's own stars and views;
//! - [`Action::Write`]: creating, editing, moving, duplicating, publishing, archiving, trashing and restoring items
//!   and collections, document and board content, and uploads (`?discard_empty` deletes a new empty item: Write);
//! - [`Action::Admin`]: purging the trash (one item, one collection, or all of it), the workspace settings, export
//!   and backup;
//! - [`Resource::OwnAccount`] with Read or Write: `/auth/me`, `/auth/password`, `/auth/logout`, `/auth/sessions…`
//!   and `/tokens…`.
//!
//! Multi-user later changes only this function and `shared::visible_items`.

use wikanvas_api::{ErrorCode, Role};

use crate::error::ApiError;
use crate::extract::Actor;

/// What the actor wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Read content or structure; also the caller's own stars and views.
    Read,
    /// Change content or structure (the trash included, except purging it).
    Write,
    /// Permanent deletion, settings, other users' tokens, export, backup.
    Admin,
}

/// What it wants to do it to (ids as stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource<'a> {
    /// The workspace as a whole: its settings, the collection list, lists and search across collections, uploads
    /// without an item, export and backup.
    Workspace,
    Collection(&'a str),
    Item(&'a str),
    Attachment(&'a str),
    /// Another user's API token (no route yet): [`Action::Admin`]. The caller's own are [`Resource::OwnAccount`].
    Token(&'a str),
    /// The caller's own profile, password, sessions and tokens: every role may read and change its own.
    OwnAccount,
}

/// Allows or refuses (403 `forbidden`); see the module docs.
pub fn require(actor: &Actor, action: Action, resource: Resource<'_>) -> Result<(), ApiError> {
    let allowed = matches!(
        (actor.role, action, resource),
        (_, Action::Read | Action::Write, Resource::OwnAccount)
            | (Role::Owner | Role::Admin, _, _)
            | (Role::Member, Action::Read | Action::Write, _)
            | (Role::Viewer, Action::Read, _)
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

    #[test]
    fn every_role_manages_its_own_account() {
        for role in [Role::Owner, Role::Admin, Role::Member, Role::Viewer] {
            for action in [Action::Read, Action::Write] {
                assert!(require(&actor(role), action, Resource::OwnAccount).is_ok());
            }
        }
        // Admin on the own account is still the role's: nothing asks for it today.
        assert!(require(&actor(Role::Member), Action::Admin, Resource::OwnAccount).is_err());
        assert!(require(&actor(Role::Admin), Action::Admin, Resource::OwnAccount).is_ok());
        // Another user's token is Admin.
        assert!(require(&actor(Role::Member), Action::Admin, Resource::Token("t")).is_err());
        assert!(require(&actor(Role::Admin), Action::Admin, Resource::Token("t")).is_ok());
    }
}
