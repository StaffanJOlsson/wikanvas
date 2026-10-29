//! Configuration of §7.6: flags, environment and the profile file (owner: W8)

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use serde_json::{Value, json};

use crate::common::{Mock, Reply, Sandbox, fake_token, last_error, run_json};

/// `config show --json` with extra args and env; returns (exit, stdout JSON, stderr).
fn show(
    sandbox: &Sandbox,
    args: &[&str],
    env: &[(&str, &str)],
    stdin: Option<&str>,
) -> (i32, Value, String) {
    let mut cmd = sandbox.wikanvas();
    cmd.args(args).args(["config", "show", "--json"]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(s) = stdin {
        cmd.write_stdin(s);
    }
    run_json(&mut cmd)
}

fn me_body() -> Value {
    json!({"user": {"id": crate::common::ID, "email": "o@x.invalid", "name": "O", "role": "owner", "theme": "system"},
           "workspace": {"name": "W", "logo_url": null}, "must_change_password": false,
           "auth": {"method": "token", "token": null}})
}

#[test]
fn url_and_token_resolve_as_a_pair() {
    let s = Sandbox::new();
    let (a, b, c, d) = (fake_token(1), fake_token(2), fake_token(3), fake_token(4));
    let token_file = s.file("t.token", format!("{c}\n").as_bytes(), 0o600);
    let tf = token_file.to_str().unwrap();
    s.write_config(
        &format!(
            "default_profile = \"home\"\n\
             [profiles.home]\nurl = \"https://home.example\"\ntoken = \"{a}\"\n\
             [profiles.work]\nurl = \"https://work.example\"\ntoken_file = \"{tf}\"\n\
             [profiles.bare]\nurl = \"https://bare.example\"\n"
        ),
        0o600,
    );
    let row = |args: &[&str], env: &[(&str, &str)], stdin: Option<&str>| {
        let (code, v, err) = show(&s, args, env, stdin);
        assert_eq!(code, 0, "{args:?} {env:?}: {err}");
        (
            v["profile"].as_str().map(str::to_owned),
            v["url"].as_str().map(str::to_owned),
            v["url_source"].as_str().map(str::to_owned),
            v["token_source"].as_str().map(str::to_owned),
            v["token_prefix"].as_str().map(str::to_owned),
        )
    };
    let p = |t: &str| Some(t[..12].to_owned());
    let some = |x: &str| Some(x.to_owned());

    // The default profile: its URL and its token.
    assert_eq!(
        row(&[], &[], None),
        (
            some("home"),
            some("https://home.example"),
            some("profile"),
            some("profile"),
            p(&a)
        )
    );
    // WIKANVAS_PROFILE names another; its token_file is read.
    assert_eq!(
        row(&[], &[("WIKANVAS_PROFILE", "work")], None),
        (
            some("work"),
            some("https://work.example"),
            some("profile"),
            some("profile"),
            p(&c)
        )
    );
    // WIKANVAS_URL and WIKANVAS_TOKEN pair with each other.
    assert_eq!(
        row(
            &[],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN", &b)
            ],
            None
        ),
        (
            some("home"),
            some("https://env.example"),
            some("WIKANVAS_URL"),
            some("WIKANVAS_TOKEN"),
            p(&b)
        )
    );
    // --url beats WIKANVAS_URL; --token-file beats WIKANVAS_TOKEN; WIKANVAS_TOKEN beats WIKANVAS_TOKEN_FILE.
    assert_eq!(
        row(
            &["--url", "https://flag.example", "--token-file", tf],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN", &b)
            ],
            None
        ),
        (
            some("home"),
            some("https://flag.example"),
            some("--url"),
            some("--token-file"),
            p(&c)
        )
    );
    assert_eq!(
        row(
            &[],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN", &b),
                ("WIKANVAS_TOKEN_FILE", tf)
            ],
            None
        )
        .3,
        some("WIKANVAS_TOKEN")
    );
    assert_eq!(
        row(
            &[],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN_FILE", tf)
            ],
            None
        )
        .3,
        some("WIKANVAS_TOKEN_FILE")
    );
    // --token-stdin beats everything.
    assert_eq!(
        row(
            &["--token-stdin"],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN", &b)
            ],
            Some(&format!("{d}\n"))
        ),
        (
            some("home"),
            some("https://env.example"),
            some("WIKANVAS_URL"),
            some("--token-stdin"),
            p(&d)
        )
    );
    // A URL with the profile's origin keeps the profile's token.
    assert_eq!(
        row(&["--url", "https://HOME.example/"], &[], None).3,
        some("profile")
    );
    // An explicit --profile beats WIKANVAS_URL and WIKANVAS_TOKEN (and WIKANVAS_PROFILE).
    assert_eq!(
        row(
            &["--profile", "work"],
            &[
                ("WIKANVAS_URL", "https://env.example"),
                ("WIKANVAS_TOKEN", &b),
                ("WIKANVAS_PROFILE", "home")
            ],
            None
        ),
        (
            some("work"),
            some("https://work.example"),
            some("profile"),
            some("profile"),
            p(&c)
        )
    );
    // --profile with --url elsewhere and an explicit token.
    assert_eq!(
        row(
            &[
                "--profile",
                "work",
                "--url",
                "https://x.example",
                "--token-file",
                tf
            ],
            &[],
            None
        ),
        (
            some("work"),
            some("https://x.example"),
            some("--url"),
            some("--token-file"),
            p(&c)
        )
    );
    // A profile without a token and no other token: no token, no error.
    assert_eq!(
        row(&["--profile", "bare"], &[], None),
        (
            some("bare"),
            some("https://bare.example"),
            some("profile"),
            None,
            None
        )
    );
    // Inside the container: WIKANVAS_LISTEN, when nothing else names a URL.
    let empty = Sandbox::new();
    let (code, v, err) = show(
        &empty,
        &[],
        &[("WIKANVAS_LISTEN", "0.0.0.0:8080"), ("WIKANVAS_TOKEN", &b)],
        None,
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["url"], "http://127.0.0.1:8080");
    assert_eq!(v["url_source"], "WIKANVAS_LISTEN");
    let (_, v, _) = show(
        &empty,
        &[],
        &[
            ("WIKANVAS_LISTEN", "0.0.0.0:8080"),
            ("WIKANVAS_URL", "https://e.example"),
        ],
        None,
    );
    assert_eq!(v["url_source"], "WIKANVAS_URL");
    // Nothing configured.
    let (code, v, _) = show(&empty, &[], &[], None);
    assert_eq!(code, 0);
    assert_eq!(
        v,
        json!({"profile": null, "url": null, "url_source": null, "token_source": null, "token_prefix": null})
    );

    // Refusals, all exit 3 (cli_config).
    for (args, env) in [
        (&["--profile", "nope"][..], &[][..]),
        (&[][..], &[("WIKANVAS_PROFILE", "nope")][..]),
        (
            &["--profile", "work", "--url", "https://x.example"][..],
            &[][..],
        ),
        (&[][..], &[("WIKANVAS_URL", "https://env.example")][..]),
        (&["--url", "ftp://x"][..], &[][..]),
    ] {
        let (code, v, err) = show(&s, args, env, None);
        assert_eq!(code, 3, "{args:?} {env:?}: {err}");
        assert_eq!(v, Value::Null);
        assert_eq!(last_error(&err)["error"]["code"], "cli_config", "{err}");
    }

    // The pair is what goes on the wire: the profile's token to the profile's URL, the env token to the env URL.
    let mock = Mock::always(Reply::json(200, &me_body()));
    s.write_config(
        &format!(
            "default_profile = \"m\"\n[profiles.m]\nurl = \"{}\"\ntoken = \"{a}\"\n",
            mock.url
        ),
        0o600,
    );
    s.wikanvas()
        .args(["--json", "auth", "whoami"])
        .assert()
        .success();
    let other = Mock::always(Reply::json(200, &me_body()));
    s.wikanvas()
        .args(["--json", "auth", "whoami"])
        .env("WIKANVAS_URL", &other.url)
        .env("WIKANVAS_TOKEN", &b)
        .assert()
        .success();
    assert_eq!(
        mock.requests()[0].header("authorization"),
        Some(format!("Bearer {a}").as_str())
    );
    assert_eq!(
        other.requests()[0].header("authorization"),
        Some(format!("Bearer {b}").as_str())
    );
}

