//! `wikanvas user` and `wikanvas invite` against a running TestServer (D148)
//!
//! The owner's admin token makes an invite; the invite is accepted through `wikanvas api` (the route is public and
//! web-only); then the new account is listed, given another role, disabled and enabled by id or by email.

use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};
use wikanvas_api::Scope;

use crate::common::{Harness, last_error};

/// Accepts the invite link `url` through `wikanvas api` as `email`; returns the new account's id.
fn accept(h: &Harness, url: &str, email: &str) -> String {
    let token = url.rsplit('/').next().unwrap();
    let body = h.sandbox.file(
        "accept.json",
        json!({"name": "New Person", "email": email, "password": "a long enough password"})
            .to_string()
            .as_bytes(),
        0o600,
    );
    let (code, v, err) = h.json(
        Scope::Read,
        &[
            "api",
            "POST",
            &format!("/api/v1/invites/{token}/accept"),
            "--input",
            body.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{err}");
    v["user"]["id"].as_str().unwrap().to_owned()
}

#[test]
fn invites_are_created_listed_and_revoked() {
    let h = Harness::start();
    let (code, v, err) = h.json(
        Scope::Admin,
        &[
            "invite",
            "create",
            "--role",
            "member",
            "--expires",
            "3d",
            "--note",
            "For Bea",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let url = v["url"].as_str().unwrap().to_owned();
    assert!(url.contains("/invite/qinv_"), "{url}");
    assert_eq!(v["invite"]["role"], "member");
    assert_eq!(v["invite"]["note"], "For Bea");
    let id = v["invite"]["id"].as_str().unwrap().to_owned();
    // The list never has the link.
    let out = h
        .wikanvas_as(Scope::Admin)
        .args(["--json", "invite", "list"])
        .assert()
        .success();
    let listed = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(!listed.contains(url.rsplit('/').next().unwrap()));
    let list: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], id.as_str());
    // Without --json, only the link goes to stdout.
    let out = h
        .wikanvas_as(Scope::Admin)
        .args(["invite", "create", "--role", "viewer"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.trim().contains("/invite/qinv_") && stdout.lines().count() == 1,
        "{stdout}"
    );
    // --url-out writes a 0600 file and keeps the link off stdout.
    let path = h.sandbox.path().join("invite.url");
    let out = h
        .wikanvas_as(Scope::Admin)
        .args([
            "--json",
            "invite",
            "create",
            "--role",
            "viewer",
            "--url-out",
        ])
        .arg(&path)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let link = std::fs::read_to_string(&path).unwrap();
    assert!(link.trim_end().contains("/invite/qinv_"), "{link}");
    assert!(!stdout.contains(link.trim_end()), "{stdout}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["invite"]["role"], "viewer");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Revoke: ok, then 404 (exit 7).
    let (code, v, err) = h.json(Scope::Admin, &["invite", "revoke", &id]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        (v["ok"].clone(), v["status"].clone()),
        (json!(true), json!(204))
    );
    let (code, _, err) = h.json(Scope::Admin, &["invite", "revoke", &id]);
    assert_eq!(code, 7, "{err}");
    // Usage errors before any request; a write token is refused the scope; an admin invite needs the owner, whom
    // this token belongs to.
    for args in [
        &["invite", "create", "--role", "member", "--expires", "31d"][..],
        &["invite", "create", "--role", "owner"][..],
        &["invite", "revoke", "nope"][..],
    ] {
        let (code, _, err) = h.json(Scope::Admin, args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    let (code, _, err) = h.json(Scope::Write, &["invite", "list"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "insufficient_scope");
    let (code, _, err) = h.json(Scope::Admin, &["invite", "create", "--role", "admin"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn accounts_are_listed_and_changed_by_id_or_email() {
    let h = Harness::start();
    let (code, v, err) = h.json(Scope::Admin, &["invite", "create", "--role", "member"]);
    assert_eq!(code, 0, "{err}");
    let id = accept(&h, v["url"].as_str().unwrap(), "bea@wikanvas.invalid");
    let (code, list, err) = h.json(Scope::Admin, &["user", "list"]);
    assert_eq!(code, 0, "{err}");
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["role"], "owner");
    assert_eq!(list[1]["id"], id.as_str());
    // By email, ignoring case, and by id.
    let (code, v, err) = h.json(
        Scope::Admin,
        &[
            "user",
            "set-role",
            "BEA@wikanvas.invalid",
            "--role",
            "viewer",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["role"], "viewer");
    let (code, v, err) = h.json(Scope::Admin, &["user", "disable", &id]);
    assert_eq!(code, 0, "{err}");
    assert!(v["disabled_at"].is_string());
    let (code, v, err) = h.json(Scope::Admin, &["user", "enable", "bea@wikanvas.invalid"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["disabled_at"], Value::Null);
    // The owner's own account is refused (exit 6), an unknown email is exit 7, a bad id or role exit 2.
    let (code, _, err) = h.json(
        Scope::Admin,
        &[
            "user",
            "set-role",
            "owner@wikanvas.invalid",
            "--role",
            "admin",
        ],
    );
    assert_eq!(code, 6, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "forbidden");
    let (code, _, err) = h.json(
        Scope::Admin,
        &["user", "disable", "nobody@wikanvas.invalid"],
    );
    assert_eq!(code, 7, "{err}");
    for args in [
        &["user", "disable", "nope"][..],
        &["user", "set-role", &id, "--role", "owner"][..],
    ] {
        let (code, _, err) = h.json(Scope::Admin, args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    // The human list names the disabled state.
    let (code, _, err) = h.json(Scope::Admin, &["user", "disable", &id]);
    assert_eq!(code, 0, "{err}");
    let out = h
        .wikanvas_as(Scope::Admin)
        .args(["user", "list"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        text.contains("bea@wikanvas.invalid") && text.contains("(disabled)"),
        "{text}"
    );
}

/// D152: a collection's default access on create and update, its members by id, email or directory name, and the
/// directory.
#[test]
fn collection_access_and_members() {
    let h = Harness::start();
    let (code, v, err) = h.json(Scope::Admin, &["invite", "create", "--role", "member"]);
    assert_eq!(code, 0, "{err}");
    let bea = accept(&h, v["url"].as_str().unwrap(), "bea@wikanvas.invalid");
    let (code, c, err) = h.json(
        Scope::Write,
        &[
            "collection",
            "create",
            "--name",
            "Private",
            "--access",
            "none",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        (c["default_access"].clone(), c["my_access"].clone()),
        (json!("none"), json!("manage"))
    );
    let id = c["id"].as_str().unwrap().to_owned();
    let (code, c, err) = h.json(
        Scope::Write,
        &["collection", "update", &id, "--access", "read"],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(c["default_access"], "read");
    // The directory: everyone who can sign in, by name, without emails.
    let (code, people, err) = h.json(Scope::Read, &["user", "directory"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(people.as_array().unwrap().len(), 2);
    assert!(!people.to_string().contains('@'), "{people}");
    // A member by name, then by email, then by id; the list shows what it gives them.
    let (code, m, err) = h.json(
        Scope::Write,
        &[
            "collection",
            "member",
            "set",
            &id,
            "new person",
            "--access",
            "write",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        (m["user"]["id"].clone(), m["effective_access"].clone()),
        (json!(bea), json!("write"))
    );
    let (code, m, err) = h.json(
        Scope::Admin,
        &[
            "collection",
            "member",
            "set",
            &id,
            "BEA@wikanvas.invalid",
            "--access",
            "manage",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(m["access"], "manage");
    let (code, list, err) = h.json(Scope::Read, &["collection", "member", "list", &id]);
    assert_eq!(code, 0, "{err}");
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2, "the owner (its creator) and Bea: {list:?}");
    assert!(
        list.iter()
            .any(|m| m["user"]["id"] == bea.as_str() && m["access"] == "manage")
    );
    let out = h
        .wikanvas_as(Scope::Read)
        .args(["collection", "member", "list", &id])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        text.contains(&bea) && text.contains("manage") && text.contains("New Person"),
        "{text}"
    );
    let (code, v, err) = h.json(Scope::Write, &["collection", "member", "remove", &id, &bea]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["ok"], true);
    let (code, list, err) = h.json(Scope::Read, &["collection", "member", "list", &id]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(list.as_array().unwrap().len(), 1);
    // Usage errors before any request; an unknown name is exit 7; a read token cannot set (exit 6).
    for args in [
        &["collection", "member", "set", &id, &bea, "--access", "none"][..],
        &["collection", "create", "--name", "X", "--access", "manage"][..],
    ] {
        let (code, _, err) = h.json(Scope::Write, args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    let (code, _, err) = h.json(
        Scope::Write,
        &[
            "collection",
            "member",
            "set",
            &id,
            "Nobody Here",
            "--access",
            "read",
        ],
    );
    assert_eq!(code, 7, "{err}");
    let (code, _, err) = h.json(
        Scope::Read,
        &["collection", "member", "set", &id, &bea, "--access", "read"],
    );
    assert_eq!(code, 6, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "insufficient_scope");
}
