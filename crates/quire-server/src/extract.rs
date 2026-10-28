//! Extractors: Auth<S> (session cookie or bearer token, with scopes), Actor, Viewer and ItemRef (owner: W2)
//!
//! `Auth<S>` is always the first argument of a handler (D44), so authentication and the scope check answer before
//! any path, query or body is parsed. Resolution (§7.3):
//! 1. while no owner exists: 409 `setup_required` (the owner-exists cache re-checks on every request while false);
//! 2. a Quire bearer request ([`crate::csrf::is_quire_bearer`]) is authenticated by the token alone: a malformed
//!    token is 401 `invalid_token` without a lookup; otherwise `tokens::lookup` classifies it (live; expired: 401
//!    with `details.reason = "expired"`; revoked, disabled or unknown: 401). The cookie is ignored;
//! 3. any other request (no `Authorization`, or any other value) takes the session cookie named for the request's
//!    scheme (`__Host-quire_session` over HTTPS from the trusted proxy, `quire_session` otherwise); none, unknown or
//!    expired is 401 `unauthenticated`;
//! 4. the requirement `S`: `Read`/`Write`/`Admin` need that scope (a session has every scope), else 403
//!    `insufficient_scope`; `Session` answers a token with 403 `session_required`; `AnyToken` answers a session with
//!    403 `forbidden`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};
use std::ops::Deref;

use axum::extract::{ConnectInfo, FromRequestParts, Path};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use base64::Engine as _;
use quire_api::{ErrorCode, Role, Scope};
use quire_store::tokens::TokenLookup;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::{Config, canonical_ip};
use crate::csrf::bearer_token;
use crate::error::ApiError;
use crate::state::{AppState, now_ms};

pub use quire_store::shared::Viewer;

/// The session cookie on a plain-HTTP request.
pub const SESSION_COOKIE: &str = "quire_session";
/// The session cookie on an HTTPS request (behind the trusted proxy).
pub const SESSION_COOKIE_SECURE: &str = "__Host-quire_session";

/// How the request was authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// The session cookie (a session has every scope).
    Session { session_id: String },
    /// A personal access token; `scopes` are expanded (`write` → read, write).
    Token {
        token_id: String,
        scopes: Vec<Scope>,
    },
}

/// The authenticated caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    pub user_id: String,
    pub role: Role,
    pub method: AuthMethod,
}

impl Actor {
    /// Whether the caller may do what `required` needs.
    pub fn has_scope(&self, required: Scope) -> bool {
        match &self.method {
            AuthMethod::Session { .. } => true,
            AuthMethod::Token { scopes, .. } => Scope::granted(scopes, required),
        }
    }

    /// The calling token's id, for a token request.
    pub fn token_id(&self) -> Option<&str> {
        match &self.method {
            AuthMethod::Token { token_id, .. } => Some(token_id),
            AuthMethod::Session { .. } => None,
        }
    }

    /// The calling session's id, for a cookie request.
    pub fn session_id(&self) -> Option<&str> {
        match &self.method {
            AuthMethod::Session { session_id } => Some(session_id),
            AuthMethod::Token { .. } => None,
        }
    }

    /// The scopes of the caller: every scope for a session.
    pub fn scopes(&self) -> Vec<Scope> {
        match &self.method {
            AuthMethod::Session { .. } => Scope::ALL.to_vec(),
            AuthMethod::Token { scopes, .. } => scopes.clone(),
        }
    }

    /// The viewer for list-shaped store functions (`shared::visible_items`).
    pub fn viewer(&self) -> Viewer {
        Viewer {
            user_id: self.user_id.clone(),
            role: self.role,
        }
    }

    /// The author of a content write (D53).
    pub fn write_by(&self) -> quire_store::shared::WriteBy<'_> {
        quire_store::shared::WriteBy {
            user_id: &self.user_id,
            token_id: self.token_id(),
        }
    }
}

/// What a route requires of its caller (the `S` of [`Auth<S>`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// A token with this scope, or a session.
    Scope(Scope),
    /// The session cookie only.
    Session,
    /// Any valid token, never a session.
    AnyToken,
}

/// Marker types for [`Auth<S>`].
pub trait Require: Send + Sync + 'static {
    const REQUIREMENT: Requirement;
}

/// `read` scope (or a session).
pub struct Read;
/// `write` scope (or a session).
pub struct Write;
/// `admin` scope (or a session).
pub struct Admin;
/// The session cookie only (password change, logout).
pub struct Session;
/// Any valid token, never a session (`DELETE /tokens/current`).
pub struct AnyToken;

