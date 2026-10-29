//! Login sessions: create, lookup, renewal, revocation and expiry (owner: W2).
//!
//! D18, §8: a session is an opaque 32-byte token; the cookie carries it as base64url and the row keeps only its
//! sha256 (`token_hash`). A persistent session ("Keep me signed in") has a 30-day window capped at
//! `created_at + 180 d`, any other a 24-hour window. When less than half of the window remains, an authenticated
//! request moves `expires_at` to `now + window` ([`renewal`]). `last_seen_at` is touched at most every 5 minutes.

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use sha2::{Digest, Sha256};
use wikanvas_core::api::{ErrorCode, Role, SessionInfo, Timestamp};

use crate::error::{Error, Result};
use crate::shared::{new_id, parse_uuid};

const DAY_MS: i64 = 86_400_000;
/// The window of a persistent session.
pub const PERSISTENT_WINDOW_MS: i64 = 30 * DAY_MS;
/// A persistent session never outlives `created_at` + this.
pub const PERSISTENT_CAP_MS: i64 = 180 * DAY_MS;
/// The window of a browser-session (not persistent) session.
pub const SESSION_WINDOW_MS: i64 = DAY_MS;
/// `last_seen_at` is written at most this often.
pub const TOUCH_INTERVAL_MS: i64 = 5 * 60_000;
/// The longest stored `User-Agent`.
pub const MAX_USER_AGENT: usize = 512;

/// One live session, with its user's role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub user_id: String,
    pub role: Role,
    pub persistent: bool,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

impl SessionRow {
    /// The `GET /auth/sessions` entry; `current` marks the caller's own session.
    pub fn to_api(&self, current: bool) -> Result<SessionInfo> {
        Ok(SessionInfo {
            id: parse_uuid(&self.id)?,
            created_at: Timestamp(self.created_at),
            last_seen_at: Timestamp(self.last_seen_at),
            user_agent: self.user_agent.clone(),
            ip: self.ip.clone(),
            current,
        })
    }

    /// The cookie's `Max-Age` in seconds for a persistent session (`None` for a browser-session cookie).
    pub fn max_age_secs(&self, now: i64) -> Option<i64> {
        self.persistent
            .then(|| ((self.expires_at - now).max(0) + 999) / 1000)
    }
}

/// A session to create.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewSession {
    pub user_id: String,
    pub persistent: bool,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

/// sha256 of a session token, as stored in `sessions.token_hash`; `None` when `cookie` is not the base64url form of
/// 32 bytes.
pub fn token_hash(cookie: &str) -> Option<[u8; 32]> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cookie)
        .ok()?;
    (raw.len() == 32).then(|| Sha256::digest(&raw).into())
}

/// The window of a session kind.
pub fn window(persistent: bool) -> i64 {
    if persistent {
        PERSISTENT_WINDOW_MS
    } else {
        SESSION_WINDOW_MS
    }
}

/// The new `expires_at` when the session is due for renewal at `now` (less than half of its window left), or `None`.
/// A persistent session is capped at `created_at + 180 d`, so near the cap there is nothing to extend.
pub fn renewal(row: &SessionRow, now: i64) -> Option<i64> {
    let window = window(row.persistent);
    if row.expires_at - now >= window / 2 {
        return None;
    }
    let mut next = now + window;
    if row.persistent {
        next = next.min(row.created_at + PERSISTENT_CAP_MS);
    }
    (next > row.expires_at).then_some(next)
}

/// Creates a session and returns it with the cookie value (base64url of 32 random bytes), which exists nowhere else.
pub fn create(tx: &Transaction<'_>, new: &NewSession, now: i64) -> Result<(SessionRow, String)> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw)
        .map_err(|e| Error::new(ErrorCode::Internal, format!("getrandom: {e}")))?;
    let cookie = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    let hash: [u8; 32] = Sha256::digest(raw).into();
    let id = new_id();
    let expires_at = now + window(new.persistent);
    let user_agent = new
        .user_agent
        .as_deref()
        .map(|ua| truncate(ua, MAX_USER_AGENT));
    tx.prepare_cached(
        "INSERT INTO sessions (id, token_hash, user_id, persistent, created_at, last_seen_at, expires_at, user_agent, ip)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        id,
        hash.as_slice(),
        new.user_id,
        new.persistent,
        now,
        expires_at,
        user_agent,
        new.ip
    ])?;
    let role: String = tx.query_row(
        "SELECT role FROM users WHERE id = ?1",
        [&new.user_id],
        |r| r.get(0),
    )?;
    let row = SessionRow {
        id,
        user_id: new.user_id.clone(),
        role: role
            .parse()
            .map_err(|e| Error::internal("stored role", e))?,
        persistent: new.persistent,
        created_at: now,
        last_seen_at: now,
        expires_at,
        user_agent,
        ip: new.ip.clone(),
    };
    Ok((row, cookie))
}

