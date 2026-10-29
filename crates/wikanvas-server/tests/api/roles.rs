//! The per-role matrix over the generated spec (§8 "Authorization seam", `authz::require`) (owner: W2)
//!
//! Every operation of `docs/api/openapi.json` is called with the session of the owner, an `admin`, a `member` and a
//! `viewer` (a session has every scope, so the role is the only limit). Each role gets its own app, with fixtures
//! the owner made: a call the role may make must succeed (2xx); a call it may not make must be 403 `forbidden` and
//! leave the database (but for the preview cache and last-seen columns) and the data directory exactly as they were.
//! The requests are real ones (existing ids, bodies that would change something), so a refusal that came too late
//! would show. [`OPS`] must name every operation of the spec, so a new route cannot skip its role check.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use axum::http::Method;
use axum_test::{TestResponse, TestServer};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wikanvas_api::tokens::NewToken;
use wikanvas_api::{DocState, ErrorCode, Role, Scope, YFIELD};
use wikanvas_core::convert::yrs;
use wikanvas_store::tokens::CreatedVia;
use yrs::updates::decoder::Decode;
use yrs::{Doc, Options, ReadTxn, Transact, Update, XmlElementPrelim, XmlFragment, XmlTextPrelim};

use crate::common::{OWNER_PASSWORD, TestApp, USER_PASSWORD, assert_error};

/// What an operation needs of the caller's role (`authz::require`'s action, or no check at all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    /// No credentials, no role check.
    Public,
    /// The caller's own account (`Resource::OwnAccount`): every role.
    Own,
    Read,
    Write,
    Admin,
}

impl Need {
    fn allows(self, role: Role) -> bool {
        match self {
            Need::Public | Need::Own | Need::Read => true,
            Need::Write => role != Role::Viewer,
            Need::Admin => matches!(role, Role::Owner | Role::Admin),
        }
    }
}

/// Every operation of the spec, in the order the matrix calls them: reads, the caller's own account, writes, then
/// the admin actions (the trash last).
const OPS: &[(&str, Need)] = &[
    ("get_config", Need::Public),
    ("get_openapi", Need::Public),
    ("get_setup", Need::Public),
    ("setup", Need::Public),
    ("login", Need::Public),
    ("get_workspace", Need::Read),
    ("get_doc_schema", Need::Read),
    ("get_sidebar", Need::Read),
    ("list_collections", Need::Read),
    ("get_collection", Need::Read),
    ("get_tree", Need::Read),
    ("list_collection_updated", Need::Read),
    ("get_item", Need::Read),
    ("get_item_text", Need::Read),
    ("get_children", Need::Read),
    ("get_doc", Need::Read),
    ("get_doc_markdown", Need::Read),
    ("get_doc_prosemirror", Need::Read),
    ("get_board", Need::Read),
    ("download_board", Need::Read),
    ("get_board_page", Need::Read),
    ("download_attachment", Need::Read),
    ("search", Need::Read),
    ("search_titles", Need::Read),
    ("list_archive", Need::Read),
    ("list_drafts", Need::Read),
    ("list_updated", Need::Read),
    ("get_trash", Need::Read),
    ("star_item", Need::Read),
    ("list_starred", Need::Read),
    ("unstar_item", Need::Read),
    ("view_item", Need::Read),
    ("list_recent", Need::Read),
    ("get_me", Need::Own),
    ("patch_me", Need::Own),
    ("list_sessions", Need::Own),
    ("revoke_session", Need::Own),
    ("revoke_other_sessions", Need::Own),
    ("change_password", Need::Own),
    ("logout", Need::Own),
    ("list_tokens", Need::Own),
    ("create_token", Need::Own),
    ("revoke_token", Need::Own),
    ("revoke_current_token", Need::Own),
    ("create_collection", Need::Write),
    ("patch_collection", Need::Write),
    ("move_collection", Need::Write),
    ("restore_collection", Need::Write),
    ("delete_collection", Need::Write),
    ("create_item", Need::Write),
    ("patch_item", Need::Write),
    ("move_item", Need::Write),
    ("duplicate_item", Need::Write),
    ("publish_item", Need::Write),
    ("unpublish_item", Need::Write),
    ("archive_item", Need::Write),
    ("unarchive_item", Need::Write),
    ("restore_item", Need::Write),
    ("delete_item", Need::Write),
    ("post_doc_updates", Need::Write),
    ("put_doc_markdown", Need::Write),
    ("put_doc_prosemirror", Need::Write),
    ("put_board", Need::Write),
    ("upload_attachment", Need::Write),
    ("patch_workspace", Need::Admin),
    ("backup", Need::Admin),
    ("export", Need::Admin),
    ("purge_trash_item", Need::Admin),
    ("purge_trash_collection", Need::Admin),
    ("empty_trash", Need::Admin),
];