impl Require for Read {
    const REQUIREMENT: Requirement = Requirement::Scope(Scope::Read);
}
impl Require for Write {
    const REQUIREMENT: Requirement = Requirement::Scope(Scope::Write);
}
impl Require for Admin {
    const REQUIREMENT: Requirement = Requirement::Scope(Scope::Admin);
}
impl Require for Session {
    const REQUIREMENT: Requirement = Requirement::Session;
}
impl Require for AnyToken {
    const REQUIREMENT: Requirement = Requirement::AnyToken;
}

/// The authenticated caller of a route that requires `S`; derefs to [`Actor`]. Always the first handler argument.
pub struct Auth<S> {
    pub actor: Actor,
    _requirement: PhantomData<fn() -> S>,
}

impl<S> Auth<S> {
    pub fn into_actor(self) -> Actor {
        self.actor
    }
}

impl<S> Deref for Auth<S> {
    type Target = Actor;

    fn deref(&self) -> &Actor {
        &self.actor
    }
}

impl<S> std::fmt::Debug for Auth<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Auth").field(&self.actor).finish()
    }
}

/// Checks `actor` against a requirement (step 4 of the module docs).
pub fn check(actor: &Actor, requirement: Requirement) -> Result<(), ApiError> {
    match (requirement, &actor.method) {
        (Requirement::Scope(scope), _) if actor.has_scope(scope) => Ok(()),
        (Requirement::Scope(scope), AuthMethod::Token { scopes, .. }) => {
            Err(ApiError::insufficient_scope(scope, scopes))
        }
        (Requirement::Scope(_), AuthMethod::Session { .. }) => Ok(()),
        (Requirement::Session, AuthMethod::Session { .. }) => Ok(()),
        (Requirement::Session, AuthMethod::Token { .. }) => Err(ApiError::new(
            ErrorCode::SessionRequired,
            "this operation needs a signed-in browser session, not a token",
        )),
        (Requirement::AnyToken, AuthMethod::Token { .. }) => Ok(()),
        (Requirement::AnyToken, AuthMethod::Session { .. }) => Err(ApiError::new(
            ErrorCode::Forbidden,
            "this operation needs an API token, not a session",
        )),
    }
}

impl<S: Require> FromRequestParts<AppState> for Auth<S> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let actor = authenticate(parts, state).await?;
        check(&actor, S::REQUIREMENT)?;
        Ok(Auth {
            actor,
            _requirement: PhantomData,
        })
    }
}

impl FromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(authenticate(parts, state).await?.viewer())
    }
}

/// The authenticated actor of the request, resolved once and cached in the request's extensions.
pub async fn authenticate(parts: &mut Parts, state: &AppState) -> Result<Actor, ApiError> {
    if let Some(actor) = parts.extensions.get::<Actor>() {
        return Ok(actor.clone());
    }
    if !state.owner_exists().await? {
        return Err(ApiError::new(
            ErrorCode::SetupRequired,
            "no owner exists yet: finish setup at /setup or run quire-server admin create-owner",
        ));
    }
    let client = ClientInfo::from_parts(parts, &state.config);
    let actor = match bearer_token(&parts.headers) {
        Some(token) => authenticate_token(state, token.to_owned()).await?,
        None => authenticate_cookie(state, &parts.headers, client.https).await?,
    };
    parts.extensions.insert(actor.clone());
    Ok(actor)
}

async fn authenticate_token(state: &AppState, token: String) -> Result<Actor, ApiError> {
    if quire_api::token::check(&token).is_err() {
        return Err(ApiError::invalid_token(false));
    }
    let now = now_ms();
    let found = state
        .db
        .read(move |conn| {
            Ok(match quire_store::tokens::lookup(conn, &token, now)? {
                TokenLookup::Live(row) => Ok(Actor {
                    user_id: row.user_id,
                    role: row.role,
                    method: AuthMethod::Token {
                        token_id: row.id,
                        scopes: Scope::expand_all(&row.scopes),
                    },
                }),
                TokenLookup::Expired => Err(true),
                TokenLookup::Rejected => Err(false),
            })
        })
        .await?;
    found.map_err(ApiError::invalid_token)
}

/// The value of cookie `name` in the request's `Cookie` headers.
pub fn cookie_value<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().trim_matches('"'))
}

