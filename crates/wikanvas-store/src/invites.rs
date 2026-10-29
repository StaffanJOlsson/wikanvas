//! Invite links: create, list, revoke, look up by token, accept (D148, §5 "Invites", §8).
//!
//! An invite is the only way to a new account besides `wikanvas-server admin create-user`. Its token is drawn and
//! encoded like an API token (D42: 32 bytes from `getrandom`, base62, a CRC32 checksum), with the prefix `qinv_`
//! (`wikanvas_api::token::encode_invite`); only `sha256(token)` is stored, and the token exists once, in the URL
//! [`create`] returns to its caller. An invite is pending until it is used (single use), expires (7 days by default,
//! at most 30) or is revoked. [`accept`] creates the user with the invite's role and marks the invite used in the
//! same transaction, so two accepts of one invite cannot both succeed. Who may invite for which role is
//! [`Manager::may_manage`]: an admin invite only by the owner.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use sha2::{Digest, Sha256};
use wikanvas_core::api::token;
use wikanvas_core::api::{
    ErrorCode, Invite, InviteState, MAX_INVITE_DAYS, MAX_INVITE_NOTE, MAX_PENDING_INVITES,
    NewInvite, Role, Timestamp,
};

use crate::error::{Error, Result};
use crate::shared::{new_id, parse_uuid};
use crate::users::{Manager, NewUser, UserRow};

const DAY_MS: i64 = 86_400_000;
/// Revoked invites, and expired ones nobody used, are deleted this long after (the hourly job, §12). Used invites stay:
/// they record who invited whom.
pub const RETAIN_DEAD_MS: i64 = 30 * DAY_MS;

/// One row of `invites`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteRow {
    pub id: String,
    pub role: Role,
    pub note: Option<String>,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    pub used_by: Option<String>,
    pub revoked_at: Option<i64>,
}

impl InviteRow {
    /// Where it stands at `now`: used and revoked win over expired.
    pub fn state(&self, now: i64) -> InviteState {
        if self.used_at.is_some() {
            InviteState::Used
        } else if self.revoked_at.is_some() {
            InviteState::Revoked
        } else if self.expires_at <= now {
            InviteState::Expired
        } else {
            InviteState::Pending
        }
    }

    /// The API shape (never the token).
    pub fn to_api(&self) -> Result<Invite> {
        Ok(Invite {
            id: parse_uuid(&self.id)?,
            role: self.role,
            note: self.note.clone(),
            created_by: self.created_by.as_deref().map(parse_uuid).transpose()?,
            created_at: Timestamp(self.created_at),
            expires_at: Timestamp(self.expires_at),
        })
    }
}

/// A new invite and its token, which exists nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub invite: InviteRow,
    /// `qinv_…`, 51 characters.
    pub token: String,
}

const COLUMNS: &str =
    "id, role, note, created_by, created_at, expires_at, used_at, used_by, revoked_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<InviteRow> {
    let role: String = r.get(1)?;
    Ok(InviteRow {
        id: r.get(0)?,
        role: role.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?,
        note: r.get(2)?,
        created_by: r.get(3)?,
        created_at: r.get(4)?,
        expires_at: r.get(5)?,
        used_at: r.get(6)?,
        used_by: r.get(7)?,
        revoked_at: r.get(8)?,
    })
}

/// sha256 of the whole token, the `invites.token_hash` key.
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Creates an invite by `by` and returns it with its token. 422: `role` for `owner`, `expires_in_days` outside 1–30,
/// `note` over 200 characters (a blank note is none), `invites` for a 101st pending invite; 403 `forbidden` for an
/// admin invite by anyone but the owner (or the operator).
pub fn create(tx: &Transaction<'_>, by: Manager<'_>, new: &NewInvite, now: i64) -> Result<Created> {
    if new.role == Role::Owner {
        return Err(Error::validation(
            "role",
            "an invite's role is admin, member or viewer",
        ));
    }
    let days = new.expiry_days();
    if !(1..=MAX_INVITE_DAYS).contains(&days) {
        return Err(Error::validation(
            "expires_in_days",
            format!("an invite expires after 1 to {MAX_INVITE_DAYS} days"),
        ));
    }
    let note = new.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > MAX_INVITE_NOTE) {
        return Err(Error::validation(
            "note",
            format!("a note is at most {MAX_INVITE_NOTE} characters"),
        ));
    }
    if !by.may_manage(new.role) {
        return Err(Error::new(
            ErrorCode::Forbidden,
            "only the owner invites an admin",
        ));
    }
    let pending: i64 = tx.query_row(
        "SELECT count(*) FROM invites WHERE used_at IS NULL AND revoked_at IS NULL AND expires_at > ?1",
        [now],
        |r| r.get(0),
    )?;
    if pending as usize >= MAX_PENDING_INVITES {
        return Err(Error::validation(
            "invites",
            format!("at most {MAX_PENDING_INVITES} pending invites; revoke one first"),
        ));
    }
    let mut random = [0u8; token::RANDOM_BYTES];
    getrandom::fill(&mut random)
        .map_err(|e| Error::new(ErrorCode::Internal, format!("getrandom: {e}")))?;
    let secret = token::encode_invite(&random);
    let id = new_id();
    tx.execute(
        "INSERT INTO invites (id, token_hash, role, note, created_by, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            &hash(&secret)[..],
            new.role.as_str(),
            note,
            by.user_id(),
            now,
            now + i64::from(days) * DAY_MS
        ],
    )?;
    Ok(Created {
        invite: get(tx, &id)?.ok_or_else(|| Error::not_found("no such invite"))?,
        token: secret,
    })
}