const ROLES: [Role; 4] = [Role::Owner, Role::Admin, Role::Member, Role::Viewer];

/// Every operation of the spec: operation id → (method, path template).
fn spec_operations() -> BTreeMap<String, (Method, String)> {
    let spec: Value = serde_json::from_str(&wikanvas_server::openapi::spec_json()).unwrap();
    let mut out = BTreeMap::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let id = op["operationId"]
                .as_str()
                .expect("an operationId")
                .to_owned();
            let method: Method = method.to_uppercase().parse().unwrap();
            assert!(
                out.insert(id.clone(), (method, path.clone())).is_none(),
                "{id} twice"
            );
        }
    }
    out
}

#[test]
fn the_matrix_names_every_operation_once() {
    let listed: Vec<&str> = OPS.iter().map(|(op, _)| *op).collect();
    let unique: BTreeSet<&str> = listed.iter().copied().collect();
    assert_eq!(unique.len(), listed.len(), "an operation is listed twice");
    let spec: BTreeSet<String> = spec_operations().into_keys().collect();
    let listed: BTreeSet<String> = unique.into_iter().map(str::to_owned).collect();
    assert_eq!(
        listed, spec,
        "OPS must list every operation of the spec with what it needs of the role"
    );
}

/// The resources each role's calls act on, made by the owner.
struct Fixtures {
    collection: String,
    doomed_collection: String,
    restorable_collection: String,
    purgeable_collection: String,
    doc: String,
    board: String,
    attachment: String,
    draft: String,
    published: String,
    to_archive: String,
    archived: String,
    to_trash: String,
    trashed: String,
    purgeable: String,
}

fn ok_json(res: TestResponse, what: &str) -> Value {
    assert!(
        res.status_code().is_success(),
        "{what}: {} {}",
        res.status_code(),
        res.text()
    );
    res.json()
}

async fn new_collection(owner: &TestServer, name: &str, trashed: bool) -> String {
    let c = ok_json(
        owner
            .post("/api/v1/collections")
            .json(&json!({"name": name}))
            .await,
        "create a collection",
    );
    let id = c["id"].as_str().unwrap().to_owned();
    if trashed {
        ok_json(
            owner.delete(&format!("/api/v1/collections/{id}")).await,
            "trash a collection",
        );
    }
    id
}

/// A new item in `collection`; then `POST /items/{id}/<then>` (or `DELETE` for `then == "trash"`).
async fn new_item(
    owner: &TestServer,
    collection: &str,
    kind: &str,
    title: &str,
    then: Option<&str>,
) -> String {
    let item = ok_json(
        owner
            .post("/api/v1/items")
            .json(&json!({"collection_id": collection, "kind": kind, "title": title}))
            .await,
        "create an item",
    );
    let id = item["id"].as_str().unwrap().to_owned();
    match then {
        None => {}
        Some("trash") => {
            ok_json(owner.delete(&format!("/api/v1/items/{id}")).await, "trash");
        }
        Some(action) => {
            ok_json(
                owner.post(&format!("/api/v1/items/{id}/{action}")).await,
                action,
            );
        }
    }
    id
}