/// sha256 of a session token, as stored in `sessions.token_hash`. `None` when `cookie` is not the base64url form of 32
/// bytes.
pub fn session_token_hash(cookie: &str) -> Option<[u8; 32]> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cookie)
        .ok()?;
    (raw.len() == 32).then(|| Sha256::digest(&raw).into())
}

async fn authenticate_cookie(
    state: &AppState,
    headers: &HeaderMap,
    https: bool,
) -> Result<Actor, ApiError> {
    let name = if https {
        SESSION_COOKIE_SECURE
    } else {
        SESSION_COOKIE
    };
    let hash = cookie_value(headers, name)
        .and_then(session_token_hash)
        .ok_or_else(ApiError::unauthenticated)?;
    let now = now_ms();
    let row = state
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.user_id, u.role FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.disabled_at IS NULL",
            )?;
            let mut rows = stmt.query(rusqlite::params![hash.as_slice(), now])?;
            Ok(match rows.next()? {
                Some(r) => Some((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                )),
                None => None,
            })
        })
        .await?;
    let (session_id, user_id, role) = row.ok_or_else(ApiError::unauthenticated)?;
    Ok(Actor {
        user_id,
        role: role.parse().map_err(ApiError::internal)?,
        method: AuthMethod::Session { session_id },
    })
}

/// The client's address and scheme (§8): the socket peer and plain HTTP, unless the peer is in
/// `QUIRE_TRUSTED_PROXIES`; from a trusted peer, the rightmost `X-Forwarded-For` address and
/// `X-Forwarded-Proto: https`. `Forwarded` is never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// `None` when the peer is unknown (an in-process test transport).
    pub ip: Option<IpAddr>,
    /// The request reached the trusted proxy over HTTPS.
    pub https: bool,
}

impl ClientInfo {
    pub fn from_parts(parts: &Parts, config: &Config) -> ClientInfo {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| canonical_ip(addr.ip()));
        let trusted = peer.is_some_and(|ip| config.trusted_proxies.iter().any(|n| n.contains(ip)));
        if !trusted {
            return ClientInfo {
                ip: peer,
                https: false,
            };
        }
        let forwarded = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .rfind(|s| !s.is_empty())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .map(canonical_ip);
        let https = parts
            .headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
        ClientInfo {
            ip: forwarded.or(peer),
            https,
        }
    }
}

impl FromRequestParts<AppState> for ClientInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientInfo::from_parts(parts, &state.config))
    }
}

/// The `{id}` path parameter, also on routes with more parameters (`…/board/pages/{n}`).
async fn id_param<S: Send + Sync>(parts: &mut Parts, state: &S) -> Option<String> {
    let Path(mut params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
        .await
        .ok()?;
    params.remove("id")
}

/// The `{id}` path parameter of a route that also accepts a `url_id` (`GET /items/{id}`, `GET /collections/{id}`):
/// a UUID, or 10 base62 characters. Anything else is 404 `not_found`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemRef {
    Id(Uuid),
    UrlId(String),
}

/// The length of a `url_id` (D4).
pub const URL_ID_LEN: usize = 10;

impl ItemRef {
    pub fn parse(s: &str) -> Option<ItemRef> {
        if let Ok(id) = Uuid::parse_str(s) {
            return Some(ItemRef::Id(id));
        }
        (s.len() == URL_ID_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then(|| ItemRef::UrlId(s.to_owned()))
    }

    /// The column to look the row up by: `id` or `url_id`.
    pub fn column(&self) -> &'static str {
        match self {
            ItemRef::Id(_) => "id",
            ItemRef::UrlId(_) => "url_id",
        }
    }

