//! Users: the owner row, the other accounts, profile changes and passwords (owner: W2).
//!
//! There is exactly one owner (§5: `users_single_owner`). [`create_owner`] is its one insert, guarded by `NOT EXISTS`
//! so two concurrent setups cannot both succeed (the unique index backs it up). Every other account is an `admin`,
//! `member` or `viewer` made by [`create`]: through an accepted invite (`invites::accept`) or `wikanvas-server admin
//! create-user` (D148). Emails are unique ignoring ASCII case (`COLLATE NOCASE`). Passwords are argon2id PHC strings
//! hashed by the server (D17); the store never sees a plain password.
//!
//! The account rules of D148 live in [`change`] and [`Manager::may_manage`], for the API and the admin commands alike:
//! the owner's role never changes and the owner cannot be disabled; nobody changes their own role or disables
//! themselves; only the owner makes or unmakes an admin; admins manage members and viewers. Disabling ends the account's
//! sessions, API tokens and pending invites at once (login and every lookup refuse a disabled user anyway).

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use wikanvas_core::api::{ErrorCode, Role, Theme, Timestamp, User, UserInfo, ValidationDetails};

use crate::error::{Error, Result};
use crate::shared::{new_id, parse_uuid};

/// One row of `users`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRow {
    pub id: String,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub theme: Theme,
    /// The argon2id PHC string; `None` only in an export copy (or for SSO users later).
    pub password_hash: Option<String>,
    pub must_change_password: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub disabled_at: Option<i64>,
}

impl UserRow {
    /// The API shape.
    pub fn to_api(&self) -> Result<User> {
        Ok(User {
            id: parse_uuid(&self.id)?,
            email: self.email.clone(),
            name: self.name.clone(),
            role: self.role,
            theme: self.theme,
        })
    }
}

impl UserRow {
    /// The `GET /users` entry.
    pub fn to_info(&self) -> Result<UserInfo> {
        Ok(UserInfo {
            id: parse_uuid(&self.id)?,
            email: self.email.clone(),
            name: self.name.clone(),
            role: self.role,
            created_at: Timestamp(self.created_at),
            disabled_at: self.disabled_at.map(Timestamp),
        })
    }
}

/// The owner to create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOwner {
    pub email: String,
    pub name: String,
    pub password_hash: String,
    /// A generated password must be changed at the first sign-in.
    pub must_change_password: bool,
}

const COLUMNS: &str = "id, email, name, role, theme, password_hash, must_change_password, created_at, updated_at, \
     disabled_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<UserRow> {
    let enum_err = |i: usize, e: wikanvas_core::api::UnknownValue| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
    };
    let role: String = r.get(3)?;
    let theme: String = r.get(4)?;
    Ok(UserRow {
        id: r.get(0)?,
        email: r.get(1)?,
        name: r.get(2)?,
        role: role.parse().map_err(|e| enum_err(3, e))?,
        theme: theme.parse().map_err(|e| enum_err(4, e))?,
        password_hash: r.get(5)?,
        must_change_password: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        disabled_at: r.get(9)?,
    })
}

/// A user by id; `not_found` when missing.
pub fn get(conn: &Connection, id: &str) -> Result<UserRow> {
    conn.prepare_cached(&format!("SELECT {COLUMNS} FROM users WHERE id = ?1"))?
        .query_row([id], from_row)
        .optional()?
        .ok_or_else(|| Error::not_found("no such user"))
}

/// A user by email (case-insensitive, after trimming), whatever its state.
pub fn by_email(conn: &Connection, email: &str) -> Result<Option<UserRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM users WHERE email = ?1"))?
        .query_row([email.trim()], from_row)
        .optional()?)
}

/// The owner, if setup has happened.
pub fn owner(conn: &Connection) -> Result<Option<UserRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM users WHERE role = 'owner'"))?
        .query_row([], from_row)
        .optional()?)
}

/// Every user, the owner first (in the order they were created), disabled ones included.
pub fn list(conn: &Connection) -> Result<Vec<UserRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM users ORDER BY role <> 'owner', created_at, id"
        ))?
        .query_map([], from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

/// The number of users that are not disabled (`config.multi_user` is this above 1).
pub fn count_enabled(conn: &Connection) -> Result<u64> {
    let n: i64 = conn
        .prepare_cached("SELECT count(*) FROM users WHERE disabled_at IS NULL")?
        .query_row([], |r| r.get(0))?;
    Ok(n as u64)
}

/// Whether an owner exists.
pub fn owner_exists(conn: &Connection) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM users WHERE role = 'owner')")?
        .query_row([], |r| r.get(0))?)
}