async fn fixtures(owner: &TestServer) -> Fixtures {
    let collection = new_collection(owner, "Matrix", false).await;
    let doc = new_item(owner, &collection, "doc", "Matrix doc", None).await;
    let attachment = ok_json(
        owner
            .post(&format!(
                "/api/v1/attachments?filename=matrix.txt&item_id={doc}"
            ))
            .bytes(b"matrix attachment".to_vec().into())
            .content_type("application/octet-stream")
            .await,
        "upload",
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let c = &collection;
    Fixtures {
        board: new_item(owner, c, "board", "Matrix board", None).await,
        draft: new_item(owner, c, "doc", "Matrix draft", Some("unpublish")).await,
        published: new_item(owner, c, "doc", "Matrix published", None).await,
        to_archive: new_item(owner, c, "doc", "Matrix to archive", None).await,
        archived: new_item(owner, c, "doc", "Matrix archived", Some("archive")).await,
        to_trash: new_item(owner, c, "doc", "Matrix to trash", None).await,
        trashed: new_item(owner, c, "doc", "Matrix trashed", Some("trash")).await,
        purgeable: new_item(owner, c, "doc", "Matrix purgeable", Some("trash")).await,
        doomed_collection: new_collection(owner, "Doomed", false).await,
        restorable_collection: new_collection(owner, "Restorable", true).await,
        purgeable_collection: new_collection(owner, "Purgeable", true).await,
        attachment,
        doc,
        collection,
    }
}

/// The user a role's calls run as.
struct Caller {
    role: Role,
    id: String,
    password: &'static str,
    client: TestServer,
}

/// How a call authenticates.
enum Cred {
    /// The caller's session.
    Session,
    /// A new session of the caller (for logout).
    NewSession,
    /// A new admin-scope token of the caller (for `DELETE /tokens/current`).
    NewToken,
}

enum Body {
    None,
    Json(Value),
    Raw(&'static str, Vec<u8>),
}

/// One call: what replaces `{id}` (`{n}` is page 0), the query, headers, body and credentials.
struct Call {
    id: Option<String>,
    query: String,
    headers: Vec<(&'static str, String)>,
    body: Body,
    cred: Cred,
}

impl Call {
    fn new() -> Call {
        Call {
            id: None,
            query: String::new(),
            headers: Vec::new(),
            body: Body::None,
            cred: Cred::Session,
        }
    }

    fn id(mut self, id: &str) -> Call {
        self.id = Some(id.to_owned());
        self
    }

    fn query(mut self, query: impl Into<String>) -> Call {
        self.query = query.into();
        self
    }

    fn header(mut self, name: &'static str, value: &str) -> Call {
        self.headers.push((name, value.to_owned()));
        self
    }

    fn json(mut self, body: Value) -> Call {
        self.body = Body::Json(body);
        self
    }

    fn raw(mut self, content_type: &'static str, body: impl Into<Vec<u8>>) -> Call {
        self.body = Body::Raw(content_type, body.into());
        self
    }

    fn cred(mut self, cred: Cred) -> Call {
        self.cred = cred;
        self
    }
}

/// A new token of `user`: (its id, its secret).
async fn new_token(app: &TestApp, user: &str) -> (String, String) {
    let user = user.to_owned();
    let now = wikanvas_server::state::now_ms();
    let new = NewToken {
        name: "matrix".into(),
        scopes: vec![Scope::Admin],
        expires_in_days: None,
        password: None,
    };
    let created = app
        .state
        .db
        .write(move |tx| {
            wikanvas_store::tokens::create(tx, &user, &new, CreatedVia::Admin, None, now)
        })
        .await
        .expect("create a token");
    (created.api_token.id.to_string(), created.token)
}

/// The id of the session whose cookie value is `cookie`.
async fn session_id(app: &TestApp, cookie: &str) -> String {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cookie)
        .unwrap();
    let hash: [u8; 32] = Sha256::digest(raw).into();
    app.state
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT id FROM sessions WHERE token_hash = ?1",
                [hash.as_slice()],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap()
}

/// A Yjs update that appends a paragraph to the document's current state (read by the owner).
async fn doc_update(owner: &TestServer, doc: &str) -> Vec<u8> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let res = owner.get(&format!("/api/v1/items/{doc}/doc")).await;
    res.assert_status_ok();
    let state: DocState = res.json();
    let ydoc = Doc::with_options(Options {
        offset_kind: yrs::OffsetKind::Utf16,
        ..Options::with_client_id(yrs::ClientID::new(4242))
    });
    let update = Update::decode_v1(&b64.decode(state.update).unwrap()).unwrap();
    ydoc.transact_mut().apply_update(update).unwrap();
    let frag = ydoc.get_or_insert_xml_fragment(YFIELD);
    let before = ydoc.transact().state_vector();
    {
        let mut txn = ydoc.transact_mut();
        let n = frag.len(&txn);
        let p = frag.insert(&mut txn, n, XmlElementPrelim::empty("paragraph"));
        p.insert(&mut txn, 0, XmlTextPrelim::new("Added by the matrix"));
    }
    ydoc.transact().encode_diff_v1(&before)
}

/// The call for `op`, against `fx`, as `caller`.
async fn call_for(op: &str, app: &TestApp, caller: &Caller, fx: &Fixtures) -> Call {
    let call = Call::new();
    match op {
        "get_workspace" | "get_doc_schema" | "get_sidebar" | "list_collections" | "list_archive"
        | "list_drafts" | "list_updated" | "get_trash" | "list_starred" | "list_recent"
        | "get_me" | "list_sessions" | "list_tokens" | "backup" | "export" | "empty_trash" => call,
        "get_collection" | "get_tree" | "list_collection_updated" => call.id(&fx.collection),
        "get_item" | "get_item_text" | "get_children" | "get_doc" | "get_doc_markdown"
        | "get_doc_prosemirror" | "star_item" | "unstar_item" | "view_item" | "duplicate_item" => {
            call.id(&fx.doc)
        }
        "get_board" | "download_board" | "get_board_page" => call.id(&fx.board),
        "download_attachment" => call.id(&fx.attachment),
        "search" | "search_titles" => call.query("q=matrix"),
        "patch_me" => call.json(json!({"name": "Renamed by the matrix"})),
        "revoke_session" => {
            let cookie = app.insert_session(&caller.id).await;
            call.id(&session_id(app, &cookie).await)
        }
        "revoke_other_sessions" => call.query("others=true"),
        "change_password" => call.json(json!({
            "current": caller.password,
            "new": "a new password from the matrix",
        })),
        "logout" => call.cred(Cred::NewSession),
        "create_token" => call.json(json!({"name": "matrix", "scopes": ["read"], "expires_in_days": 1})),
        "revoke_token" => call.id(&new_token(app, &caller.id).await.0),
        "revoke_current_token" => call.cred(Cred::NewToken),
        "create_collection" => call.json(json!({"name": "Made by the matrix"})),
        "patch_collection" => call
            .id(&fx.collection)
            .json(json!({"description": "Changed by the matrix"})),
        "move_collection" => call.id(&fx.collection).json(json!({"position": {}})),
        "restore_collection" => call.id(&fx.restorable_collection),
        "delete_collection" => call.id(&fx.doomed_collection),
        "create_item" => call.json(json!({
            "collection_id": fx.collection,
            "kind": "doc",
            "title": "Made by the matrix",
        })),
        "patch_item" => call.id(&fx.doc).json(json!({"title": "Changed by the matrix"})),
        "move_item" => call.id(&fx.doc).json(json!({"parent_id": fx.board})),
        "publish_item" => call.id(&fx.draft),
        "unpublish_item" => call.id(&fx.published),
        "archive_item" => call.id(&fx.to_archive),
        "unarchive_item" => call.id(&fx.archived),
        "restore_item" => call.id(&fx.trashed),
        "delete_item" => call.id(&fx.to_trash),
        "post_doc_updates" => {
            let update = doc_update(&app.server, &fx.doc).await;
            let update = base64::engine::general_purpose::STANDARD.encode(update);
            call.id(&fx.doc).json(json!({"update": update}))
        }
        "put_doc_markdown" => call
            .id(&fx.doc)
            .header("if-match", "*")
            .raw("text/markdown", "Written by the matrix.\n"),
        "put_doc_prosemirror" => call.id(&fx.doc).header("if-match", "*").json(json!({
            "schema_version": 1,
            "doc": {
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Written by the matrix"}]}],
            },
        })),
        "put_board" => call
            .id(&fx.board)
            .header("if-match", "*")
            .raw("application/json", crate::boards::board("x", "Written by the matrix")),
        "upload_attachment" => call
            .query(format!("filename=matrix.txt&item_id={}", fx.doc))
            .raw("application/octet-stream", "uploaded by the matrix"),
        "patch_workspace" => call.json(json!({"name": "Renamed by the matrix"})),
        "purge_trash_item" => call.id(&fx.purgeable),
        "purge_trash_collection" => call.id(&fx.purgeable_collection),
        other => panic!("no call for {other}: add it to call_for"),
    }
}

async fn send(
    app: &TestApp,
    caller: &Caller,
    method: &Method,
    template: &str,
    call: Call,
) -> TestResponse {
    let mut path = template.replace("{n}", "0");
    if let Some(id) = &call.id {
        path = path.replace("{id}", id);
    }
    assert!(!path.contains('{'), "{template}: no id for {path}");
    if !call.query.is_empty() {
        path = format!("{path}?{}", call.query);
    }
    let client = match call.cred {
        Cred::Session => None,
        Cred::NewSession => Some(app.cookie_client(&app.insert_session(&caller.id).await)),
        Cred::NewToken => Some(app.token_client(&new_token(app, &caller.id).await.1)),
    };
    let client = client.as_ref().unwrap_or(&caller.client);
    let mut req = client.method(method.clone(), &path);
    for (name, value) in call.headers {
        req = req.add_header(name.to_string(), value);
    }
    match call.body {
        Body::None => {}
        Body::Json(v) => req = req.json(&v),
        Body::Raw(content_type, bytes) => req = req.bytes(bytes.into()).content_type(content_type),
    }
    req.await
}

/// The columns that change on their own (last use, renewal) and the preview cache the worker fills in the background.
const VOLATILE: &[(&str, &str)] = &[
    ("sessions", "last_seen_at"),
    ("sessions", "expires_at"),
    ("sessions", "ip"),
    ("api_tokens", "last_used_at"),
    ("api_tokens", "last_used_ip"),
];

/// Every row of every table (but `board_previews` and [`VOLATILE`]) and every file under `files/` and `backups/`.
async fn fingerprint(app: &TestApp) -> String {
    let mut out = app
        .state
        .db
        .read(|conn| {
            let tables: Vec<String> = conn
                .prepare(
                    "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     AND name <> 'board_previews' ORDER BY name",
                )?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let mut out = String::new();
            for table in tables {
                let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\""))?;
                let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
                let _ = writeln!(out, "{table}");
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    for (i, column) in columns.iter().enumerate() {
                        if !VOLATILE.contains(&(table.as_str(), column.as_str())) {
                            let v: rusqlite::types::Value = row.get(i)?;
                            let _ = write!(out, "{column}={v:?} ");
                        }
                    }
                    out.push('\n');
                }
            }
            Ok(out)
        })
        .await
        .expect("read every table");
    fn walk(dir: &Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                let len = path.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = writeln!(out, "{} {len}", path.display());
            }
        }
    }
    walk(&app.state.config.files_dir(), &mut out);
    walk(&app.state.config.backups_dir(), &mut out);
    out
}