#[test]
fn profile_token_never_sent_to_overridden_url() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &me_body()));
    s.write_config(
        &format!(
            "default_profile = \"home\"\n[profiles.home]\nurl = \"https://home.example\"\ntoken = \"{}\"\n",
            fake_token(5)
        ),
        0o600,
    );
    for (args, env) in [
        (&[][..], &[("WIKANVAS_URL", mock.url.as_str())][..]),
        (&["--url", mock.url.as_str()][..], &[][..]),
        (
            &["--profile", "home", "--url", mock.url.as_str()][..],
            &[][..],
        ),
    ] {
        let mut cmd = s.wikanvas();
        cmd.args(args).args(["--json", "auth", "whoami"]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let (code, out, err) = run_json(&mut cmd);
        assert_eq!(code, 3, "{args:?}: {err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], "cli_config");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not bound"),
            "{e}"
        );
    }
    assert_eq!(
        mock.connections(),
        0,
        "no request, so no Authorization ever left"
    );
}

#[test]
fn group_readable_profile_with_token_exit_3() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &me_body()));
    let text = format!(
        "default_profile = \"m\"\n[profiles.m]\nurl = \"{}\"\ntoken = \"{}\"\n",
        mock.url,
        fake_token(6)
    );
    for mode in [0o640, 0o604, 0o644] {
        s.write_config(&text, mode);
        let (code, _, err) = run_json(s.wikanvas().args(["--json", "auth", "whoami"]));
        assert_eq!(code, 3, "{mode:o}: {err}");
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], "cli_config");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("readable by group or others"),
            "{e}"
        );
    }
    assert_eq!(mock.connections(), 0);
    // Without a token in it, a readable profile file is fine.
    s.write_config(&format!("[profiles.m]\nurl = \"{}\"\n", mock.url), 0o644);
    let (code, _, err) =
        run_json(
            s.wikanvas()
                .args(["--json", "--profile", "m", "config", "show"]),
        );
    assert_eq!(code, 0, "{err}");
    // And 0600 with the token works.
    s.write_config(&text, 0o600);
    s.wikanvas()
        .args(["--json", "auth", "whoami"])
        .assert()
        .success();
}

