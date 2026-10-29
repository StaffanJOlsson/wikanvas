//! The `wikanvas version`, `wikanvas exit-codes` and `wikanvas help` commands (owner: W8)

use std::fmt::Write;

use clap::CommandFactory;
use wikanvas_api::cli::ExitCode;
use serde_json::{Value, json};

use super::{Ctx, Res};
use crate::args::{Cli, HelpArgs};
use crate::error::Failure;
use crate::http::{Client, Req};
use crate::output;

/// `wikanvas version`: `{cli: {version, api_version}, server: {version, commit, api_version} | null}`. `server` is
/// null when no URL is configured; when the server cannot be asked, also with a `server_unavailable` warning.
pub fn version(ctx: &Ctx) -> Res {
    let cli =
        json!({ "version": env!("CARGO_PKG_VERSION"), "api_version": wikanvas_api::API_VERSION });
    let server = match server_version(ctx) {
        Ok(v) => v,
        Err(f) => {
            output::warn(
                "server_unavailable",
                &format!("no server version: {}", f.message()),
                Some(json!({ "error": f.line() })),
            );
            Value::Null
        }
    };
    if output::json() {
        return output::print_json(&json!({ "cli": cli, "server": server }));
    }
    let mut text = format!(
        "wikanvas {} (API {})\n",
        env!("CARGO_PKG_VERSION"),
        wikanvas_api::API_VERSION
    );
    if !server.is_null() {
        let s = |k: &str| server[k].as_str().unwrap_or("-").to_owned();
        let _ = writeln!(
            text,
            "server {} ({}) (API {})",
            s("version"),
            s("commit"),
            s("api_version")
        );
    }
    output::print_text(&text)
}

fn server_version(ctx: &Ctx) -> Result<Value, Failure> {
    let resolved = ctx.resolved()?;
    if resolved.url.is_none() {
        return Ok(Value::Null);
    }
    let client = Client::new(&resolved, &ctx.env, ctx.g.retries, ctx.g.timeout)?;
    let text = client.send_text(&Req::get("/api/v1/config").public())?.text;
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("GET /config: not JSON: {e}")))?;
    Ok(json!({ "version": v["version"], "commit": v["commit"], "api_version": v["api_version"] }))
}

/// `wikanvas exit-codes`: the §7.7 table. `--json` prints `[{"code": 0, "name": "success", "meaning": "Success"}, …]`;
/// without it, one line per code with its name, meaning and typical causes.
pub fn exit_codes() -> Res {
    if output::json() {
        output::print_json(&ExitCode::table())
    } else {
        output::print_text(&exit_codes_text())
    }
}

fn exit_codes_text() -> String {
    let width = ExitCode::ALL
        .iter()
        .map(|c| c.name().len())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for code in ExitCode::ALL {
        let _ = write!(
            text,
            "{:>2}  {:<width$}  {}",
            code.code(),
            code.name(),
            code.meaning()
        );
        if !code.causes().is_empty() {
            let _ = write!(text, ": {}", code.causes());
        }
        text.push('\n');
    }
    text
}

fn arg_name(a: &clap::Arg) -> String {
    match a.get_long() {
        Some(long) => format!("--{long}"),
        None => a
            .get_value_names()
            .and_then(|v| v.first())
            .map(|v| v.to_string())
            .unwrap_or_else(|| a.get_id().to_string().to_ascii_uppercase()),
    }
}

fn describe_arg(a: &clap::Arg) -> Value {
    let values: Vec<String> = a
        .get_possible_values()
        .iter()
        .map(|p| p.get_name().to_owned())
        .collect();
    let defaults: Vec<String> = a
        .get_default_values()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    let takes_value = a.get_action().takes_values();
    let multiple = matches!(a.get_action(), clap::ArgAction::Append)
        || a.get_num_args().is_some_and(|r| r.max_values() > 1);
    let mut v = json!({
        "name": arg_name(a),
        "required": a.is_required_set(),
        "multiple": multiple,
        "help": a.get_help().map(|h| h.to_string()),
    });
    if !a.is_positional() {
        v["value"] = if takes_value {
            json!(
                a.get_value_names()
                    .and_then(|n| n.first())
                    .map(|n| n.to_string())
            )
        } else {
            Value::Null
        };
        if let Some(s) = a.get_short() {
            v["short"] = json!(format!("-{s}"));
        }
    }
    if !values.is_empty() {
        v["values"] = json!(values);
    }
    if !defaults.is_empty() {
        v["default"] = json!(defaults.join(","));
    }
    v
}