    /// The value for [`column`](Self::column), as stored.
    pub fn value(&self) -> String {
        match self {
            ItemRef::Id(id) => id.to_string(),
            ItemRef::UrlId(u) => u.clone(),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ItemRef {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let raw = id_param(parts, state).await;
        raw.as_deref()
            .and_then(ItemRef::parse)
            .ok_or_else(|| ApiError::not_found("no such item"))
    }
}

/// A `{id}` path parameter that must be a UUID; anything else is 404 `not_found` (§7.1: path ids are UUIDs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdPath(pub Uuid);

impl<S: Send + Sync> FromRequestParts<S> for IdPath {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        id_param(parts, state)
            .await
            .and_then(|raw| Uuid::parse_str(&raw).ok())
            .map(IdPath)
            .ok_or_else(|| ApiError::not_found("not found"))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn token_actor(scopes: &[Scope]) -> Actor {
        Actor {
            user_id: "u".into(),
            role: Role::Owner,
            method: AuthMethod::Token {
                token_id: "t".into(),
                scopes: Scope::expand_all(scopes),
            },
        }
    }

    fn session_actor() -> Actor {
        Actor {
            user_id: "u".into(),
            role: Role::Owner,
            method: AuthMethod::Session {
                session_id: "s".into(),
            },
        }
    }

    fn code(r: Result<(), ApiError>) -> Option<ErrorCode> {
        r.err().map(|e| e.code)
    }

    #[test]
    fn requirements() {
        let read = token_actor(&[Scope::Read]);
        let write = token_actor(&[Scope::Write]);
        let session = session_actor();
        let scope = |s| Requirement::Scope(s);
        assert_eq!(code(check(&read, scope(Scope::Read))), None);
        assert_eq!(
            code(check(&read, scope(Scope::Write))),
            Some(ErrorCode::InsufficientScope)
        );
        assert_eq!(code(check(&write, scope(Scope::Read))), None);
        assert_eq!(
            code(check(&write, scope(Scope::Admin))),
            Some(ErrorCode::InsufficientScope)
        );
        assert_eq!(code(check(&session, scope(Scope::Admin))), None);
        assert_eq!(
            code(check(&write, Requirement::Session)),
            Some(ErrorCode::SessionRequired)
        );
        assert_eq!(code(check(&session, Requirement::Session)), None);
        assert_eq!(code(check(&read, Requirement::AnyToken)), None);
        assert_eq!(
            code(check(&session, Requirement::AnyToken)),
            Some(ErrorCode::Forbidden)
        );
        let err = check(&write, scope(Scope::Admin)).unwrap_err();
        assert_eq!(
            err.details.unwrap(),
            serde_json::json!({"required": "admin", "granted": ["read", "write"]})
        );
    }

    #[test]
    fn cookies_and_session_hashes() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; quire_session=abc"),
        );
        h.append(
            header::COOKIE,
            HeaderValue::from_static("__Host-quire_session=def"),
        );
        assert_eq!(cookie_value(&h, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie_value(&h, SESSION_COOKIE_SECURE), Some("def"));
        assert_eq!(cookie_value(&h, "missing"), None);
        let raw = [7u8; 32];
        let cookie = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let expect: [u8; 32] = Sha256::digest(raw).into();
        assert_eq!(session_token_hash(&cookie), Some(expect));
        assert_eq!(session_token_hash("abc"), None);
        assert_eq!(session_token_hash("!!!"), None);
    }

    #[test]
    fn item_refs() {
        let id = Uuid::now_v7();
        assert_eq!(ItemRef::parse(&id.to_string()), Some(ItemRef::Id(id)));
        assert_eq!(
            ItemRef::parse("aB3dE5gH9k"),
            Some(ItemRef::UrlId("aB3dE5gH9k".into()))
        );
        assert_eq!(ItemRef::parse("short"), None);
        assert_eq!(ItemRef::parse("aB3dE5gH9-"), None);
        assert_eq!(ItemRef::UrlId("x".into()).column(), "url_id");
    }

    #[test]
    fn client_info_trusts_only_configured_proxies() {
        let vars = [("QUIRE_TRUSTED_PROXIES", "172.31.240.2")];
        let config = Config::from_lookup(|k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        })
        .unwrap();
        let parts = |peer: &str| {
            let mut req = axum::http::Request::builder()
                .header("x-forwarded-for", "203.0.113.9, 198.51.100.7")
                .header("x-forwarded-proto", "https")
                .body(())
                .unwrap();
            let addr: SocketAddr = peer.parse().unwrap();
            req.extensions_mut().insert(ConnectInfo(addr));
            req.into_parts().0
        };
        let from_proxy = ClientInfo::from_parts(&parts("172.31.240.2:5000"), &config);
        assert_eq!(from_proxy.ip, Some("198.51.100.7".parse().unwrap()));
        assert!(from_proxy.https);
        let direct = ClientInfo::from_parts(&parts("10.0.0.5:5000"), &config);
        assert_eq!(direct.ip, Some("10.0.0.5".parse().unwrap()));
        assert!(!direct.https);
        let no_peer = axum::http::Request::builder()
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert_eq!(
            ClientInfo::from_parts(&no_peer, &config),
            ClientInfo {
                ip: None,
                https: false
            }
        );
    }
}