/// The app for one role: the owner and the fixtures, and the caller of `role` (the owner itself for `Owner`).
async fn app_for(role: Role) -> (TestApp, Caller, Fixtures) {
    let app = TestApp::with_owner().await;
    let fx = fixtures(&app.server).await;
    let caller = if role == Role::Owner {
        Caller {
            role,
            id: app.owner().to_owned(),
            password: OWNER_PASSWORD,
            client: app.cookie_client(app.session.as_deref().unwrap()),
        }
    } else {
        let id = app.insert_user(role).await;
        let session = app.insert_session(&id).await;
        Caller {
            role,
            client: app.cookie_client(&session),
            id,
            password: USER_PASSWORD,
        }
    };
    (app, caller, fx)
}

#[tokio::test]
async fn every_operation_checks_the_role() {
    let ops = spec_operations();
    let (mut allowed, mut refused) = (0, 0);
    for role in ROLES {
        let (app, caller, fx) = app_for(role).await;
        for (op, need) in OPS {
            if *need == Need::Public {
                continue;
            }
            let (method, template) = &ops[*op];
            let what = format!("{} {op} ({method} {template}, {need:?})", caller.role);
            let call = call_for(op, &app, &caller, &fx).await;
            if need.allows(role) {
                let res = send(&app, &caller, method, template, call).await;
                assert!(
                    res.status_code().is_success(),
                    "{what} is allowed: got {} {}",
                    res.status_code(),
                    res.text()
                );
                allowed += 1;
            } else {
                let before = fingerprint(&app).await;
                let res = send(&app, &caller, method, template, call).await;
                assert_eq!(res.status_code(), 403, "{what} is refused: {}", res.text());
                let body = assert_error(&res, 403, ErrorCode::Forbidden);
                assert!(
                    body.error.message.starts_with("not allowed: "),
                    "{what}: {}",
                    body.error.message
                );
                let after = fingerprint(&app).await;
                if before != after {
                    let (b, a): (BTreeSet<&str>, BTreeSet<&str>) =
                        (before.lines().collect(), after.lines().collect());
                    panic!(
                        "{what} changed something:\n- {:?}\n+ {:?}",
                        b.difference(&a).collect::<Vec<_>>(),
                        a.difference(&b).collect::<Vec<_>>()
                    );
                }
                refused += 1;
            }
        }
    }
    // 65 operations with credentials, for 4 roles: the viewer is refused the 20 writes and the 6 admin actions,
    // the member the 6 admin actions.
    assert_eq!((allowed, refused), (228, 32));
}

