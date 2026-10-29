//! Access per collection (D152): what a viewer may do in a collection, an item's collection and with an attachment;
//! the memberships of a collection; the directory of people to pick from; and the per-user access epoch of the tree
//! ETag (D21).
//!
//! The rule is `CollectionAccess::effective`: the owner and admins have `manage` on every collection; anyone else
//! has the greater of the collection's `default_access` and their membership (`collection_members`), capped by their
//! role (a viewer reads at most). An item has its collection's access, whatever the item's or the collection's state
//! (the routes refuse what a state does not allow). Lists select through `shared::visible_items` and
//! `shared::readable_collections`, the same rule in SQL. Groups, later, add a second kind of membership here and in
//! those two functions: the greater of every membership counts.
//!
//! An attachment is readable by the owner and admins, by the account that uploaded it, by everyone when it is the
//! workspace logo, and otherwise by whoever may read an item that shows it (`item_attachments`, D36); never through
//! `attachments.item_id`, the upload context.

use std::collections::HashSet;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use uuid::Uuid;
use wikanvas_core::api::{
    CollectionAccess, CollectionMember, DefaultAccess, DirectoryUser, MemberAccess, Role, Timestamp,
};

use crate::error::{Error, Result};
use crate::settings::WORKSPACE_LOGO;
use crate::shared::{Viewer, parse_uuid, readable_collections};

/// A stored access value; a malformed one is `internal`.
fn parse<T: std::str::FromStr<Err = wikanvas_core::api::UnknownValue>>(s: &str) -> Result<T> {
    s.parse()
        .map_err(|e| Error::internal("stored collection access", e))
}

/// The access of `role` to a collection with `default` and the stored membership `member`.
fn effective(role: Role, default: &str, member: Option<&str>) -> Result<CollectionAccess> {
    let default: DefaultAccess = parse(default)?;
    let member: Option<MemberAccess> = member.map(parse).transpose()?;
    Ok(CollectionAccess::effective(role, default, member))
}

/// The viewer's access to a collection, by its id or `url_id`, in any state; `none` when there is no such
/// collection (which the caller answers as for one the viewer may not read: 404).
pub fn collection(
    conn: &Connection,
    viewer: &Viewer,
    id_or_url_id: &str,
) -> Result<CollectionAccess> {
    let column = if Uuid::parse_str(id_or_url_id).is_ok() {
        "id"
    } else {
        "url_id"
    };
    let row: Option<(String, Option<String>)> = conn
        .prepare_cached(&format!(
            "SELECT c.default_access,
               (SELECT m.access FROM collection_members m WHERE m.collection_id = c.id AND m.user_id = ?2)
             FROM collections c WHERE c.{column} = ?1"
        ))?
        .query_row(params![id_or_url_id, viewer.user_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    match row {
        None => Ok(CollectionAccess::None),
        Some((default, member)) => effective(viewer.role, &default, member.as_deref()),
    }
}

/// The viewer's access to the collection of an item, by the item's id or `url_id`, in any state; `none` when there
/// is no such item.
pub fn item(conn: &Connection, viewer: &Viewer, id_or_url_id: &str) -> Result<CollectionAccess> {
    let column = if Uuid::parse_str(id_or_url_id).is_ok() {
        "id"
    } else {
        "url_id"
    };
    let row: Option<(String, Option<String>)> = conn
        .prepare_cached(&format!(
            "SELECT c.default_access,
               (SELECT m.access FROM collection_members m WHERE m.collection_id = c.id AND m.user_id = ?2)
             FROM items i JOIN collections c ON c.id = i.collection_id WHERE i.{column} = ?1"
        ))?
        .query_row(params![id_or_url_id, viewer.user_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    match row {
        None => Ok(CollectionAccess::None),
        Some((default, member)) => effective(viewer.role, &default, member.as_deref()),
    }
}

/// Whether the viewer may read the attachment `id` (module docs). False when there is none.
pub fn attachment_readable(conn: &Connection, viewer: &Viewer, id: &str) -> Result<bool> {
    if matches!(viewer.role, Role::Owner | Role::Admin) {
        return Ok(conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM attachments WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?);
    }
    let readable = readable_collections(viewer);
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM attachments a WHERE a.id = ? AND (
           a.created_by = ?
           OR a.id = (SELECT json_extract(s.value, '$') FROM settings s WHERE s.key = ?)
           OR EXISTS (SELECT 1 FROM item_attachments ia JOIN items i ON i.id = ia.item_id
                        JOIN collections c ON c.id = i.collection_id
                      WHERE ia.attachment_id = a.id AND {})))",
        readable.sql
    );
    let mut args = vec![
        Value::Text(id.to_owned()),
        Value::Text(viewer.user_id.clone()),
        Value::Text(WORKSPACE_LOGO.to_owned()),
    ];
    args.extend(readable.args);
    Ok(conn
        .prepare_cached(&sql)?
        .query_row(params_from_iter(args.iter()), |r| r.get(0))?)
}

/// Of `ids` (item ids), those the viewer may read: items that exist, in collections the viewer may read, in any
/// state.
pub fn readable_items(
    conn: &Connection,
    viewer: &Viewer,
    ids: &[String],
) -> Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let readable = readable_collections(viewer);
    let sql = format!(
        "SELECT i.id FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE i.id IN (SELECT value FROM json_each(?)) AND {}",
        readable.sql
    );
    let mut args = vec![Value::Text(serde_json::to_string(ids)?)];
    args.extend(readable.args);
    Ok(conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args.iter()), |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

/// Of the targets of a board's cards (`item_links` of kind `embed`, §18.8), those the viewer may read, and how many
/// the board links (at most `EMBED_LINKS_MAX`: a board at the cap may hold cards whose targets are not linked).
pub fn readable_embeds(
    conn: &Connection,
    viewer: &Viewer,
    board_id: &str,
) -> Result<(HashSet<String>, usize)> {
    let targets: Vec<String> = conn
        .prepare_cached(
            "SELECT target_id FROM item_links WHERE source_id = ?1 AND kind = 'embed' ORDER BY target_id",
        )?
        .query_map([board_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let linked = targets.len();
    Ok((readable_items(conn, viewer, &targets)?, linked))
}

const MEMBER_COLUMNS: &str =
    "u.id, u.name, u.role, u.disabled_at, m.access, m.updated_at, c.default_access";

fn member_from_row(r: &rusqlite::Row<'_>) -> Result<CollectionMember> {
    let id: String = r.get(0)?;
    let role: Role = parse::<Role>(&r.get::<_, String>(2)?)?;
    let disabled: Option<i64> = r.get(3)?;
    let access: String = r.get(4)?;
    let default: String = r.get(6)?;
    let effective_access = if disabled.is_some() {
        CollectionAccess::None
    } else {
        effective(role, &default, Some(&access))?
    };
    Ok(CollectionMember {
        user: DirectoryUser {
            id: parse_uuid(&id)?,
            name: r.get(1)?,
            role,
        },
        access: parse(&access)?,
        effective_access,
        disabled: disabled.is_some(),
        updated_at: Timestamp(r.get(5)?),
    })
}

/// `GET /collections/{id}/members`: the memberships of a collection (any state), by name.
pub fn members(conn: &Connection, collection_id: &str) -> Result<Vec<CollectionMember>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {MEMBER_COLUMNS} FROM collection_members m JOIN users u ON u.id = m.user_id
           JOIN collections c ON c.id = m.collection_id
         WHERE m.collection_id = ?1 ORDER BY u.name COLLATE NOCASE, u.id"
    ))?;
    let mut rows = stmt.query([collection_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(member_from_row(r)?);
    }
    Ok(out)
}

fn member(conn: &Connection, collection_id: &str, user_id: &str) -> Result<CollectionMember> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {MEMBER_COLUMNS} FROM collection_members m JOIN users u ON u.id = m.user_id
           JOIN collections c ON c.id = m.collection_id
         WHERE m.collection_id = ?1 AND m.user_id = ?2"
    ))?;
    let mut rows = stmt.query(params![collection_id, user_id])?;
    match rows.next()? {
        Some(r) => member_from_row(r),
        None => Err(Error::not_found("no such member")),
    }
}

