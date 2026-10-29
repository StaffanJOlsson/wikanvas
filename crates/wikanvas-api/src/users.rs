//! Accounts and invite links (D148, ARCHITECTURE §7.2, §8): the account list, role and disable changes, and the
//! invites that are the only way to a new account. The invite token's format is in [`crate::token`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::common::{Role, Timestamp, string_enum};

/// Expiry of an invite when `expires_in_days` is omitted.
pub const DEFAULT_INVITE_DAYS: u32 = 7;
/// The longest expiry an invite can be created with.
pub const MAX_INVITE_DAYS: u32 = 30;
/// Creation refuses a 101st pending invite (422 `validation`, `field: "invites"`).
pub const MAX_PENDING_INVITES: usize = 100;
/// The longest invite note, in characters.
pub const MAX_INVITE_NOTE: usize = 200;

/// An account as `GET /users` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UserInfo {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub created_at: Timestamp,
    /// When the account was disabled; `null` while it can sign in.
    pub disabled_at: Option<Timestamp>,
}

/// Body of `PATCH /users/{id}`: a new role, and/or disable (`true`) or enable (`false`) the account. Disabling ends
/// the account's sessions, API tokens and pending invites at once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UserPatch {
    /// `admin`, `member` or `viewer`; only the owner makes or unmakes an admin, and nobody changes the owner's role
    /// or their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

string_enum! {
    /// Where an invite stands.
    pub enum InviteState {
        /// Can be accepted.
        Pending = "pending",
        /// Accepted once; an invite is single-use.
        Used = "used",
        /// Past `expires_at`.
        Expired = "expired",
        /// Revoked by the owner or an admin.
        Revoked = "revoked",
    }
}

/// An invite as the owner and admins see it. Never its token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Invite {
    pub id: Uuid,
    /// The role the new account gets: `admin`, `member` or `viewer`.
    pub role: Role,
    /// A note for the inviters ("For Alex"), never shown on the invite page.
    pub note: Option<String>,
    /// Who made it; `null` once that account is gone.
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// Body of `POST /invites`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NewInvite {
    /// `admin` (the owner only), `member` or `viewer`.
    pub role: Role,
    /// 1–30; omitted: 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, maximum = 30)]
    pub expires_in_days: Option<u32>,
    /// At most 200 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl NewInvite {
    /// The requested lifetime in days (omitted: 7).
    pub fn expiry_days(&self) -> u32 {
        self.expires_in_days.unwrap_or(DEFAULT_INVITE_DAYS)
    }
}

/// Response of `POST /invites`: the only response that contains the invite's link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreatedInvite {
    pub invite: Invite,
    /// `<public URL>/invite/<token>`; the token (`qinv_…`, 51 characters) is its last segment.
    pub url: String,
}

/// Response of the public `GET /invites/{id}` (the `id` is the invite's token): what the invite page shows. Never
/// who made the invite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct InviteLookup {
    /// Whether it can be accepted (`state` is `pending`).
    pub valid: bool,
    pub state: InviteState,
    pub workspace_name: String,
    /// The role the new account gets.
    pub role: Role,
    pub expires_at: Timestamp,
}

/// Body of the public `POST /invites/{id}/accept` (the `id` is the invite's token).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct AcceptInvite {
    pub name: String,
    pub email: String,
    /// 12–1024 bytes, as at setup.
    pub password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes() {
        let patch: UserPatch = serde_json::from_str(r#"{"role":"viewer"}"#).unwrap();
        assert_eq!(patch.role, Some(Role::Viewer));
        assert_eq!(patch.disabled, None);
        assert_eq!(serde_json::to_string(&UserPatch::default()).unwrap(), "{}");
        let new: NewInvite = serde_json::from_str(r#"{"role":"member"}"#).unwrap();
        assert_eq!(new.expiry_days(), DEFAULT_INVITE_DAYS);
        assert_eq!(InviteState::Expired.as_str(), "expired");
        assert_eq!("used".parse::<InviteState>(), Ok(InviteState::Used));
    }
}