/// Checks an email address: one `@` with text on both sides, no whitespace, at most 254 bytes (422 `email`).
pub fn check_email(email: &str) -> Result<()> {
    let ok = email.len() <= 254
        && !email.contains(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
    if ok {
        Ok(())
    } else {
        Err(Error::validation("email", "not an email address"))
    }
}

/// Checks a display name: 1–200 characters after trimming (422 `name`).
pub fn check_name(name: &str) -> Result<()> {
    if (1..=200).contains(&name.trim().chars().count()) {
        Ok(())
    } else {
        Err(Error::validation("name", "a name is 1 to 200 characters"))
    }
}

/// Inserts the owner, guarded by `NOT EXISTS` an owner in the same statement: 409 `already_setup` when one exists.
/// The email and name are trimmed and validated (422 `email` / `name`).
pub fn create_owner(tx: &Transaction<'_>, new: &NewOwner, now: i64) -> Result<UserRow> {
    let email = new.email.trim();
    check_email(email)?;
    check_name(&new.name)?;
    let id = new_id();
    let inserted = tx.execute(
        "INSERT INTO users (id, email, name, password_hash, role, theme, must_change_password, created_at, updated_at)
         SELECT ?1, ?2, ?3, ?4, 'owner', 'system', ?5, ?6, ?6
         WHERE NOT EXISTS (SELECT 1 FROM users WHERE role = 'owner')",
        params![
            id,
            email,
            new.name.trim(),
            new.password_hash,
            new.must_change_password,
            now
        ],
    );
    match inserted {
        Ok(1) => get(tx, &id),
        Ok(_) => Err(Error::new(
            ErrorCode::AlreadySetup,
            "an owner already exists",
        )),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(Error::new(
                ErrorCode::AlreadySetup,
                "an owner already exists",
            ))
        }
        Err(e) => Err(e.into()),
    }
}

/// An account to create beside the owner (an accepted invite, `admin create-user`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewUser {
    pub email: String,
    pub name: String,
    pub password_hash: String,
    /// `admin`, `member` or `viewer` (`owner` is 422 `role`).
    pub role: Role,
    /// A generated password must be changed at the first sign-in.
    pub must_change_password: bool,
}

/// The 409 `conflict` of an email another user has (`details.field = "email"`).
fn email_taken() -> Error {
    Error::new(ErrorCode::Conflict, "another user has this email").with_details(
        &ValidationDetails {
            field: "email".into(),
        },
    )
}

/// Inserts an `admin`, `member` or `viewer`. The email and name are trimmed and validated (422 `email` / `name`); an
/// email another user has, ignoring case, is 409 `conflict` (`field: "email"`); the owner is 422 `role`.
pub fn create(tx: &Transaction<'_>, new: &NewUser, now: i64) -> Result<UserRow> {
    if new.role == Role::Owner {
        return Err(Error::validation(
            "role",
            "the role is admin, member or viewer: the owner is made once, at setup",
        ));
    }
    let email = new.email.trim();
    check_email(email)?;
    check_name(&new.name)?;
    let id = new_id();
    let inserted = tx.execute(
        "INSERT INTO users (id, email, name, password_hash, role, theme, must_change_password, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'system', ?6, ?7, ?7)",
        params![
            id,
            email,
            new.name.trim(),
            new.password_hash,
            new.role.as_str(),
            new.must_change_password,
            now
        ],
    );
    match inserted {
        Ok(_) => get(tx, &id),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(email_taken())
        }
        Err(e) => Err(e.into()),
    }
}

/// Who changes an account (D148): a signed-in user through the API, or the server's operator through
/// `wikanvas-server admin`, who holds the owner's say over every account but has none of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager<'a> {
    User { id: &'a str, role: Role },
    Operator,
}

impl Manager<'_> {
    /// The manager's own account (none for the operator).
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Manager::User { id, .. } => Some(id),
            Manager::Operator => None,
        }
    }

    /// Whether it may change, disable, enable or invite accounts of `role`: admins only by the owner (and the
    /// operator), members and viewers also by admins, the owner by nobody.
    pub fn may_manage(&self, role: Role) -> bool {
        let owner = matches!(
            self,
            Manager::Operator
                | Manager::User {
                    role: Role::Owner,
                    ..
                }
        );
        let admin = owner
            || matches!(
                self,
                Manager::User {
                    role: Role::Admin,
                    ..
                }
            );
        match role {
            Role::Owner => false,
            Role::Admin => owner,
            Role::Member | Role::Viewer => admin,
        }
    }
}

/// A change to an account (`PATCH /users/{id}`, `admin set-role`, `disable-user`, `enable-user`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserChange {
    pub role: Option<Role>,
    pub disabled: Option<bool>,
}

/// What [`change`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub user: UserRow,
    /// Sessions deleted because the account was disabled.
    pub sessions_revoked: u64,
    /// API tokens revoked because the account was disabled.
    pub tokens_revoked: u64,
    /// Pending invites the account made, revoked because it was disabled or lost the admin role.
    pub invites_revoked: u64,
}

