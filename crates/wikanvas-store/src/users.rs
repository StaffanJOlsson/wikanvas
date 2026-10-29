//! Users: the owner row, profile changes and passwords (owner: W2).
//!
//! M1 has one user, the owner (§5: `users_single_owner`). [`create_owner`] is the one insert, guarded by `NOT EXISTS`
//! so two concurrent setups cannot both succeed (the unique index backs it up). Passwords are argon2id PHC strings
//! hashed by the server (D17); the store never sees a plain password.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use wikanvas_core::api::{ErrorCode, Role, Theme, User};

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
            Err(
                Error::new(ErrorCode::Conflict, "another user has this email").with_details(
                    &wikanvas_core::api::ValidationDetails {
                        field: "email".into(),
                    },
                ),
            )
        }
        Err(e) => Err(e.into()),
    }
}
