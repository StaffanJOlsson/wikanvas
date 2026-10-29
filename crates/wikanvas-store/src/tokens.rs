//! Personal access tokens: create, lookup, list and revoke (owner: W2).
//!
//! D42, §5 "Tokens", §7.3. A token is `qpat_` + 40 base62 characters from `getrandom` + a 6-character CRC32
//! (`wikanvas_api::token::encode`); only `sha256(token)` (32 bytes) and the 12-character display prefix are stored, and
//! scopes are stored expanded (`write` → read, write). A token is live when `revoked_at IS NULL AND (expires_at IS NULL
//! OR expires_at > now)` and its user has no `disabled_at`. Create (with the parent caps and the 100-live-tokens
//! limit), lookup with classification, list, revoke with children, the `last_used` write and the 30-day purge.
//! Password checks for a session's `admin` or never-expiring token are the route's.

use wikanvas_core::api::token;
use wikanvas_core::api::{
    ApiToken, CreatedToken, ErrorCode, MAX_EXPIRY_DAYS, MAX_LIVE_TOKENS, NewToken, Role, Scope,
    Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

pub use wikanvas_core::api::CreatedVia;

use crate::error::{Error, Result};
use crate::shared::{new_id, parse_uuid};

const DAY_MS: i64 = 86_400_000;

/// A live token, as [`lookup`] returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRow {
    pub id: String,
    pub user_id: String,
    /// The user's role (for `Actor.role`).
    pub role: Role,
    /// Expanded, lowest first.
    pub scopes: Vec<Scope>,
    /// `None`: never expires.
    pub expires_at: Option<i64>,
}

/// How [`lookup`] classifies a presented token (§7.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenLookup {
    /// Authenticates.
    Live(TokenRow),
    /// 401 `invalid_token` with `details.reason = "expired"`.
    Expired,
    /// Malformed, unknown, revoked, or of a disabled user: 401 `invalid_token` without a reason.
    Rejected,
}

/// sha256 of the whole token string, the `api_tokens.token_hash` key.
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Creates a token for `user_id` and returns the secret (the only time it exists outside the caller) with its
/// listing. `parent` is the calling token for a token caller (`None` for a session or `admin create-token`); it
/// becomes `parent_token_id` and caps the new token: `admin` scope gives 403 `session_required`, a scope above the
/// parent's gives 422 (`field: "scopes"`), and an expiry later than the parent's (or none while the parent expires)
/// gives 422 (`field: "expires_in_days"`). Also 422: a name outside 1–100 characters (`name`), no scopes (`scopes`),
/// `expires_in_days` outside 1–3650 (`expires_in_days`), and a 101st live token of the user (`field: "tokens"`).
pub fn create(
    tx: &Transaction<'_>,
    user_id: &str,
    new: &NewToken,
    via: CreatedVia,
    parent: Option<&TokenRow>,
    now: i64,
) -> Result<CreatedToken> {
    let name = new.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(Error::validation(
            "name",
            "the name must be 1 to 100 characters",
        ));
    }
    let Some(highest) = Scope::highest(&new.scopes) else {
        return Err(Error::validation(
            "scopes",
            "at least one scope is required",
        ));
    };
    let days = new.expiry_days();
    if let Some(d) = days
        && !(1..=MAX_EXPIRY_DAYS).contains(&d)
    {
        return Err(Error::validation(
            "expires_in_days",
            format!("expires_in_days must be 1 to {MAX_EXPIRY_DAYS}, or null for never"),
        ));
    }
    let expires_at = days.map(|d| now + i64::from(d) * DAY_MS);
    if let Some(p) = parent {
        if p.user_id != user_id {
            return Err(Error::new(
                ErrorCode::Forbidden,
                "a token can only create tokens for its own user",
            ));
        }
        if highest == Scope::Admin {
            return Err(Error::new(
                ErrorCode::SessionRequired,
                "a token cannot create an admin token; use a session",
            ));
        }
        if !Scope::granted(&p.scopes, highest) {
            return Err(Error::validation(
                "scopes",
                "a scope exceeds the calling token's scopes",
            ));
        }
        let outlives = match (p.expires_at, expires_at) {
            (Some(_), None) => true,
            (Some(pe), Some(e)) => e > pe,
            (None, _) => false,
        };
        if outlives {
            return Err(Error::validation(
                "expires_in_days",
                "the new token would expire later than the calling token",
            ));
        }
    }
    let live: i64 = tx.query_row(
        "SELECT count(*) FROM api_tokens
         WHERE user_id = ?1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?2)",
        params![user_id, now],
        |r| r.get(0),
    )?;
    if live as usize >= MAX_LIVE_TOKENS {
        return Err(Error::validation(
            "tokens",
            format!("at most {MAX_LIVE_TOKENS} live tokens per user; revoke one first"),
        ));
    }

    let mut random = [0u8; token::RANDOM_BYTES];
    getrandom::fill(&mut random)
        .map_err(|e| Error::new(ErrorCode::Internal, format!("getrandom: {e}")))?;
    let secret = token::encode(&random);
    let prefix = token::display_prefix(&secret).to_owned();
    let id = new_id();
    let scopes = Scope::expand_all(&new.scopes);
    tx.execute(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, prefix, created_via, parent_token_id, created_at,
           expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            user_id,
            name,
            &hash(&secret)[..],
            prefix,
            via.as_str(),
            parent.map(|p| p.id.as_str()),
            now,
            expires_at
        ],
    )?;
    let mut insert =
        tx.prepare_cached("INSERT INTO api_token_scopes (token_id, scope) VALUES (?1, ?2)")?;
    for s in &scopes {
        insert.execute(params![id, s.as_str()])?;
    }
    Ok(CreatedToken {
        token: secret,
        api_token: ApiToken {
            id: parse_uuid(&id)?,
            name: name.to_owned(),
            prefix,
            scopes,
            created_at: Timestamp(now),
            created_via: via,
            parent_token_id: parent.map(|p| parse_uuid(&p.id)).transpose()?,
            expires_at: expires_at.map(Timestamp),
            last_used_at: None,
            last_used_ip: None,
        },
    })
}

