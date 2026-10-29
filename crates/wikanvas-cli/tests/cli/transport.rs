//! Transport of §7.6: retries, timeouts, redirects, plain HTTP and TLS roots, against the mock listener (owner: W8)

use std::net::TcpListener;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::common::{
    ID, Mock, PLACEHOLDER_CA, RawListener, Reply, Sandbox, fake_token, last_error, non_loopback_ip,
    run_json,
};

fn collection() -> Value {
    json!({"id": ID, "url_id": "AbCdEfGhIj", "name": "C", "description": "", "icon": null, "color": "#8D8D8D",
           "sort_key": "a0", "child_order": "manual", "item_count": 0, "updated_at": "2026-10-28T00:00:00.000Z"})
}

fn write_result() -> Value {
    json!({"seq": 1, "etag": "\"d1\"", "updated_at": "2026-10-28T00:00:00.000Z", "changed": true, "merged": false,
           "warnings": []})
}

/// `quire --json --retries N <args>` against `url`.
fn q(s: &Sandbox, url: &str, retries: u32, args: &[&str]) -> (i32, Value, String) {
    run_json(
        s.quire_at(url, &fake_token(1))
            .args(["--json", "--retries", &retries.to_string()])
            .args(args),
    )
}

#[test]
fn retries_only_unprocessed_requests() {
    let s = Sandbox::new();
    let md = s.file("a.md", b"# A\n", 0o600);
    let md = md.to_str().unwrap();
    let post: &[&str] = &["collection", "create", "--name", "C"];
    let put: &[&str] = &["doc", "put", ID, "--md", md, "--force"];
    let get: &[&str] = &["auth", "whoami"];

    // Answered 429 and 503 are retried for every method (the server did not process them).
    for (status, args, ok) in [
        (429, post, Reply::json(201, &collection())),
        (503, post, Reply::json(201, &collection())),
        (429, put, Reply::json(200, &write_result())),
        (503, put, Reply::json(200, &write_result())),
    ] {
        let mock = Mock::start(move |_, i| {
            if i == 0 {
                Reply::error(
                    status,
                    if status == 429 {
                        "rate_limited"
                    } else {
                        "unavailable"
                    },
                )
                .with("Retry-After", "0")
            } else {
                ok.clone()
            }
        });
        let (code, _, err) = q(&s, &mock.url, 2, args);
        assert_eq!(code, 0, "{status} {args:?}: {err}");
        assert_eq!(mock.requests().len(), 2, "{status} {args:?}");
    }
    // Retries are bounded: --retries 1 gives two attempts, then the status's exit code.
    let mock = Mock::always(Reply::error(503, "unavailable").with("Retry-After", "0"));
    let (code, _, err) = q(&s, &mock.url, 1, post);
    assert_eq!(code, 12, "{err}");
    assert_eq!(mock.requests().len(), 2);
    let mock = Mock::always(Reply::error(429, "rate_limited").with("Retry-After", "0"));
    let (code, _, _) = q(&s, &mock.url, 0, get);
    assert_eq!(code, 11);
    assert_eq!(mock.requests().len(), 1);

    // A connect failure is retried for a write too: the port refuses, then a listener appears (within the 1 s
    // back-off).
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("rebind the port");
        Mock::serve(listener, |_, _| Reply::json(201, &collection()))
    });
    let started = Instant::now();
    let (code, out, err) = q(&s, &url, 1, post);
    let mock = late.join().unwrap();
    assert_eq!(code, 0, "{err}");
    assert_eq!(out["id"], ID);
    assert_eq!(
        mock.requests().len(),
        1,
        "the first attempt never connected"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "after the 1 s back-off"
    );

    // A GET is retried after a timeout and after a lost response.
    for first in [Reply::Hang(Duration::from_secs(3)), Reply::Drop] {
        let mock = Mock::start(move |_, i| {
            if i == 0 {
                first.clone()
            } else {
                Reply::json(200, &json!({"user": {}, "auth": {}}))
            }
        });
        let (code, _, err) = run_json(s.quire_at(&mock.url, &fake_token(1)).args([
            "--json",
            "--retries",
            "1",
            "--timeout",
            "1",
            "auth",
            "whoami",
        ]));
        assert_eq!(code, 0, "{err}");
        assert_eq!(mock.requests().len(), 2);
    }
    // A POST and a PUT whose response is lost, or that time out, are never retried: exit 4, details.sent true.
    for (reply, code_name) in [
        (Reply::Drop, "network"),
        (Reply::Hang(Duration::from_secs(3)), "timeout"),
    ] {
        for args in [post, put] {
            let mock = Mock::always(reply.clone());
            let (code, out, err) = run_json(
                s.quire_at(&mock.url, &fake_token(1))
                    .args(["--json", "--retries", "2", "--timeout", "1"])
                    .args(args),
            );
            assert_eq!(code, 4, "{args:?}: {err}");
            assert_eq!(out, Value::Null);
            let e = last_error(&err);
            assert_eq!(e["error"]["code"], code_name, "{e}");
            assert_eq!(e["error"]["details"]["sent"], true, "{e}");
            assert_eq!(e["exit_code"], 4);
            assert_eq!(e["request_id"], Value::Null);
            assert_eq!(mock.requests().len(), 1, "{args:?} was not retried");
        }
    }

    // Retry-After is honoured (2 s, where the default back-off would be 1 s).
    let mock = Mock::start(|_, i| {
        if i == 0 {
            Reply::error(429, "rate_limited").with("Retry-After", "2")
        } else {
            Reply::json(201, &collection())
        }
    });
    let started = Instant::now();
    let (code, _, err) = q(&s, &mock.url, 1, post);
    assert_eq!(code, 0, "{err}");
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(2), "{waited:?}");
    assert!(waited < Duration::from_secs(10), "{waited:?}");
}