fn truncate(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

const COLUMNS: &str = "s.id, s.user_id, u.role, s.persistent, s.created_at, s.last_seen_at, s.expires_at, \
     s.user_agent, s.ip";

fn from_row(r: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let role: String = r.get(2)?;
    Ok(SessionRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        role: role.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?,
        persistent: r.get(3)?,
        created_at: r.get(4)?,
        last_seen_at: r.get(5)?,
        expires_at: r.get(6)?,
        user_agent: r.get(7)?,
        ip: r.get(8)?,
    })
}

/// The live session whose cookie value is `cookie`: not expired, of a user that is not disabled. `None` for anything
/// else, including a cookie that is not 32 bytes of base64url (no query then).
pub fn lookup(conn: &Connection, cookie: &str, now: i64) -> Result<Option<SessionRow>> {
    let Some(hash) = token_hash(cookie) else {
        return Ok(None);
    };
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.disabled_at IS NULL"
        ))?
        .query_row(params![hash.as_slice(), now], from_row)
        .optional()?)
}

/// Records use of a session: a new `expires_at` (from [`renewal`]) and/or `last_seen_at = now` with the client IP.
pub fn refresh(
    tx: &Transaction<'_>,
    id: &str,
    expires_at: Option<i64>,
    now: i64,
    ip: Option<&str>,
) -> Result<()> {
    tx.prepare_cached(
        "UPDATE sessions SET expires_at = coalesce(?2, expires_at), last_seen_at = ?3, ip = coalesce(?4, ip)
         WHERE id = ?1",
    )?
    .execute(params![id, expires_at, now, ip])?;
    Ok(())
}

/// Whether a request at `now` should write [`refresh`]: renewal is due or `last_seen_at` is older than 5 minutes.
pub fn needs_refresh(row: &SessionRow, now: i64) -> bool {
    renewal(row, now).is_some() || now - row.last_seen_at >= TOUCH_INTERVAL_MS
}

/// The user's live sessions, most recently seen first.
pub fn list(conn: &Connection, user_id: &str, now: i64) -> Result<Vec<SessionRow>> {
    let rows = conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.user_id = ?1 AND s.expires_at > ?2 ORDER BY s.last_seen_at DESC, s.id DESC"
        ))?
        .query_map(params![user_id, now], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes the user's session `id`; returns the number deleted (0: none of the user's).
pub fn delete(tx: &Transaction<'_>, user_id: &str, id: &str) -> Result<u64> {
    Ok(tx.execute(
        "DELETE FROM sessions WHERE id = ?1 AND user_id = ?2",
        params![id, user_id],
    )? as u64)
}

/// Deletes every session of the user except `keep` (all of them with `None`); returns the number deleted.
pub fn delete_others(tx: &Transaction<'_>, user_id: &str, keep: Option<&str>) -> Result<u64> {
    Ok(tx.execute(
        "DELETE FROM sessions WHERE user_id = ?1 AND id IS NOT ?2",
        params![user_id, keep],
    )? as u64)
}

/// Deletes every expired session (the hourly job, §12); returns the number deleted.
pub fn delete_expired(tx: &Transaction<'_>, now: i64) -> Result<u64> {
    Ok(tx.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])? as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(persistent: bool, created_at: i64, expires_at: i64) -> SessionRow {
        SessionRow {
            id: "s".into(),
            user_id: "u".into(),
            role: Role::Owner,
            persistent,
            created_at,
            last_seen_at: created_at,
            expires_at,
            user_agent: None,
            ip: None,
        }
    }

    #[test]
    fn renewal_after_half_the_window() {
        let s = row(false, 0, SESSION_WINDOW_MS);
        assert_eq!(renewal(&s, SESSION_WINDOW_MS / 2 - 1), None);
        assert_eq!(
            renewal(&s, SESSION_WINDOW_MS / 2 + 1),
            Some(SESSION_WINDOW_MS / 2 + 1 + SESSION_WINDOW_MS)
        );
        let p = row(true, 0, PERSISTENT_WINDOW_MS);
        let at = 16 * DAY_MS;
        assert_eq!(renewal(&p, at), Some(at + PERSISTENT_WINDOW_MS));
        // Near the 180-day cap the new expiry is the cap, and at the cap nothing changes.
        let late = row(true, 0, 170 * DAY_MS);
        assert_eq!(renewal(&late, 160 * DAY_MS), Some(PERSISTENT_CAP_MS));
        let capped = row(true, 0, PERSISTENT_CAP_MS);
        assert_eq!(renewal(&capped, 170 * DAY_MS), None);
    }

    #[test]
    fn max_age_and_hash() {
        let p = row(true, 0, 30_000);
        assert_eq!(p.max_age_secs(0), Some(30));
        assert_eq!(row(false, 0, 30_000).max_age_secs(0), None);
        assert!(token_hash("short").is_none());
        let cookie = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 32]);
        assert_eq!(token_hash(&cookie), Some(Sha256::digest([1u8; 32]).into()));
        assert_eq!(truncate("åäö", 3), "å");
    }
}