/// 403 `forbidden` for a change the rules of D148 refuse.
fn refused(message: &str) -> Error {
    Error::new(ErrorCode::Forbidden, message)
}

/// Applies `change` to the user `id` by `by`, under the rules of the module docs: 404 for an unknown user, 422 `role`
/// for `owner`, 403 `forbidden` for the owner's account, the manager's own, an admin's (or a new admin) when `by` is
/// not the owner. A change to what the account already is changes nothing. Disabling deletes its sessions and revokes
/// its API tokens and the pending invites it made; taking the admin role away revokes those invites too.
pub fn change(
    tx: &Transaction<'_>,
    by: Manager<'_>,
    id: &str,
    change: &UserChange,
    now: i64,
) -> Result<Changed> {
    let user = get(tx, id)?;
    let unchanged = |user: UserRow| Changed {
        user,
        sessions_revoked: 0,
        tokens_revoked: 0,
        invites_revoked: 0,
    };
    if change.role.is_none() && change.disabled.is_none() {
        return Ok(unchanged(user));
    }
    if change.role == Some(Role::Owner) {
        return Err(Error::validation(
            "role",
            "the role is admin, member or viewer: there is one owner, made at setup",
        ));
    }
    if user.role == Role::Owner {
        return Err(refused(
            "the owner's role never changes, and the owner cannot be disabled",
        ));
    }
    if by.user_id() == Some(user.id.as_str()) {
        return Err(refused(
            "you cannot change your own role or disable your own account",
        ));
    }
    if !by.may_manage(user.role) || change.role.is_some_and(|r| !by.may_manage(r)) {
        return Err(refused("only the owner makes, changes or removes an admin"));
    }
    let role = change.role.unwrap_or(user.role);
    let disable = change.disabled.unwrap_or(user.disabled_at.is_some());
    if role == user.role && disable == user.disabled_at.is_some() {
        return Ok(unchanged(user));
    }
    let disabled_at = match (disable, user.disabled_at) {
        (true, Some(at)) => Some(at),
        (true, None) => Some(now),
        (false, _) => None,
    };
    // The role caps collection access and a disabled account has none (D152): the tree ETag's epoch moves on.
    tx.execute(
        "UPDATE users SET role = ?2, disabled_at = ?3, updated_at = ?4, access_epoch = access_epoch + 1 WHERE id = ?1",
        params![id, role.as_str(), disabled_at, now],
    )?;
    let newly_disabled = disable && user.disabled_at.is_none();
    let (sessions_revoked, tokens_revoked) = if newly_disabled {
        (
            crate::sessions::delete_others(tx, id, None)?,
            crate::tokens::revoke_all(tx, id, now)?,
        )
    } else {
        (0, 0)
    };
    let loses_admin = user.role == Role::Admin && role != Role::Admin;
    let invites_revoked = if newly_disabled || loses_admin {
        crate::invites::revoke_created_by(tx, id, now)?
    } else {
        0
    };
    Ok(Changed {
        user: get(tx, id)?,
        sessions_revoked,
        tokens_revoked,
        invites_revoked,
    })
}

/// Sets a new password hash and the `must_change_password` flag.
pub fn set_password(
    tx: &Transaction<'_>,
    id: &str,
    password_hash: &str,
    must_change_password: bool,
    now: i64,
) -> Result<()> {
    let n = tx.execute(
        "UPDATE users SET password_hash = ?2, must_change_password = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, password_hash, must_change_password, now],
    )?;
    if n == 0 {
        return Err(Error::not_found("no such user"));
    }
    Ok(())
}

/// A profile change (`PATCH /auth/me`); omitted fields stay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub email: Option<String>,
    pub theme: Option<Theme>,
}

/// Applies `patch` (validated: 422 `name` / `email`; an email another user has: 409 `conflict`) and returns the row.
pub fn update_profile(
    tx: &Transaction<'_>,
    id: &str,
    patch: &ProfilePatch,
    now: i64,
) -> Result<UserRow> {
    let current = get(tx, id)?;
    let name = match &patch.name {
        Some(n) => {
            check_name(n)?;
            n.trim().to_owned()
        }
        None => current.name.clone(),
    };
    let email = match &patch.email {
        Some(e) => {
            let e = e.trim();
            check_email(e)?;
            e.to_owned()
        }
        None => current.email.clone(),
    };
    let theme = patch.theme.unwrap_or(current.theme);
    if (&name, &email, theme) == (&current.name, &current.email, current.theme) {
        return Ok(current);
    }
    let updated = tx.execute(
        "UPDATE users SET name = ?2, email = ?3, theme = ?4, updated_at = ?5 WHERE id = ?1",
        params![id, name, email, theme.as_str(), now],
    );
    match updated {
        Ok(_) => get(tx, id),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(email_taken())
        }
        Err(e) => Err(e.into()),
    }
}