fn visible(a: &clap::Arg) -> bool {
    !a.is_hide_set() && !matches!(a.get_id().as_str(), "help" | "version")
}

/// The command tree of `wikanvas help --json` for `cmd` (full path `path`).
pub fn describe(cmd: &clap::Command, path: &str) -> Value {
    let args: Vec<&clap::Arg> = cmd
        .get_arguments()
        .filter(|a| visible(a) && !a.is_global_set())
        .collect();
    let arguments: Vec<Value> = args
        .iter()
        .filter(|a| a.is_positional())
        .map(|a| describe_arg(a))
        .collect();
    let flags: Vec<Value> = args
        .iter()
        .filter(|a| !a.is_positional())
        .map(|a| describe_arg(a))
        .collect();
    let one_of: Vec<Vec<String>> = cmd
        .get_groups()
        .filter(|g| g.is_required_set())
        .map(|g| {
            g.get_args()
                .filter_map(|id| cmd.get_arguments().find(|a| a.get_id() == id))
                .map(arg_name)
                .collect()
        })
        .collect();
    let commands: Vec<Value> = cmd
        .get_subcommands()
        .map(|sub| {
            let p = if path.is_empty() {
                sub.get_name().to_owned()
            } else {
                format!("{path} {}", sub.get_name())
            };
            describe(sub, &p)
        })
        .collect();
    let mut v = json!({
        "name": cmd.get_name(),
        "path": path,
        "about": cmd.get_about().map(|a| a.to_string()),
        "arguments": arguments,
        "flags": flags,
    });
    if !one_of.is_empty() {
        v["one_of"] = json!(one_of);
    }
    if !commands.is_empty() {
        v["commands"] = json!(commands);
    }
    v
}

/// The full tree: the commands, the global flags and the exit codes.
pub fn tree() -> Value {
    let mut root = Cli::command();
    root.build();
    let global: Vec<Value> = root
        .get_arguments()
        .filter(|a| visible(a))
        .map(describe_arg)
        .collect();
    let commands = describe(&root, "")["commands"].clone();
    json!({
        "name": "wikanvas",
        "version": env!("CARGO_PKG_VERSION"),
        "api_version": wikanvas_api::API_VERSION,
        "global_flags": global,
        "commands": commands,
        "exit_codes": ExitCode::table(),
    })
}

/// `wikanvas help [COMMAND…] [--json]`.
pub fn help(a: HelpArgs) -> Res {
    let mut root = Cli::command().bin_name("wikanvas");
    root.build();
    let mut cmd = &mut root;
    let mut path = Vec::new();
    for word in &a.command {
        path.push(word.as_str());
        cmd = cmd.find_subcommand_mut(word).ok_or_else(|| {
            Failure::usage(format!("help: no command `wikanvas {}`", path.join(" ")))
        })?;
    }
    if output::json() {
        if a.command.is_empty() {
            return output::print_json(&tree());
        }
        return output::print_json(&describe(cmd, &path.join(" ")));
    }
    output::print_text(&cmd.render_long_help().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_table_has_one_line_per_code() {
        let text = exit_codes_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), ExitCode::ALL.len());
        assert_eq!(lines[0].trim_end(), " 0  success              Success");
        assert!(
            lines[9].starts_with(" 9  precondition_failed  Precondition failed: 412 rev_mismatch"),
            "{}",
            lines[9]
        );
        assert!(lines[12].starts_with("12  server_error"), "{}", lines[12]);
    }

    #[test]
    fn tree_has_groups_and_flags() {
        let t = tree();
        let doc = t["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "doc")
            .unwrap();
        let put = doc["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "put")
            .unwrap();
        assert_eq!(put["path"], "doc put");
        assert!(put["one_of"].to_string().contains("--if-match"), "{put}");
        assert!(t["global_flags"].to_string().contains("--json"));
        assert_eq!(t["exit_codes"].as_array().unwrap().len(), 13);
    }
}