#[test]
fn connection_refused_exit_4_sent_false() {
    let s = Sandbox::new();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    for args in [
        &["collection", "create", "--name", "C"][..],
        &["auth", "whoami"][..],
    ] {
        let (code, out, err) = q(&s, &url, 0, args);
        assert_eq!(code, 4, "{err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], "network");
        assert_eq!(e["error"]["details"]["sent"], false);
        assert_eq!(e["status"], Value::Null);
    }
}

#[test]
fn timeout_exit_4() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::Hang(Duration::from_secs(4)));
    let started = Instant::now();
    let (code, _, err) = run_json(s.quire_at(&mock.url, &fake_token(1)).args([
        "--json",
        "--retries",
        "0",
        "--timeout",
        "1",
        "auth",
        "whoami",
    ]));
    assert_eq!(code, 4, "{err}");
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "timeout");
    assert_eq!(e["error"]["details"]["sent"], true);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the timeout cut it short"
    );
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn plain_http_refused_for_non_loopback() {
    let s = Sandbox::new();
    let me = json!({"user": {}, "auth": {}});
    // A plain-HTTP URL for a host that is not this machine's loopback: this machine's own LAN address when it has
    // one (a mock listens there), else a name that does not resolve.
    let lan = non_loopback_ip().map(|ip| {
        let me = me.clone();
        Mock::start_on(ip, move |_, _| Reply::json(200, &me))
    });
    let remote = lan.as_ref().map_or_else(
        || "http://quire-host.invalid.:8080".to_owned(),
        |m| m.url.clone(),
    );
    let reached = if lan.is_some() { 0 } else { 4 };
    // Refused before any lookup or connection: exit 3.
    let (code, _, err) = q(&s, &remote, 0, &["auth", "whoami"]);
    assert_eq!(code, 3, "{remote}: {err}");
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "cli_config");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("plain HTTP"),
        "{e}"
    );
    if let Some(m) = &lan {
        assert_eq!(m.connections(), 0, "refused before connecting");
    }
    // QUIRE_ALLOW_HTTP=1 lifts it: the request is sent (to the LAN mock, or into a failing DNS lookup).
    let (code, _, err) = run_json(
        s.quire_at(&remote, &fake_token(1))
            .env("QUIRE_ALLOW_HTTP", "1")
            .args([
                "--json",
                "--retries",
                "0",
                "--timeout",
                "10",
                "auth",
                "whoami",
            ]),
    );
    assert_eq!(code, reached, "{remote}: {err}");
    // So does the profile's allow_http = true.
    s.write_config(
        &format!(
            "default_profile = \"lan\"\n[profiles.lan]\nurl = \"{remote}\"\ntoken = \"{}\"\nallow_http = true\n",
            fake_token(2)
        ),
        0o600,
    );
    let (code, _, err) = run_json(s.quire().args([
        "--json",
        "--retries",
        "0",
        "--timeout",
        "10",
        "auth",
        "whoami",
    ]));
    assert_eq!(code, reached, "{remote}: {err}");
    if let Some(m) = &lan {
        assert_eq!(m.requests().len(), 2);
    }
    // Loopback is always allowed.
    let mock = Mock::always(Reply::json(200, &me));
    for url in [mock.url.clone(), format!("http://localhost:{}", mock.port)] {
        let (code, _, err) = q(&Sandbox::new(), &url, 0, &["auth", "whoami"]);
        assert_eq!(code, 0, "{url}: {err}");
    }
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn redirect_never_followed_exit_3() {
    let s = Sandbox::new();
    let target = Mock::always(Reply::json(200, &json!({})));
    let location = format!("{}/api/v1/auth/me", target.url);
    let loc = location.clone();
    let mock = Mock::always(Reply::empty(302).with("Location", &loc));
    for args in [
        &["auth", "whoami"][..],
        &["collection", "create", "--name", "C"][..],
    ] {
        let (code, out, err) = q(&s, &mock.url, 2, args);
        assert_eq!(code, 3, "{err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["status"], 302);
        assert_eq!(e["exit_code"], 3);
        assert_eq!(e["error"]["details"]["location"], location.as_str());
        assert!(
            e["error"]["message"].as_str().unwrap().contains(&location),
            "{e}"
        );
    }
    assert_eq!(
        mock.requests().len(),
        2,
        "one request per command, no retry"
    );
    assert_eq!(target.connections(), 0, "never followed");
    // Human form names it too.
    let out = s
        .quire_at(&mock.url, &fake_token(1))
        .args(["auth", "whoami"])
        .assert()
        .code(3)
        .stdout("");
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(stderr.contains(&location), "{stderr}");
}

#[test]
fn https_without_ca_bundle_exit_3() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &json!({})));
    let url = format!("https://127.0.0.1:{}", mock.port);
    let missing = s.path().join("no-such-bundle.pem");
    let (code, _, err) = run_json(
        s.quire_at(&url, &fake_token(1))
            .env("SSL_CERT_FILE", &missing)
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!(code, 3, "{err}");
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "cli_config");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("CA bundle"),
        "{e}"
    );
    // A --ca-file that is missing, or holds no certificate, is the same.
    let empty = s.file("empty.pem", b"not a certificate\n", 0o644);
    for ca in [missing.to_str().unwrap(), empty.to_str().unwrap()] {
        let (code, _, err) = run_json(s.quire_at(&url, &fake_token(1)).args([
            "--json",
            "--ca-file",
            ca,
            "auth",
            "whoami",
        ]));
        assert_eq!(code, 3, "{ca}: {err}");
    }
    assert_eq!(mock.connections(), 0, "nothing was sent");
}