fn get(conn: &Connection, id: &str) -> Result<Option<InviteRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM invites WHERE id = ?1"))?
        .query_row([id], from_row)
        .optional()?)
}

/// The pending invites (not used, not revoked, not expired), newest first. At most 100 exist.
pub fn list_pending(conn: &Connection, now: i64) -> Result<Vec<InviteRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM invites
             WHERE used_at IS NULL AND revoked_at IS NULL AND expires_at > ?1
             ORDER BY created_at DESC, id DESC"
        ))?
        .query_map([now], from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

/// Revokes the pending invite `id`; returns the number revoked (0: unknown, used, revoked or expired).
pub fn revoke(tx: &Transaction<'_>, id: &str, now: i64) -> Result<u64> {
    Ok(tx.execute(
        "UPDATE invites SET revoked_at = ?2
         WHERE id = ?1 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?2",
        params![id, now],
    )? as u64)
}

/// Revokes every pending invite `user_id` made (the account was disabled or lost the admin role); returns the number.
pub fn revoke_created_by(tx: &Transaction<'_>, user_id: &str, now: i64) -> Result<u64> {
    Ok(tx.execute(
        "UPDATE invites SET revoked_at = ?2
         WHERE created_by = ?1 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?2",
        params![user_id, now],
    )? as u64)
}

/// The invite whose token is `token`, whatever its state; `None` for a string that is not an invite token (no query
/// then) or that no invite has.
pub fn lookup(conn: &Connection, token: &str) -> Result<Option<InviteRow>> {
    if token::check_invite(token).is_err() {
        return Ok(None);
    }
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM invites WHERE token_hash = ?1"
        ))?
        .query_row([&hash(token)[..]], from_row)
        .optional()?)
}

/// 404 `not_found` for a token that is not a pending invite, with the reason when the invite exists.
pub fn not_pending(state: Option<InviteState>) -> Error {
    Error::not_found(match state {
        Some(InviteState::Used) => "this invite link was used already: an invite works once",
        Some(InviteState::Expired) => "this invite link has expired: ask for a new one",
        Some(InviteState::Revoked) => "this invite link was revoked: ask for a new one",
        _ => "no such invite: check the link",
    })
}

/// The account an accepted invite creates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceptance {
    pub email: String,
    pub name: String,
    pub password_hash: String,
}

/// Accepts the invite with `token`: creates the user with its role (`users::create`: 422 `email` / `name`, 409
/// `conflict` for a taken email) and marks the invite used by them, in the caller's transaction. 404 `not_found` when
/// the token is not a pending invite.
pub fn accept(tx: &Transaction<'_>, token: &str, who: &Acceptance, now: i64) -> Result<UserRow> {
    let invite = lookup(tx, token)?;
    let state = invite.as_ref().map(|i| i.state(now));
    let Some(invite) = invite.filter(|_| state == Some(InviteState::Pending)) else {
        return Err(not_pending(state));
    };
    let user = crate::users::create(
        tx,
        &NewUser {
            email: who.email.clone(),
            name: who.name.clone(),
            password_hash: who.password_hash.clone(),
            role: invite.role,
            must_change_password: false,
        },
        now,
    )?;
    let n = tx.execute(
        "UPDATE invites SET used_at = ?2, used_by = ?3 WHERE id = ?1 AND used_at IS NULL AND revoked_at IS NULL",
        params![invite.id, now, user.id],
    )?;
    if n != 1 {
        return Err(not_pending(Some(InviteState::Used)));
    }
    Ok(user)
}

/// Deletes revoked invites, and expired ones nobody used, more than 30 days after (the hourly job, §12). Returns the
/// number deleted.
pub fn purge(tx: &Transaction<'_>, now: i64) -> Result<u64> {
    let cutoff = now - RETAIN_DEAD_MS;
    Ok(tx.execute(
        "DELETE FROM invites WHERE used_at IS NULL
           AND ((revoked_at IS NOT NULL AND revoked_at <= ?1) OR expires_at <= ?1)",
        [cutoff],
    )? as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> InviteRow {
        InviteRow {
            id: "i".into(),
            role: Role::Member,
            note: None,
            created_by: None,
            created_at: 0,
            expires_at: 100,
            used_at: None,
            used_by: None,
            revoked_at: None,
        }
    }

    #[test]
    fn states() {
        let pending = row();
        assert_eq!(pending.state(99), InviteState::Pending);
        assert_eq!(pending.state(100), InviteState::Expired);
        let used = InviteRow {
            used_at: Some(50),
            ..row()
        };
        assert_eq!(used.state(200), InviteState::Used);
        let revoked = InviteRow {
            revoked_at: Some(50),
            ..row()
        };
        assert_eq!(revoked.state(200), InviteState::Revoked);
        assert_eq!(not_pending(None).code, ErrorCode::NotFound);
    }
}
