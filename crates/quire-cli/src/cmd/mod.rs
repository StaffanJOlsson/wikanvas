//! One module per command group of ARCHITECTURE §7.6, the shared command context, and the coverage table
//! (owner: W8)

mod admin;
mod api;
mod attachment;
mod auth;
mod board;
mod collection;
mod doc;
mod export;
mod item;
mod list;
mod profile;
mod search;
mod star;
mod token;
mod trash;
mod tree;
mod version;
mod workspace;

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use quire_api::cli::ExitCode;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::args::{Cli, Command, GlobalArgs, ListArgs, PositionArgs};
use crate::config::{self, Env, Resolved};
use crate::error::Failure;
use crate::http::{self, Client, Req};
use crate::refs::{self, Ref};
use crate::{human, output};

/// What a command returns: success, or the failure to print.
pub type Res = Result<(), Failure>;

/// `--all` stops after this many items (§7.6).
pub const ALL_CAP: usize = 10_000;

/// Every API operation a command calls: (method, path as the spec names it, command). `coverage::` checks that every
/// operation of `docs/api/openapi.json` is here or in [`API_ONLY`].
pub const OPERATIONS: &[(&str, &str, &str)] = &[
    ("GET", "/api/v1/auth/me", "auth login"),
    ("GET", "/api/v1/auth/me", "auth whoami"),
    ("DELETE", "/api/v1/tokens/current", "auth logout"),
    ("PATCH", "/api/v1/auth/me", "me update"),
    ("GET", "/api/v1/auth/sessions", "session list"),
    ("DELETE", "/api/v1/auth/sessions/{id}", "session revoke"),
    ("DELETE", "/api/v1/auth/sessions", "session revoke"),
    ("GET", "/api/v1/tokens", "token list"),
    ("POST", "/api/v1/tokens", "token create"),
    ("DELETE", "/api/v1/tokens/{id}", "token revoke"),
    ("GET", "/api/v1/workspace", "workspace get"),
    ("PATCH", "/api/v1/workspace", "workspace set"),
    ("GET", "/api/v1/sidebar", "sidebar"),
    ("GET", "/api/v1/collections", "collection list"),
    ("GET", "/api/v1/collections/{id}", "collection get"),
    ("POST", "/api/v1/collections", "collection create"),
    ("PATCH", "/api/v1/collections/{id}", "collection update"),
    ("POST", "/api/v1/collections/{id}/move", "collection move"),
    ("DELETE", "/api/v1/collections/{id}", "collection delete"),
    (
        "POST",
        "/api/v1/collections/{id}/restore",
        "collection restore",
    ),
    (
        "GET",
        "/api/v1/collections/{id}/updated",
        "collection updated",
    ),
    ("GET", "/api/v1/collections/{id}/tree", "tree"),
    ("GET", "/api/v1/items/{id}", "item get"),
    ("POST", "/api/v1/items", "item create"),
    ("PATCH", "/api/v1/items/{id}", "item update"),
    ("POST", "/api/v1/items/{id}/move", "item move"),
    ("POST", "/api/v1/items/{id}/duplicate", "item duplicate"),
    ("POST", "/api/v1/items/{id}/publish", "item publish"),
    ("POST", "/api/v1/items/{id}/unpublish", "item unpublish"),
    ("POST", "/api/v1/items/{id}/archive", "item archive"),
    ("POST", "/api/v1/items/{id}/unarchive", "item unarchive"),
    ("DELETE", "/api/v1/items/{id}", "item trash"),
    ("POST", "/api/v1/items/{id}/restore", "item restore"),
    ("GET", "/api/v1/items/{id}/children", "item children"),
    ("GET", "/api/v1/items/{id}/text", "item text"),
    ("GET", "/api/v1/recent", "list recent"),
    ("GET", "/api/v1/updated", "list updated"),
    ("GET", "/api/v1/drafts", "list drafts"),
    ("GET", "/api/v1/starred", "list starred"),
    ("GET", "/api/v1/archive", "list archive"),
    ("GET", "/api/v1/trash", "trash list"),
    ("DELETE", "/api/v1/trash/items/{id}", "trash purge"),
    ("DELETE", "/api/v1/trash/collections/{id}", "trash purge"),
    ("DELETE", "/api/v1/trash", "trash purge"),
    ("GET", "/api/v1/search", "search"),
    ("GET", "/api/v1/search/titles", "search"),
    ("PUT", "/api/v1/items/{id}/star", "star add"),
    ("DELETE", "/api/v1/items/{id}/star", "star remove"),
    ("GET", "/api/v1/items/{id}/doc/markdown", "doc get"),
    ("GET", "/api/v1/items/{id}/doc/prosemirror", "doc get"),
    ("PUT", "/api/v1/items/{id}/doc/markdown", "doc put"),
    ("PUT", "/api/v1/items/{id}/doc/prosemirror", "doc put"),
    ("GET", "/api/v1/schema/doc", "doc schema"),
    ("GET", "/api/v1/items/{id}/board", "board get"),
    ("PUT", "/api/v1/items/{id}/board", "board put"),
    ("GET", "/api/v1/items/{id}/board/pages/{n}", "board preview"),
    ("POST", "/api/v1/attachments", "attachment upload"),
    ("GET", "/api/v1/attachments/{id}", "attachment get"),
    ("GET", "/api/v1/export", "export"),
    ("POST", "/api/v1/admin/backup", "backup"),
    ("GET", "/api/v1/config", "version"),
];