/// Looks a presented token up by its sha256, with no liveness filter and no cache, and classifies it (§7.3). A
/// string that fails `wikanvas_api::token::check` is [`TokenLookup::Rejected`] without a query. Revoked, unknown and
/// disabled-user tokens are all `Rejected`; an expired one is `Expired`.
pub fn lookup(conn: &Connection, token: &str, now: i64) -> Result<TokenLookup> {
    if token::check(token).is_err() {
        return Ok(TokenLookup::Rejected);
    }
    type Found = (
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        String,
    );
    let row: Option<Found> = conn
        .prepare_cached(
            "SELECT t.id, t.user_id, t.expires_at, t.revoked_at, u.disabled_at, u.role
             FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?1",
        )?
        .query_row([&hash(token)[..]], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .optional()?;
    let Some((id, user_id, expires_at, revoked_at, disabled_at, role)) = row else {
        return Ok(TokenLookup::Rejected);
    };
    if revoked_at.is_some() || disabled_at.is_some() {
        return Ok(TokenLookup::Rejected);
    }
    if expires_at.is_some_and(|e| e <= now) {
        return Ok(TokenLookup::Expired);
    }
    let role = role
        .parse::<Role>()
        .map_err(|e| Error::internal("stored role", e))?;
    let scopes = scopes_of(conn, &id)?;
    Ok(TokenLookup::Live(TokenRow {
        id,
        user_id,
        role,
        scopes,
        expires_at,
    }))
}

const TOKEN_COLUMNS: &str =
    "id, name, prefix, created_at, created_via, parent_token_id, expires_at, last_used_at,
   last_used_ip";

type TokenListRow = (
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

fn token_list_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TokenListRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ))
}

fn api_token(conn: &Connection, row: TokenListRow) -> Result<ApiToken> {
    let (id, name, prefix, created_at, via, parent, expires_at, last_used_at, last_used_ip) = row;
    Ok(ApiToken {
        scopes: scopes_of(conn, &id)?,
        id: parse_uuid(&id)?,
        name,
        prefix,
        created_at: Timestamp(created_at),
        created_via: via
            .parse::<CreatedVia>()
            .map_err(|e| Error::internal("stored created_via", e))?,
        parent_token_id: parent.as_deref().map(parse_uuid).transpose()?,
        expires_at: expires_at.map(Timestamp),
        last_used_at: last_used_at.map(Timestamp),
        last_used_ip,
    })
}

/// The user's tokens that are not revoked (live and expired), newest first, at most 100. Never the secret or hash.
/// Every live token is listed (there are at most 100); expired ones fill the rest, newest first, so newer expired
/// tokens never hide an older live one.
pub fn list(conn: &Connection, user_id: &str, now: i64) -> Result<Vec<ApiToken>> {
    let rows: Vec<TokenListRow> = conn
        .prepare_cached(&format!(
            "SELECT {TOKEN_COLUMNS} FROM (
               SELECT * FROM api_tokens WHERE user_id = ?1 AND revoked_at IS NULL
               ORDER BY (expires_at IS NULL OR expires_at > ?3) DESC, created_at DESC, id DESC LIMIT ?2
             ) ORDER BY created_at DESC, id DESC"
        ))?
        .query_map(params![user_id, MAX_LIVE_TOKENS as i64, now], token_list_row)?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter().map(|row| api_token(conn, row)).collect()
}