#[test]
fn group_readable_token_file_warns() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &me_body()));
    let token = fake_token(7);
    let f = s.file("readable.token", token.as_bytes(), 0o644);
    let (code, out, err) = run_json(s.wikanvas().args([
        "--json",
        "--url",
        &mock.url,
        "--token-file",
        f.to_str().unwrap(),
        "auth",
        "whoami",
    ]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(out["user"]["name"], "O");
    let lines: Vec<Value> = err
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON Lines"))
        .collect();
    assert_eq!(lines.len(), 1, "{err}");
    assert_eq!(lines[0]["warning"]["code"], "token_file_permissions");
    assert_eq!(
        mock.requests()[0].header("authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    // The same through WIKANVAS_TOKEN_FILE; a 0600 file does not warn.
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (code, _, err) = run_json(
        s.wikanvas()
            .env("WIKANVAS_URL", &mock.url)
            .env("WIKANVAS_TOKEN_FILE", &f)
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!((code, err.as_str()), (0, ""));
}

#[test]
fn malformed_token_exit_3_without_request() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &me_body()));
    let good = fake_token(8);
    let mut bad_sum = good.clone().into_bytes();
    bad_sum[50] = if bad_sum[50] == b'A' { b'B' } else { b'A' };
    let bad_sum = String::from_utf8(bad_sum).unwrap();
    for (token, reason) in [
        ("qpak_".to_owned() + &good[5..], "prefix"),
        (good[..50].to_owned(), "length"),
        (format!("{}-{}", &good[..20], &good[21..]), "alphabet"),
        (bad_sum, "checksum"),
    ] {
        let (code, out, err) = run_json(
            s.wikanvas_at(&mock.url, &token)
                .args(["--json", "auth", "whoami"]),
        );
        assert_eq!(code, 3, "{reason}: {err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], "cli_config");
        assert_eq!(e["error"]["details"]["reason"], reason);
        assert!(!err.contains(&token), "the token is never printed: {err}");
    }
    assert_eq!(mock.connections(), 0, "no connection at all");
}

#[test]
fn login_writes_profile_0600_in_0700_dir_atomically() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &me_body()));
    let token = fake_token(9);
    let (code, out, err) = run_json(
        s.wikanvas()
            .args([
                "--json",
                "auth",
                "login",
                "--url",
                &mock.url,
                "--token-stdin",
            ])
            .write_stdin(format!("{token}\n")),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, me_body(), "prints Me");
    assert_eq!(mock.requests()[0].path(), "/api/v1/auth/me");
    let path = s.config_path();
    let dir = path.parent().unwrap();
    assert_eq!(
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: toml::Table = text.parse().unwrap();
    assert_eq!(parsed["default_profile"].as_str(), Some("default"));
    assert_eq!(
        parsed["profiles"]["default"]["url"].as_str(),
        Some(mock.url.as_str())
    );
    assert_eq!(
        parsed["profiles"]["default"]["token"].as_str(),
        Some(token.as_str())
    );

    // A second login into another profile replaces the file by rename (a new inode) and keeps the first profile.
    let inode = std::fs::metadata(&path).unwrap().ino();
    let tf = s.file("second.token", fake_token(10).as_bytes(), 0o600);
    let (code, _, err) = run_json(s.wikanvas().args([
        "--json",
        "auth",
        "login",
        "--url",
        &mock.url,
        "--token-file",
        tf.to_str().unwrap(),
        "--profile",
        "two",
    ]));
    assert_eq!(code, 0, "{err}");
    assert_ne!(
        std::fs::metadata(&path).unwrap().ino(),
        inode,
        "written as a new file and renamed"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let parsed: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(parsed["default_profile"].as_str(), Some("default"));
    assert_eq!(
        parsed["profiles"]["default"]["token"].as_str(),
        Some(token.as_str())
    );
    assert_eq!(
        parsed["profiles"]["two"]["token_file"].as_str(),
        tf.to_str()
    );
    assert!(parsed["profiles"]["two"].get("token").is_none());
    let names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["config.toml"], "no temporary file is left");

    // A failed login (401) writes nothing.
    let denied = Mock::always(Reply::error(401, "invalid_token"));
    let before = std::fs::read(&path).unwrap();
    let (code, _, _) = run_json(
        s.wikanvas()
            .args([
                "--json",
                "auth",
                "login",
                "--url",
                &denied.url,
                "--token-stdin",
                "--profile",
                "three",
            ])
            .write_stdin(fake_token(11)),
    );
    assert_eq!(code, 5);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // Login needs --url and a token flag: usage errors.
    for args in [
        &["auth", "login", "--token-stdin"][..],
        &["auth", "login", "--url", "http://127.0.0.1:9"][..],
    ] {
        let (code, _, err) = run_json(s.wikanvas().arg("--json").args(args));
        assert_eq!(code, 2, "{args:?}: {err}");
    }
}