/// The web-only operations of §7.6 "Coverage", reachable through `quire api`; plus the spec document itself and the
/// board's browser download, which `quire api` and `quire board get` cover (D-W8 in docs/handoff/W8.md).
pub const API_ONLY: &[(&str, &str)] = &[
    ("GET", "/api/v1/setup"),
    ("POST", "/api/v1/setup"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/logout"),
    ("POST", "/api/v1/auth/password"),
    ("POST", "/api/v1/items/{id}/view"),
    ("GET", "/api/v1/items/{id}/doc"),
    ("POST", "/api/v1/items/{id}/doc/updates"),
    ("GET", "/api/v1/openapi.json"),
    ("GET", "/api/v1/items/{id}/board.inkboard"),
];

/// The state of one command: the global flags, the environment, and the client once configured.
pub struct Ctx {
    pub g: GlobalArgs,
    pub env: Env,
    resolved: RefCell<Option<Resolved>>,
    client: RefCell<Option<Rc<Client>>>,
}

impl Ctx {
    pub fn new(g: GlobalArgs, env: Env) -> Ctx {
        Ctx {
            g,
            env,
            resolved: RefCell::new(None),
            client: RefCell::new(None),
        }
    }

    /// The URL and token (resolved once; `--token-stdin` reads stdin then).
    pub fn resolved(&self) -> Result<Resolved, Failure> {
        if let Some(r) = self.resolved.borrow().as_ref() {
            return Ok(r.clone());
        }
        let r = config::resolve(&self.g, &self.env, read_stdin_text)?;
        *self.resolved.borrow_mut() = Some(r.clone());
        Ok(r)
    }

    /// The client (built once).
    pub fn client(&self) -> Result<Rc<Client>, Failure> {
        if let Some(c) = self.client.borrow().as_ref() {
            return Ok(Rc::clone(c));
        }
        let resolved = self.resolved()?;
        let c = Rc::new(Client::new(
            &resolved,
            &self.env,
            self.g.retries,
            self.g.timeout,
        )?);
        *self.client.borrow_mut() = Some(Rc::clone(&c));
        Ok(c)
    }

    /// Reads a content argument: a file, or stdin for `-` (which `--token-stdin` cannot share).
    pub fn read_input(&self, spec: &str, flag: &str) -> Result<Vec<u8>, Failure> {
        if spec == "-" {
            if self.g.token_stdin {
                return Err(Failure::usage(format!(
                    "{flag} - and --token-stdin both read stdin: pass the token with --token-file"
                )));
            }
            let mut bytes = Vec::new();
            std::io::stdin()
                .lock()
                .read_to_end(&mut bytes)
                .map_err(|e| Failure::usage(format!("cannot read stdin: {e}")))?;
            Ok(bytes)
        } else {
            std::fs::read(spec).map_err(|e| Failure::usage(format!("cannot read {spec}: {e}")))
        }
    }

    /// The UUID of an item reference (a `url_id` costs one `GET /items/{url_id}`).
    pub fn item_id(&self, input: &str) -> Result<String, Failure> {
        match refs::parse(input, "item")? {
            Ref::Uuid(id) => Ok(id),
            Ref::UrlId(u) => self.lookup_id(&format!("/api/v1/items/{u}"), "/item/id"),
        }
    }

    /// The UUID of a collection reference (a `url_id` costs one `GET /collections/{url_id}`).
    pub fn collection_id(&self, input: &str) -> Result<String, Failure> {
        match refs::parse(input, "collection")? {
            Ref::Uuid(id) => Ok(id),
            Ref::UrlId(u) => self.lookup_id(&format!("/api/v1/collections/{u}"), "/id"),
        }
    }

    fn lookup_id(&self, path: &str, pointer: &str) -> Result<String, Failure> {
        let v = self.get_value(Req::get(path))?;
        v.pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                Failure::bad_response(format!("GET {path}: no {pointer} in the response"))
            })
    }

    /// `{after_id}` / `{before_id}` of a position, or `None` (last).
    pub fn position(&self, p: &PositionArgs, collections: bool) -> Result<Option<Value>, Failure> {
        let id = |r: &str| {
            if collections {
                self.collection_id(r)
            } else {
                self.item_id(r)
            }
        };
        Ok(match (&p.after, &p.before) {
            (Some(a), _) => Some(json!({ "after_id": id(a)? })),
            (None, Some(b)) => Some(json!({ "before_id": id(b)? })),
            (None, None) => None,
        })
    }

    /// Sends a request and parses the JSON body.
    pub fn get_value(&self, req: Req) -> Result<Value, Failure> {
        let resp = self.client()?.send(&req)?;
        let text = http::read_text(resp)?;
        serde_json::from_str(&text).map_err(|e| {
            Failure::bad_response(format!(
                "{} {}: the response is not JSON: {e}",
                req.method, req.path
            ))
        })
    }

    /// Sends a request and prints the result: the body unchanged (`--json`) or its human form; a body-less 2xx
    /// prints the ok object, with `id` when given.
    pub fn call(&self, req: Req, id: Option<&str>) -> Res {
        let resp = self.client()?.send(&req)?;
        print_response(resp, id)
    }

    /// A paginated list: one page, or with `--all` every page merged into one (at most 10,000 items, then the real
    /// `next_cursor` and a `truncated` warning).
    pub fn list(&self, req: Req, l: &ListArgs) -> Res {
        if !l.all {
            let req = req
                .query_opt("cursor", l.cursor.clone())
                .query_opt("limit", l.limit.map(|n| n.to_string()));
            return self.call(req, None);
        }
        let client = self.client()?;
        let page_size = l.limit.unwrap_or(quire_api::page::MAX_LIMIT) as usize;
        let mut items: Vec<Box<RawValue>> = Vec::new();
        let mut cursor = l.cursor.clone();
        let mut next: Option<String>;
        #[derive(Deserialize)]
        struct PageRaw {
            items: Vec<Box<RawValue>>,
            next_cursor: Option<String>,
        }
        loop {
            let want = page_size.min(ALL_CAP.saturating_sub(items.len())).max(1);
            let r = req
                .clone()
                .query_opt("cursor", cursor.clone())
                .query("limit", want.to_string());
            let text = http::read_text(client.send(&r)?)?;
            let page: PageRaw = serde_json::from_str(&text)
                .map_err(|e| Failure::bad_response(format!("GET {}: not a page: {e}", req.path)))?;
            let empty = page.items.is_empty();
            items.extend(page.items);
            next = page.next_cursor;
            match &next {
                None => break,
                Some(_) if items.len() >= ALL_CAP || empty => {
                    output::warn(
                        "truncated",
                        &format!(
                            "--all stopped after {} items; continue with --cursor <next_cursor>",
                            items.len()
                        ),
                        Some(json!({ "count": items.len(), "next_cursor": next })),
                    );
                    break;
                }
                Some(c) => cursor = Some(c.clone()),
            }
        }
        if output::json() {
            output::print_json(&json!({ "items": items, "next_cursor": next }))
        } else {
            let v = json!({ "items": items, "next_cursor": next });
            output::print_text(&human::render(&v))
        }
    }
}

