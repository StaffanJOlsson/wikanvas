//! Setup, login, the current user, sessions, the workspace and `GET /config` (ARCHITECTURE §7.2, §7.3, §8;
//! owner: W2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::common::{Theme, Timestamp, User, double_option, string_enum};
use crate::tokens::ApiToken;

/// `{user}`: the response of setup, login and `PATCH /auth/me`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UserEnvelope {
    pub user: User,
}

/// Response of `GET /setup`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SetupStatus {
    pub needs_setup: bool,
}

/// Body of `POST /setup` (rate-limited per IP; 409 `already_setup` once an owner exists).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SetupRequest {
    /// The setup code from the server log (`XXXX-XXXX-XXXX`) or `QUIRE_SETUP_TOKEN`.
    pub code: String,
    pub workspace_name: String,
    pub name: String,
    pub email: String,
    /// 12–1024 bytes.
    pub password: String,
}

/// Body of `POST /auth/login`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    /// "Keep me signed in": a persistent cookie with a 30-day window.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remember: bool,
}

string_enum! {
    /// How the current request was authenticated.
    pub enum AuthMethodKind {
        Session = "session",
        Token = "token",
    }
}

/// `Me.auth`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MeAuth {
    pub method: AuthMethodKind,
    /// The calling token, when `method` is `token`.
    pub token: Option<ApiToken>,
}

/// The workspace's name and logo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Workspace {
    pub name: String,
    /// `/api/v1/attachments/<id>`, or `null` without a logo.
    pub logo_url: Option<String>,
}

/// Response of `GET /auth/me`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Me {
    pub user: User,
    pub workspace: Workspace,
    pub must_change_password: bool,
    pub auth: MeAuth,
}

/// Body of `PATCH /auth/me`. Changing `email` needs a session (a token gets 403 `session_required`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
}

/// Body of `POST /auth/password` (session only; revokes the other sessions).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
}

/// An entry of `GET /auth/sessions`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SessionInfo {
    pub id: Uuid,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    /// The session of this request.
    pub current: bool,
}

/// `?others=true` of `DELETE /auth/sessions`: revoke every session but the caller's (from a token: every session).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct SessionRevokeQuery {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub others: bool,
}

/// Body of `PATCH /workspace`. `logo_attachment_id: null` removes the logo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct WorkspacePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[ts(as = "Option<Uuid>")]
    #[schema(value_type = Option<Uuid>)]
    pub logo_attachment_id: Option<Option<Uuid>>,
}

/// Response of `GET /config` (public).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Config {
    /// The server version.
    pub version: String,
    /// The git commit the server was built from.
    pub commit: String,
    /// `quire_api::API_VERSION`.
    pub api_version: String,
    /// `/api/v1/openapi.json`.
    pub openapi_url: String,
    pub setup_required: bool,
    /// `false` in M1.
    pub multi_user: bool,
    /// `null` when no Inkboard build is embedded.
    pub inkboard: Option<InkboardInfo>,
    pub limits: Limits,
}

/// `Config.inkboard`: the embedded engine build (§10.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct InkboardInfo {
    /// `/inkboard/<build>/`.
    pub base: String,
    /// `0.2.0-beta.3+embed.quire.<patchset>`.
    pub engine_version: String,
    /// The envelope version the engine writes (2).
    pub format_version: u32,
    pub wasm_bytes: WasmBytes,
}

/// Sizes of the engine's wasm file and its precompressed variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct WasmBytes {
    pub raw: u64,
    pub br: u64,
    pub gz: u64,
}

/// `Config.limits`: every request limit of §7.1 and D33.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Limits {
    pub max_upload_mb: u64,
    pub max_board_mb: u64,
    pub max_doc_mb: u64,
    pub max_update_bytes: u64,
    pub max_markdown_bytes: u64,
    pub max_json_bytes: u64,
    pub max_body_bytes: u64,
    pub json_depth: u32,
    pub q_max_chars: u32,
    pub page_max: u32,
    pub token_rate_per_min: u32,
    pub token_burst: u32,
    pub token_bytes_per_hour: u64,
}

/// Response of `GET /healthz` (outside `/api/v1` and the spec).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Health {
    /// `ok`.
    pub status: String,
}
