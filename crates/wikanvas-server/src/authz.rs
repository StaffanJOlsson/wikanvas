//! The authorization seam: authz::require(state, actor, action, resource) (owner: W2; collection access: D152)
//!
//! §8: every handler with an `Auth<S>` calls [`require`] right after it, before it reads its body or changes
//! anything; list-shaped store functions also take a `Viewer` (`shared::visible_items`). The token scope (`Auth<S>`)
//! and the account's role are separate layers, and a request needs both: a session has every scope, so for a
//! session the role and the collection access are the only limits.
//!
//! The rule (§8 has the same table):
//! - [`Resource::OwnAccount`] (profile, password, sessions, tokens): every role, Read or Write; the store scopes
//!   each of those queries to the caller's `user_id`.
//! - [`Action::Admin`] (purging the trash, the workspace settings, export and backup, accounts and invites): the
//!   owner and `admin` only, on any resource. Which accounts an admin may change is the store's (D149).
//! - [`Resource::Workspace`]: Read for every role (the lists, search and the directory, which filter what they
//!   show), Write for the owner, `admin` and `member` (creating a collection, an upload without an item).
//! - [`Resource::Collection`], [`Resource::Item`] and [`Resource::Attachment`] (D152): the caller's access to the
//!   collection, an item's collection or, for an attachment, whether it may read it (`wikanvas_store::access`).
//!   Read needs `read`, Write `write`, [`Action::Manage`] (the collection's settings, default access and members)
//!   `manage`. The owner and admins have `manage` everywhere, so their calls are not looked up. For anyone else, a
//!   collection, item or attachment they may not read is **404 `not_found`**, worded as for one that does not exist
//!   (and one that does not exist is 404 too, before the handler runs), so the answer never tells that it exists;
//!   one they may read but not change as asked is 403 `forbidden`.
//! - [`Resource::Token`], [`Resource::User`], [`Resource::Invite`]: Admin.

use wikanvas_api::{CollectionAccess, ErrorCode, Role};

use crate::error::ApiError;
use crate::extract::Actor;
use crate::state::AppState;

/// What the actor wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Read content or structure; also the caller's own stars and views.
    Read,
    /// Change content or structure (the trash included, except purging it).
    Write,
    /// Change a collection itself: its settings, default access and members, its place, trashing and restoring it
    /// (`manage` on it, D152).
    Manage,
    /// Permanent deletion, settings, accounts and invites, other users' tokens, export, backup.
    Admin,
}