#[test]
fn token_stdin_with_token_file_exit_2() {
    let sandbox = Sandbox::new();
    for args in [
        &["--token-stdin", "--token-file", "t", "exit-codes"][..],
        &["exit-codes", "--token-file", "t", "--token-stdin"][..],
    ] {
        sandbox.wikanvas().args(args).assert().code(2).stdout("");
    }
    // --token-stdin and a `-` content argument cannot share stdin: every command that reads `-` refuses before any
    // request.
    let mock = Mock::always(Reply::json(200, &json!({})));
    let id = crate::common::ID;
    for args in [
        &["doc", "put", id, "--md", "-", "--force"][..],
        &["doc", "put", id, "--json", "-", "--force"][..],
        &["board", "put", id, "-", "--force"][..],
        &[
            "item",
            "create",
            "--collection",
            id,
            "--kind",
            "doc",
            "--md",
            "-",
        ][..],
        &["attachment", "upload", "-"][..],
        &["api", "POST", "/api/v1/collections", "--input", "-"][..],
    ] {
        let (code, out, err) = run_json(
            sandbox
                .wikanvas()
                .args(["--json", "--url", &mock.url, "--token-stdin"])
                .args(args)
                .write_stdin("x"),
        );
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], "cli_usage", "{args:?}");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--token-stdin"),
            "{e}"
        );
    }
    assert_eq!(mock.connections(), 0);
}
