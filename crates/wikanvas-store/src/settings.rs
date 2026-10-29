//! Workspace and instance settings (owner: W2).
//!
//! The `settings` table holds one JSON value per key (§5): `workspace.name` (string), `workspace.logo_attachment_id`
//! (string, an attachment id), `instance.id` (string) and `instance.created_at` (number, unix ms). Values are JSON so a
//! later setting can be any shape without a migration.

use wikanvas_core::api::Workspace;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;

use crate::error::Result;
use crate::shared::new_id;

/// The workspace name (a JSON string).
pub const WORKSPACE_NAME: &str = "workspace.name";
/// The workspace logo: an attachment id (a JSON string). An attachment GC root (§5).
pub const WORKSPACE_LOGO: &str = "workspace.logo_attachment_id";
/// A random id of this instance (a JSON string), set once.
pub const INSTANCE_ID: &str = "instance.id";
/// When the instance was set up (a JSON number, unix ms), set once.
pub const INSTANCE_CREATED_AT: &str = "instance.created_at";

/// A setting's value, `None` when unset (or not valid JSON, which the `CHECK` rules out).
pub fn get(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = conn
        .prepare_cached("SELECT value FROM settings WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?;
    Ok(raw.and_then(|v| serde_json::from_str(&v).ok()))
}

/// A setting that holds a JSON string.
pub fn get_str(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(get(conn, key)?.and_then(|v| v.as_str().map(str::to_owned)))
}

/// Sets `key` to `value`, replacing an earlier value.
pub fn set(tx: &Transaction<'_>, key: &str, value: &Value) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )?
    .execute(params![key, serde_json::to_string(value)?])?;
    Ok(())
}

/// Removes `key` (nothing happens when it is unset).
pub fn delete(tx: &Transaction<'_>, key: &str) -> Result<()> {
    tx.prepare_cached("DELETE FROM settings WHERE key = ?1")?
        .execute([key])?;
    Ok(())
}

/// The workspace settings of a new instance: the workspace name, and `instance.id` / `instance.created_at` unless
/// they exist already.
pub fn init_workspace(tx: &Transaction<'_>, workspace_name: &str, now: i64) -> Result<()> {
    set(
        tx,
        WORKSPACE_NAME,
        &Value::String(workspace_name.to_owned()),
    )?;
    tx.prepare_cached("INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2), (?3, ?4)")?
        .execute(params![
            INSTANCE_ID,
            serde_json::to_string(&new_id())?,
            INSTANCE_CREATED_AT,
            now.to_string()
        ])?;
    Ok(())
}

/// The URL of an attachment (`/api/v1/attachments/<id>`).
pub fn attachment_url(id: &str) -> String {
    format!("/api/v1/attachments/{id}")
}

/// The workspace's name and logo URL.
pub fn workspace(conn: &Connection) -> Result<Workspace> {
    Ok(Workspace {
        name: get_str(conn, WORKSPACE_NAME)?.unwrap_or_default(),
        logo_url: get_str(conn, WORKSPACE_LOGO)?.map(|id| attachment_url(&id)),
    })
}