/// `PUT /collections/{id}/members/{user_id}`: adds `user_id` to the collection with `access`, or changes their
/// access, and bumps their access epoch when it changes. `not_found` for a missing collection or user; 422
/// `validation` (`field: "user_id"`) for a disabled account. `by` is who adds them (`None` for the server itself).
pub fn set_member(
    tx: &Transaction<'_>,
    collection_id: &str,
    user_id: &str,
    access: MemberAccess,
    by: Option<&str>,
    now: i64,
) -> Result<CollectionMember> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM collections WHERE id = ?1)",
        [collection_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(Error::not_found("no such collection"));
    }
    let disabled: Option<Option<i64>> = tx
        .query_row(
            "SELECT disabled_at FROM users WHERE id = ?1",
            [user_id],
            |r| r.get(0),
        )
        .optional()?;
    match disabled {
        None => return Err(Error::not_found("no such user")),
        Some(Some(_)) => {
            return Err(Error::validation(
                "user_id",
                "the account is disabled; enable it first",
            ));
        }
        Some(None) => {}
    }
    let changed = tx.execute(
        "INSERT INTO collection_members (collection_id, user_id, access, created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT (collection_id, user_id) DO UPDATE SET access = excluded.access, updated_at = excluded.updated_at
         WHERE access IS NOT excluded.access",
        params![collection_id, user_id, access.as_str(), by, now],
    )?;
    if changed > 0 {
        bump_epoch(tx, user_id)?;
    }
    member(tx, collection_id, user_id)
}

/// `DELETE /collections/{id}/members/{user_id}`: removes the membership (none is not an error) and bumps the user's
/// access epoch when there was one. Returns whether there was one.
pub fn remove_member(tx: &Transaction<'_>, collection_id: &str, user_id: &str) -> Result<bool> {
    let removed = tx.execute(
        "DELETE FROM collection_members WHERE collection_id = ?1 AND user_id = ?2",
        params![collection_id, user_id],
    )? > 0;
    if removed {
        bump_epoch(tx, user_id)?;
    }
    Ok(removed)
}

/// `GET /directory`: every account that can sign in, by name: id, name and role, never the email (D152).
pub fn directory(conn: &Connection) -> Result<Vec<DirectoryUser>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, name, role FROM users WHERE disabled_at IS NULL ORDER BY name COLLATE NOCASE, id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        out.push(DirectoryUser {
            id: parse_uuid(&id)?,
            name: r.get(1)?,
            role: parse(&r.get::<_, String>(2)?)?,
        });
    }
    Ok(out)
}

/// The user's access epoch (D21: the `viewer_epoch` of the tree ETag); 0 for an unknown user.
pub fn epoch(conn: &Connection, user_id: &str) -> Result<i64> {
    Ok(conn
        .prepare_cached("SELECT access_epoch FROM users WHERE id = ?1")?
        .query_row([user_id], |r| r.get(0))
        .optional()?
        .unwrap_or(0))
}

/// Bumps the user's access epoch: their access to some collection may have changed (a membership, the role,
/// disabling or enabling the account).
pub fn bump_epoch(tx: &Transaction<'_>, user_id: &str) -> Result<()> {
    tx.execute(
        "UPDATE users SET access_epoch = access_epoch + 1 WHERE id = ?1",
        [user_id],
    )?;
    Ok(())
}