/// A token is limited by its user's role as well as by its scope: an admin-scope token of a viewer reads but
/// cannot write, one of a member cannot purge, and the scope check still comes first.
#[tokio::test]
async fn a_token_cannot_exceed_its_users_role() {
    let app = TestApp::with_owner().await;
    let fx = fixtures(&app.server).await;
    for (role, refused) in [
        (
            Role::Viewer,
            vec![
                (Method::POST, "/api/v1/collections"),
                (Method::DELETE, "/api/v1/trash"),
            ],
        ),
        (Role::Member, vec![(Method::DELETE, "/api/v1/trash")]),
    ] {
        let user = app.insert_user(role).await;
        let admin = app.token_client(&new_token(&app, &user).await.1);
        admin
            .get(&format!("/api/v1/collections/{}/tree", fx.collection))
            .await
            .assert_status_ok();
        for (method, path) in refused {
            let res = admin
                .method(method.clone(), path)
                .json(&json!({"name": "x"}))
                .await;
            assert_error(&res, 403, ErrorCode::Forbidden);
        }
        let read = app.token_client(&app.mint_token(&user, Scope::Read).await);
        let res = read.delete("/api/v1/trash").await;
        assert_error(&res, 403, ErrorCode::InsufficientScope);
    }
    // The trash was not emptied.
    let trash = ok_json(app.server.get("/api/v1/trash").await, "the trash");
    assert!(!trash["items"].as_array().unwrap().is_empty(), "{trash}");
}