/// One of the user's tokens that is not revoked (live or expired), by id, as [`list`] shows it; `None` when the user
/// has no such token. `GET /auth/me` uses it to describe the calling token, however many newer tokens the user has.
pub fn get(conn: &Connection, user_id: &str, token_id: &str) -> Result<Option<ApiToken>> {
    let row = conn
        .prepare_cached(&format!(
            "SELECT {TOKEN_COLUMNS} FROM api_tokens WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL"
        ))?
        .query_row(params![token_id, user_id], token_list_row)
        .optional()?;
    row.map(|row| api_token(conn, row)).transpose()
}

/// Revokes the user's token `token_id` and, in the same statement, every not-yet-revoked token it created,
/// transitively (`parent_token_id`, D42). Returns the number of tokens revoked; 0 means no such live or expired
/// token of this user (already revoked, unknown, or another user's).
pub fn revoke(tx: &Transaction<'_>, user_id: &str, token_id: &str, now: i64) -> Result<u64> {
    let n = tx.execute(
        "WITH RECURSIVE tree(id) AS (
           SELECT id FROM api_tokens WHERE id = ?2 AND user_id = ?1 AND revoked_at IS NULL
           UNION
           SELECT t.id FROM api_tokens t JOIN tree ON t.parent_token_id = tree.id WHERE t.user_id = ?1
         )
         UPDATE api_tokens SET revoked_at = ?3 WHERE id IN (SELECT id FROM tree) AND revoked_at IS NULL",
        params![user_id, token_id, now],
    )?;
    Ok(n as u64)
}

/// The live token `token_id` as [`lookup`] would return it (the caller's own row, for the caps of `POST /tokens`);
/// `None` when it is revoked, expired, unknown or its user is disabled.
pub fn live_row(conn: &Connection, token_id: &str, now: i64) -> Result<Option<TokenRow>> {
    let row: Option<(String, Option<i64>, String)> = conn
        .prepare_cached(
            "SELECT t.user_id, t.expires_at, u.role FROM api_tokens t JOIN users u ON u.id = t.user_id
             WHERE t.id = ?1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?2)
               AND u.disabled_at IS NULL",
        )?
        .query_row(params![token_id, now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((user_id, expires_at, role)) = row else {
        return Ok(None);
    };
    Ok(Some(TokenRow {
        id: token_id.to_owned(),
        user_id,
        role: role
            .parse::<Role>()
            .map_err(|e| Error::internal("stored role", e))?,
        scopes: scopes_of(conn, token_id)?,
        expires_at,
    }))
}

/// Records a use of the token: `last_used_at = now` and `last_used_ip` (kept when `ip` is `None`). The server calls
/// it at most once a minute per token (§7.3), off the request path.
pub fn touch_last_used(
    tx: &Transaction<'_>,
    token_id: &str,
    now: i64,
    ip: Option<&str>,
) -> Result<()> {
    tx.prepare_cached(
        "UPDATE api_tokens SET last_used_at = ?2, last_used_ip = coalesce(?3, last_used_ip) WHERE id = ?1",
    )?
    .execute(params![token_id, now, ip])?;
    Ok(())
}

/// Revokes every token of the user that is not revoked yet (`reset-password --revoke-tokens`,
/// `revoke-token --all`); returns the number revoked.
pub fn revoke_all(tx: &Transaction<'_>, user_id: &str, now: i64) -> Result<u64> {
    Ok(tx.execute(
        "UPDATE api_tokens SET revoked_at = ?2 WHERE user_id = ?1 AND revoked_at IS NULL",
        params![user_id, now],
    )? as u64)
}

/// How long revoked and expired rows are kept (§5).
pub const RETAIN_DEAD_MS: i64 = 30 * DAY_MS;

/// Deletes the tokens revoked or expired more than 30 days before `now` (the hourly job, §12); their scopes go with
/// them (FK cascade) and children keep living with `parent_token_id = NULL`. Returns the number deleted.
pub fn purge(tx: &Transaction<'_>, now: i64) -> Result<u64> {
    let cutoff = now - RETAIN_DEAD_MS;
    Ok(tx.execute(
        "DELETE FROM api_tokens WHERE (revoked_at IS NOT NULL AND revoked_at <= ?1)
                                  OR (expires_at IS NOT NULL AND expires_at <= ?1)",
        [cutoff],
    )? as u64)
}

fn scopes_of(conn: &Connection, token_id: &str) -> Result<Vec<Scope>> {
    let mut scopes = conn
        .prepare_cached("SELECT scope FROM api_token_scopes WHERE token_id = ?1")?
        .query_map([token_id], |r| r.get::<_, String>(0))?
        .map(|s| {
            s.map_err(Error::from).and_then(|s| {
                s.parse::<Scope>()
                    .map_err(|e| Error::internal("stored scope", e))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    scopes.sort();
    Ok(scopes)
}