fn read_stdin_text() -> std::io::Result<String> {
    let mut s = String::new();
    std::io::stdin()
        .lock()
        .take(64 * 1024)
        .read_to_string(&mut s)?;
    Ok(s)
}

/// Prints a 2xx response (see [`Ctx::call`]).
pub fn print_response(resp: ureq::http::Response<ureq::Body>, id: Option<&str>) -> Res {
    let status = resp.status().as_u16();
    let rid = http::header(resp.headers(), quire_api::common::header::REQUEST_ID);
    let text = if status == 204 {
        String::new()
    } else {
        http::read_text(resp)?
    };
    if text.trim().is_empty() {
        return output::print_ok(Some(status), rid.as_deref(), id);
    }
    print_json_text(&text)
}

/// Prints a JSON body: unchanged with `--json`, else in human form. A body that is not JSON is exit 1.
pub fn print_json_text(text: &str) -> Res {
    if output::json() {
        let _: &RawValue = serde_json::from_str(text)
            .map_err(|e| Failure::bad_response(format!("the response is not JSON: {e}")))?;
        output::print_body(text)
    } else {
        let v: Value = serde_json::from_str(text)
            .map_err(|e| Failure::bad_response(format!("the response is not JSON: {e}")))?;
        output::print_text(&human::render(&v))
    }
}

/// Normalizes an ETag argument: `d42`, `"d42"` and `W/"d42"` become `"d42"`; `*` stays.
pub fn if_match(input: &str) -> Result<String, Failure> {
    let s = input.trim();
    let s = s.strip_prefix("W/").unwrap_or(s);
    if s == "*" {
        return Ok("*".into());
    }
    let inner = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s);
    if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_graphic() && b != b'"') {
        return Err(Failure::usage(format!(
            "--if-match {input:?} is not an ETag such as d42, \"d42\" or W/\"d42\""
        )));
    }
    Ok(format!("\"{inner}\""))
}