/// What it wants to do it to (ids as stored, or a `url_id` where the route takes one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource<'a> {
    /// The workspace as a whole: its settings, the collection list, lists and search across collections, the
    /// directory, uploads without an item, export and backup.
    Workspace,
    /// A collection, by id or `url_id`.
    Collection(&'a str),
    /// An item, by id or `url_id`.
    Item(&'a str),
    Attachment(&'a str),
    /// Another user's API token (no route yet): [`Action::Admin`]. The caller's own are [`Resource::OwnAccount`].
    Token(&'a str),
    /// Another user's account (`PATCH /users/{id}`): [`Action::Admin`].
    User(&'a str),
    /// An invite link (`DELETE /invites/{id}`): [`Action::Admin`].
    Invite(&'a str),
    /// The caller's own profile, password, sessions and tokens: every role may read and change its own.
    OwnAccount,
}

fn forbidden(action: Action, resource: Resource<'_>) -> ApiError {
    ApiError::new(
        ErrorCode::Forbidden,
        format!("not allowed: {action:?} on {resource:?}"),
    )
}

/// What the role alone allows, for the resources that have no collection: see the module docs.
fn role_allows(role: Role, action: Action, resource: Resource<'_>) -> bool {
    matches!(
        (role, action, resource),
        (_, Action::Read | Action::Write, Resource::OwnAccount)
            | (Role::Owner | Role::Admin, _, _)
            | (
                Role::Member,
                Action::Read | Action::Write,
                Resource::Workspace
            )
            | (Role::Viewer, Action::Read, Resource::Workspace)
    )
}

/// The access an action needs on a collection.
fn needed(action: Action) -> CollectionAccess {
    match action {
        Action::Read => CollectionAccess::Read,
        Action::Write => CollectionAccess::Write,
        Action::Manage | Action::Admin => CollectionAccess::Manage,
    }
}

/// Allows or refuses (403 `forbidden`, or 404 `not_found` for what the caller may not read); see the module docs.
pub async fn require(
    state: &AppState,
    actor: &Actor,
    action: Action,
    resource: Resource<'_>,
) -> Result<(), ApiError> {
    let privileged = matches!(actor.role, Role::Owner | Role::Admin);
    let lookup = match resource {
        Resource::Collection(id) => Lookup::Collection(id.to_owned()),
        Resource::Item(id) => Lookup::Item(id.to_owned()),
        Resource::Attachment(id) => Lookup::Attachment(id.to_owned()),
        Resource::Workspace
        | Resource::Token(_)
        | Resource::User(_)
        | Resource::Invite(_)
        | Resource::OwnAccount => {
            return if role_allows(actor.role, action, resource) {
                Ok(())
            } else {
                Err(forbidden(action, resource))
            };
        }
    };
    if privileged {
        return Ok(());
    }
    if action == Action::Admin {
        return Err(forbidden(action, resource));
    }
    let viewer = actor.viewer();
    let missing = lookup.missing();
    let access = state
        .db
        .read(move |conn| lookup.access(conn, &viewer))
        .await?;
    if !access.allows(CollectionAccess::Read) {
        return Err(ApiError::not_found(missing));
    }
    if !access.allows(needed(action)) {
        return Err(forbidden(action, resource));
    }
    Ok(())
}

/// What [`require`] looks up.
enum Lookup {
    Collection(String),
    Item(String),
    Attachment(String),
}

impl Lookup {
    /// The 404's message: the one the handlers give for a missing resource.
    fn missing(&self) -> &'static str {
        match self {
            Lookup::Collection(_) => "no such collection",
            Lookup::Item(_) => "no such item",
            Lookup::Attachment(_) => "no such attachment",
        }
    }

    fn access(
        &self,
        conn: &rusqlite::Connection,
        viewer: &wikanvas_store::Viewer,
    ) -> wikanvas_store::Result<CollectionAccess> {
        use wikanvas_store::access;
        match self {
            Lookup::Collection(id) => access::collection(conn, viewer, id),
            Lookup::Item(id) => access::item(conn, viewer, id),
            // An attachment is only ever read.
            Lookup::Attachment(id) => Ok(if access::attachment_readable(conn, viewer, id)? {
                CollectionAccess::Read
            } else {
                CollectionAccess::None
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_without_a_collection() {
        let ok = |role, action, resource| role_allows(role, action, resource);
        for action in [Action::Read, Action::Write, Action::Manage, Action::Admin] {
            for role in [Role::Owner, Role::Admin] {
                assert!(ok(role, action, Resource::Workspace));
            }
        }
        assert!(ok(Role::Member, Action::Write, Resource::Workspace));
        assert!(!ok(Role::Member, Action::Admin, Resource::Workspace));
        assert!(!ok(Role::Member, Action::Manage, Resource::Workspace));
        assert!(ok(Role::Viewer, Action::Read, Resource::Workspace));
        assert!(!ok(Role::Viewer, Action::Write, Resource::Workspace));
        assert_eq!(needed(Action::Read), CollectionAccess::Read);
        assert_eq!(needed(Action::Write), CollectionAccess::Write);
        assert_eq!(needed(Action::Manage), CollectionAccess::Manage);
    }

    #[test]
    fn every_role_manages_its_own_account() {
        for role in [Role::Owner, Role::Admin, Role::Member, Role::Viewer] {
            for action in [Action::Read, Action::Write] {
                assert!(role_allows(role, action, Resource::OwnAccount));
            }
        }
        // Admin on the own account is still the role's: nothing asks for it today.
        assert!(!role_allows(
            Role::Member,
            Action::Admin,
            Resource::OwnAccount
        ));
        assert!(role_allows(
            Role::Admin,
            Action::Admin,
            Resource::OwnAccount
        ));
        // Another user's token, account or an invite is Admin.
        for resource in [
            Resource::Token("t"),
            Resource::User("u2"),
            Resource::Invite("i"),
        ] {
            assert!(!role_allows(Role::Member, Action::Admin, resource));
            assert!(!role_allows(Role::Viewer, Action::Admin, resource));
            assert!(role_allows(Role::Admin, Action::Admin, resource));
        }
    }
}
