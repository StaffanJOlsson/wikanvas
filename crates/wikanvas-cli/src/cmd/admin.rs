//! The `quire admin …` passthrough: execs `quire-server admin …` (§7.6, §7.8, D55; owner: W8)
//!
//! `quire-server` is found through `QUIRE_SERVER_BIN`, then next to the `quire` binary, then on `PATH`; none gives
//! exit 3. The process is replaced (exec), so stdin, stdout, stderr and the exit code are `quire-server`'s own. A
//! global `--json` is passed on.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

use super::{Ctx, Res};
use crate::args::AdminArgs;
use crate::error::Failure;
use crate::output;

fn executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where `quire-server` is (see the module docs).
pub fn find_server() -> Result<PathBuf, Failure> {
    if let Some(p) = std::env::var_os("QUIRE_SERVER_BIN").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        let sibling = dir.join("quire-server");
        if executable(&sibling) {
            return Ok(sibling);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate = dir.join("quire-server");
            if executable(&candidate) {
                return Ok(candidate);
            }
        }
    }
    Err(Failure::config(
        "quire-server not found: set QUIRE_SERVER_BIN, or install it next to quire or on PATH \
         (inside the image: docker compose exec quire quire admin …)",
    ))
}

pub fn run(_ctx: &Ctx, a: AdminArgs) -> Res {
    let bin = find_server()?;
    let mut args: Vec<OsString> = vec!["admin".into()];
    if output::json() && !a.args.iter().any(|x| x == "--json") {
        args.push("--json".into());
    }
    args.extend(a.args);
    let err = std::process::Command::new(&bin).args(&args).exec();
    Err(Failure::config(format!(
        "cannot run {}: {err}",
        bin.display()
    )))
}