/// The `If-Match` of a content write: `--if-match` normalized, or `*` for `--force`.
pub fn precondition(p: &crate::args::Precondition) -> Result<String, Failure> {
    match (&p.if_match, p.force) {
        (Some(e), _) => if_match(e),
        (None, true) => Ok("*".into()),
        (None, false) => Err(Failure::usage("pass --if-match ETAG or --force")),
    }
}

/// Runs a parsed command line and returns its exit code.
pub fn run(cli: Cli) -> ExitCode {
    let admin_json =
        matches!(&cli.command, Command::Admin(a) if a.args.iter().any(|x| x == "--json"));
    output::init(cli.global.json || admin_json, cli.global.debug);
    let ctx = Ctx::new(cli.global, Env::from_process());
    let result = dispatch(&ctx, cli.command);
    match result {
        Ok(()) => ExitCode::Success,
        Err(f) => output::fail(&f),
    }
}

fn dispatch(ctx: &Ctx, command: Command) -> Res {
    match command {
        Command::Auth(c) => auth::run(ctx, c),
        Command::Me(c) => auth::me(ctx, c),
        Command::Session(c) => auth::session(ctx, c),
        Command::Token(c) => token::run(ctx, c),
        Command::Config(c) => profile::run(ctx, c),
        Command::Workspace(c) => workspace::run(ctx, c),
        Command::Sidebar => workspace::sidebar(ctx),
        Command::Collection(c) => collection::run(ctx, c),
        Command::Item(c) => item::run(ctx, c),
        Command::Tree(a) => tree::run(ctx, a),
        Command::List(c) => list::run(ctx, c),
        Command::Trash(c) => trash::run(ctx, c),
        Command::Search(a) => search::run(ctx, a),
        Command::Star(c) => star::run(ctx, c),
        Command::Doc(c) => doc::run(ctx, c),
        Command::Board(c) => board::run(ctx, c),
        Command::Attachment(c) => attachment::run(ctx, c),
        Command::Export(a) => export::export(ctx, a),
        Command::Backup => export::backup(ctx),
        Command::Api(a) => api::run(ctx, a),
        Command::Admin(a) => admin::run(ctx, a),
        Command::Version => version::version(ctx),
        Command::ExitCodes => version::exit_codes(),
        Command::Help(a) => version::help(a),
    }
}

/// Writes a download: to `out` when given (stdout then gets the metadata with `--json`), else raw to stdout (exit 2
/// with `--json`, which keeps stdout for one JSON document).
pub fn download(resp: ureq::http::Response<ureq::Body>, out: Option<&std::path::Path>) -> Res {
    let content_type = http::header(resp.headers(), "content-type");
    match out {
        Some(path) => {
            let mut file = std::fs::File::create(path)
                .map_err(|e| Failure::io(&format!("creating {}", path.display()), &e))?;
            let written = http::stream_to(resp, &mut file, &format!("writing {}", path.display()))?;
            if output::json() {
                output::print_json(&json!({
                    "path": path, "bytes": written.bytes, "sha256": written.sha256, "content_type": content_type,
                }))
            } else {
                output::note(&format!(
                    "wrote {} bytes to {} (sha256 {})",
                    written.bytes,
                    path.display(),
                    written.sha256
                ));
                Ok(())
            }
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            http::stream_to(resp, &mut stdout, "writing stdout").map(|_| ())
        }
    }
}

/// Binary commands with `--json` need `--out` (§7.6).
pub fn require_out_with_json(out: Option<&std::path::Path>) -> Res {
    if output::json() && out.is_none() {
        return Err(Failure::usage(
            "with --json, binary output needs --out FILE (stdout then gets {path, bytes, sha256, content_type})",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etags() {
        for e in ["d42", "\"d42\"", "W/\"d42\"", " d42 "] {
            assert_eq!(if_match(e).unwrap(), "\"d42\"", "{e}");
        }
        assert_eq!(if_match("*").unwrap(), "*");
        assert_eq!(if_match("r1").unwrap(), "\"r1\"");
        for bad in ["", "\"\"", "a b", "d\"42"] {
            assert!(if_match(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn operations_name_real_commands() {
        use clap::CommandFactory;
        let root = Cli::command();
        for (_, _, path) in OPERATIONS {
            let mut cmd = &root;
            for word in path.split(' ') {
                cmd = cmd
                    .find_subcommand(word)
                    .unwrap_or_else(|| panic!("no command {path}"));
            }
        }
    }
}
