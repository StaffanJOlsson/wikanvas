//! Personal access tokens: scopes with their hierarchy, `ApiToken`, creation (D42, ARCHITECTURE §7.3; owner: W2).
//! The token string format itself is in [`crate::token`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::common::{Timestamp, double_option, string_enum};

/// Expiry of a new token when `expires_in_days` is omitted.
pub const DEFAULT_EXPIRY_DAYS: u32 = 90;
/// The longest expiry a token can be created with (`null` = never is separate).
pub const MAX_EXPIRY_DAYS: u32 = 3650;
/// Creation refuses a 101st live token per user (422 `validation`).
pub const MAX_LIVE_TOKENS: usize = 100;

string_enum! {
    /// A token scope. The hierarchy is `read` ⊂ `write` ⊂ `admin` (the variant order); a session has every scope.
    /// `read`: every GET except token and session management and export; `write`: content and structure; `admin`:
    /// purge, workspace and account settings, sessions, tokens, export and backup (§7.3).
    pub enum Scope {
        Read = "read",
        Write = "write",
        Admin = "admin",
    }
}

impl Scope {
    /// Whether holding `self` allows what `required` needs (`admin` covers `write` covers `read`).
    pub fn covers(self, required: Scope) -> bool {
        self >= required
    }

    /// The scopes `self` implies, lowest first: the expanded form stored in `api_token_scopes`
    /// (`write` → `[read, write]`).
    pub fn expand(self) -> &'static [Scope] {
        match self {
            Scope::Read => &[Scope::Read],
            Scope::Write => &[Scope::Read, Scope::Write],
            Scope::Admin => &[Scope::Read, Scope::Write, Scope::Admin],
        }
    }

    /// The expanded union of `scopes`, sorted and without duplicates (what a token created with them stores).
    pub fn expand_all(scopes: &[Scope]) -> Vec<Scope> {
        Scope::highest(scopes)
            .map(|s| s.expand().to_vec())
            .unwrap_or_default()
    }

    /// The highest of `scopes`, or `None` for an empty set.
    pub fn highest(scopes: &[Scope]) -> Option<Scope> {
        scopes.iter().copied().max()
    }

    /// Whether a credential with the (expanded or not) set `granted` may do what `required` needs.
    pub fn granted(granted: &[Scope], required: Scope) -> bool {
        granted.iter().any(|g| g.covers(required))
    }
}

string_enum! {
    /// Where a token was created.
    pub enum CreatedVia {
        /// Settings → API tokens (a session).
        Web = "web",
        /// `POST /tokens` with a token, or the CLI.
        Api = "api",
        /// `quire-server admin create-token`.
        Admin = "admin",
    }
}

/// A personal access token as lists show it. Never the secret or its hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    /// The first 12 characters of the token (`qpat_` + 7), for recognizing it.
    pub prefix: String,
    /// Expanded: a `write` token lists `read` and `write`.
    pub scopes: Vec<Scope>,
    pub created_at: Timestamp,
    pub created_via: CreatedVia,
    /// The token that created this one; it is revoked with it (D42).
    pub parent_token_id: Option<Uuid>,
    /// `null`: never expires.
    pub expires_at: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
    pub last_used_ip: Option<String>,
}

/// Body of `POST /tokens`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NewToken {
    /// 1–100 characters.
    pub name: String,
    /// At most the caller's scopes; a calling token can never grant `admin`.
    pub scopes: Vec<Scope>,
    /// 1–3650. Omitted: 90 days; `null`: never (a session must then send `password`; a calling token's expiry caps
    /// it).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[ts(as = "Option<u32>")]
    #[schema(value_type = Option<u32>, minimum = 1, maximum = 3650)]
    pub expires_in_days: Option<Option<u32>>,
    /// The session's password, required for an `admin` or never-expiring token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl NewToken {
    /// The requested lifetime in days: omitted → 90, `null` → `None` (never).
    pub fn expiry_days(&self) -> Option<u32> {
        match self.expires_in_days {
            None => Some(DEFAULT_EXPIRY_DAYS),
            Some(days) => days,
        }
    }
}

/// Response of `POST /tokens`: the only response that ever contains the secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreatedToken {
    /// `qpat_…`, 51 characters.
    pub token: String,
    pub api_token: ApiToken,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_hierarchy() {
        use Scope::*;
        assert!(Admin.covers(Write) && Admin.covers(Read) && Admin.covers(Admin));
        assert!(Write.covers(Read) && Write.covers(Write) && !Write.covers(Admin));
        assert!(Read.covers(Read) && !Read.covers(Write) && !Read.covers(Admin));
        assert_eq!(Read.expand(), &[Read]);
        assert_eq!(Write.expand(), &[Read, Write]);
        assert_eq!(Admin.expand(), &[Read, Write, Admin]);
        assert_eq!(Scope::expand_all(&[Write, Read]), vec![Read, Write]);
        assert_eq!(Scope::expand_all(&[]), Vec::<Scope>::new());
        assert_eq!(Scope::highest(&[Read, Admin, Write]), Some(Admin));
        assert!(Scope::granted(&[Read, Write], Write));
        assert!(Scope::granted(&[Admin], Read));
        assert!(!Scope::granted(&[Read, Write], Admin));
        assert!(!Scope::granted(&[], Read));
        assert_eq!(
            serde_json::to_string(&[Read, Write, Admin]).unwrap(),
            r#"["read","write","admin"]"#
        );
        assert_eq!("admin".parse::<Scope>(), Ok(Admin));
        assert!("root".parse::<Scope>().is_err());
    }

    #[test]
    fn new_token_expiry_is_tri_state() {
        let parse = |s: &str| serde_json::from_str::<NewToken>(s).unwrap().expiry_days();
        assert_eq!(parse(r#"{"name":"a","scopes":["read"]}"#), Some(90));
        assert_eq!(
            parse(r#"{"name":"a","scopes":["read"],"expires_in_days":null}"#),
            None
        );
        assert_eq!(
            parse(r#"{"name":"a","scopes":["read"],"expires_in_days":7}"#),
            Some(7)
        );
        let never = NewToken {
            name: "a".into(),
            scopes: vec![Scope::Read],
            expires_in_days: Some(None),
            password: None,
        };
        assert_eq!(
            serde_json::to_string(&never).unwrap(),
            r#"{"name":"a","scopes":["read"],"expires_in_days":null}"#
        );
    }
}