/// A TLS handshake that stalls or fails happens before the request is written (ureq completes it in the connector):
/// even a POST is retried, and the error says `details.sent: false`. A TCP connect timeout takes the same path
/// (`http::tests::every_connect_stage_error_counts_as_not_sent`); no listener here can make the kernel drop SYNs.
#[test]
fn tls_handshake_failures_are_not_sent() {
    let s = Sandbox::new();
    let ca = s.file("ca.pem", PLACEHOLDER_CA, 0o644);
    let ca = ca.to_str().unwrap();
    // No answer to the ClientHello: a timeout, and with only the global timeout set ureq calls it `Global`.
    let silent = RawListener::start(b"");
    // An answer that is not TLS: a TLS error.
    let plain = RawListener::start(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    for (listener, code_name) in [(&silent, "timeout"), (&plain, "tls")] {
        let url = format!("https://127.0.0.1:{}", listener.port);
        let started = Instant::now();
        let (code, out, err) = run_json(
            s.quire_at(&url, &fake_token(1))
                .args([
                    "--json",
                    "--retries",
                    "1",
                    "--timeout",
                    "1",
                    "--ca-file",
                    ca,
                ])
                .args(["collection", "create", "--name", "C"]),
        );
        assert_eq!(code, 4, "{code_name}: {err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], code_name, "{e}");
        assert_eq!(e["error"]["details"]["sent"], false, "{e}");
        assert_eq!(
            listener.connections(),
            2,
            "{code_name}: the POST was retried once"
        );
        assert!(started.elapsed() < Duration::from_secs(20), "{code_name}");
    }
}

/// A GET whose body fails in transit (a stall past the timeout, a dropped connection) is retried, within the same
/// --retries budget; a write whose body fails is not, and says `details.sent: true`.
#[test]
fn get_retried_after_its_body_fails() {
    let s = Sandbox::new();
    let tree = json!({"collection": collection(), "items": []});
    for first in [Reply::BodyHangs(Duration::from_secs(4)), Reply::BodyDrops] {
        let (first2, tree2) = (first.clone(), tree.clone());
        let mock = Mock::start(move |_, i| {
            if i == 0 {
                first2.clone()
            } else {
                Reply::json(200, &tree2)
            }
        });
        let (code, out, err) = run_json(s.quire_at(&mock.url, &fake_token(1)).args([
            "--json",
            "--retries",
            "1",
            "--timeout",
            "1",
            "tree",
            ID,
        ]));
        assert_eq!(code, 0, "{first:?}: {err}");
        assert_eq!(out, tree);
        assert_eq!(mock.requests().len(), 2, "{first:?}");
        // With no retry left, the failure is exit 4.
        let mock = Mock::always(first.clone());
        let (code, _, err) = run_json(s.quire_at(&mock.url, &fake_token(1)).args([
            "--json",
            "--retries",
            "0",
            "--timeout",
            "1",
            "tree",
            ID,
        ]));
        assert_eq!(code, 4, "{first:?}: {err}");
        assert_eq!(mock.requests().len(), 1);
    }
    for (reply, code_name) in [
        (Reply::BodyHangs(Duration::from_secs(4)), "timeout"),
        (Reply::BodyDrops, "network"),
    ] {
        let mock = Mock::always(reply.clone());
        let (code, out, err) = run_json(
            s.quire_at(&mock.url, &fake_token(1))
                .args(["--json", "--retries", "2", "--timeout", "1"])
                .args(["collection", "create", "--name", "C"]),
        );
        assert_eq!(code, 4, "{reply:?}: {err}");
        assert_eq!(out, Value::Null);
        let e = last_error(&err);
        assert_eq!(e["error"]["code"], code_name, "{e}");
        assert_eq!(e["error"]["details"]["sent"], true, "{e}");
        assert_eq!(
            mock.requests().len(),
            1,
            "{reply:?}: the POST was not retried"
        );
    }
}
