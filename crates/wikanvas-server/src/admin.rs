//! The admin commands of ARCHITECTURE §7.8 (owner: W2)
//!
//! They open the database in `WIKANVAS_DATA_DIR` the way `serve` does ([`AppState::open`]: pending migrations included,
//! with the pre-migration backup) and may run while the server is up (WAL; the writer waits on `busy_timeout`, and the
//! server's owner-exists cache sees a new owner on its next request). Every command takes `--json`, uses the exit
//! codes of §7.7, and prints secrets only to stdout or `--token-out`:
//! - `create-owner`: the `POST /setup` transaction without the code (owner, workspace settings, the "Getting started"
//!   seed unless `--no-seed`); exit 8 when an owner exists, and exit 3 (`conflict`), with nothing created, while no
//!   owner exists and `files/` holds any regular file (D144, as `serve` refuses to start);
//! - `create-token`, `list-tokens`, `revoke-token (<id> | --all --yes)`: as the API, for the owner;
//! - `reset-password --email E [--revoke-tokens]`: a random temporary password (printed once, must be changed), every
//!   session revoked, and with `--revoke-tokens` every API token too (D35);
//! - `backup [--out PATH]`: `VACUUM INTO` on the maintenance connection and `quick_check` (§12); without `--out` it is
//!   counted against `WIKANVAS_BACKUP_KEEP`;
//! - `check`: `quick_check`, `foreign_key_check`, a bounded cycle scan, the FTS `integrity-check`, and blob rows whose
//!   file is missing (§5); exit 1 when anything is wrong.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde::Serialize;
use wikanvas_api::cli::ExitCode;
use wikanvas_api::tokens::{MAX_EXPIRY_DAYS, NewToken};
use wikanvas_api::{ApiToken, BackupResult, CreatedToken, ErrorCode, Scope, User, Workspace};
use wikanvas_store::tokens::CreatedVia;

use crate::cli::{AdminArgs, AdminCommand, CreateOwnerArgs, CreateTokenArgs};
use crate::config::Config;
use crate::error::ApiError;
use crate::routes::auth::load_workspace;
use crate::state::{self, AppState};

/// The owner to create (`create-owner`, `POST /setup`, `TestServer`).
#[derive(Clone, Debug)]
pub struct NewOwner {
    pub email: String,
    pub name: String,
    pub workspace: String,
    /// The argon2id PHC string.
    pub password_hash: String,
    pub must_change_password: bool,
    /// Seed "Getting started" in the same transaction (`--no-seed` turns it off).
    pub seed: bool,
}

/// `create-owner --json` on stdout.
#[derive(Clone, Debug, Serialize)]
pub struct OwnerCreated {
    pub user: User,
    pub workspace: Workspace,
    /// The generated password (`--generate-password`), shown once; `null` with `--password-stdin`.
    pub generated_password: Option<String>,
}

/// `create-token --token-out F --json` on stdout.
#[derive(Clone, Debug, Serialize)]
pub struct TokenWritten {
    pub token_file: PathBuf,
    pub api_token: wikanvas_api::ApiToken,
}

/// `revoke-token --json` on stdout.
#[derive(Clone, Debug, Serialize)]
pub struct Revoked {
    /// Tokens revoked, children included.
    pub revoked: u64,
}

/// `reset-password --json` on stdout.
#[derive(Clone, Debug, Serialize)]
pub struct PasswordReset {
    pub user: User,
    /// The temporary password, shown once; it must be changed at the next sign-in.
    pub password: String,
    pub sessions_revoked: u64,
    pub tokens_revoked: u64,
}

/// `check --json` on stdout.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    /// `ok`, or SQLite's findings.
    pub quick_check: Vec<String>,
    pub foreign_key_violations: u64,
    /// Items whose ancestor chain leads back to themselves (at most 100).
    pub cycles: Vec<String>,
    /// `ok`, or the error of the FTS `integrity-check`.
    pub fts: String,
    /// Blob rows whose file is missing.
    pub missing_blobs: Vec<String>,
}

/// Checks the owner's fields: an email with `@` (at most 254 bytes), a name of 1–200 characters, a workspace name
/// of 1–100 characters.
pub fn validate_owner(email: &str, name: &str, workspace: &str) -> Result<(), ApiError> {
    wikanvas_store::users::check_email(email.trim())?;
    let chars = |s: &str| s.trim().chars().count();
    if !(1..=200).contains(&chars(name)) {
        return Err(ApiError::validation(
            "name",
            "a name is 1 to 200 characters",
        ));
    }
    if !(1..=100).contains(&chars(workspace)) {
        return Err(ApiError::validation(
            "workspace",
            "a workspace name is 1 to 100 characters",
        ));
    }
    Ok(())
}

/// Creates the owner, the workspace settings and (with `owner.seed`) the "Getting started" seed in one writer
/// transaction: 409 `already_setup` when an owner exists. The server's owner-exists cache flips at once.
pub async fn create_owner(state: &AppState, owner: NewOwner, now: i64) -> Result<User, ApiError> {
    validate_owner(&owner.email, &owner.name, &owner.workspace)?;
    let blobs = state.blobs.clone();
    let user = state
        .db
        .write(move |tx| {
            let row = wikanvas_store::users::create_owner(
                tx,
                &wikanvas_store::users::NewOwner {
                    email: owner.email.clone(),
                    name: owner.name.clone(),
                    password_hash: owner.password_hash.clone(),
                    must_change_password: owner.must_change_password,
                },
                now,
            )?;
            wikanvas_store::settings::init_workspace(tx, owner.workspace.trim(), now)?;
            if owner.seed {
                wikanvas_store::seed::getting_started(tx, &blobs, &row.id, now)?;
            }
            row.to_api()
        })
        .await?;
    state.owner_exists().await?;
    Ok(user)
}

/// The id of the user with `email`, or of the owner: 404 `not_found` for an unknown email, 409 `setup_required`
/// when there is no owner yet.
pub async fn find_user(state: &AppState, email: Option<String>) -> Result<String, ApiError> {
    let by_email = email.is_some();
    let id: Option<String> = state
        .db
        .read(move |conn| {
            Ok(match &email {
                Some(e) => wikanvas_store::users::by_email(conn, e)?.map(|u| u.id),
                None => wikanvas_store::users::owner(conn)?.map(|u| u.id),
            })
        })
        .await?;
    id.ok_or_else(|| {
        if by_email {
            ApiError::not_found("no user has this email")
        } else {
            ApiError::new(
                ErrorCode::SetupRequired,
                "no owner exists yet: run wikanvas-server admin create-owner first",
            )
        }
    })
}

/// Creates a token for `user_id` with `created_via = 'admin'`.
pub async fn create_token(
    state: &AppState,
    user_id: String,
    new: NewToken,
    now: i64,
) -> Result<CreatedToken, ApiError> {
    Ok(state
        .db
        .write(move |tx| {
            wikanvas_store::tokens::create(tx, &user_id, &new, CreatedVia::Admin, None, now)
        })
        .await?)
}

/// `--expires`: `<n>d` with n in 1–3650, or `never`.
pub fn parse_expires(s: &str) -> Result<Option<u32>, String> {
    if s == "never" {
        return Ok(None);
    }
    s.strip_suffix('d')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=MAX_EXPIRY_DAYS).contains(n))
        .map(Some)
        .ok_or_else(|| format!("{s:?}: expected <days>d (1d to {MAX_EXPIRY_DAYS}d) or never"))
}

/// A random password of 24 base62 characters (about 143 bits).
pub fn generate_password() -> Result<String, ApiError> {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut out = String::with_capacity(24);
    while out.len() < 24 {
        let mut buf = [0u8; 32];
        getrandom::fill(&mut buf).map_err(ApiError::internal)?;
        // 248 = 4 × 62: rejecting larger bytes keeps every character equally likely.
        out.extend(
            buf.iter()
                .filter(|b| **b < 248)
                .map(|b| char::from(ALPHABET[usize::from(*b % 62)])),
        );
    }
    out.truncate(24);
    Ok(out)
}

/// Writes `token` to `path` with mode 0600 (also when the file already existed).
fn write_secret(path: &Path, token: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
}

/// A failed admin command: the error, and the exit code it maps to.
struct Failure {
    error: ApiError,
    exit: ExitCode,
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Failure {
        let exit = ExitCode::from_status(error.code.status(), Some(error.code.as_str()));
        Failure { error, exit }
    }
}

/// Runs `wikanvas-server admin …` and returns the process exit code.
pub fn run(args: AdminArgs) -> ExitCode {
    let json = args.json;
    let result = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(dispatch(args.command, json)),
        Err(e) => Err(ApiError::internal(e).into()),
    };
    match result {
        Ok(()) => ExitCode::Success,
        Err(failure) => {
            report(&failure, json);
            failure.exit
        }
    }
}

fn report(failure: &Failure, json: bool) {
    let err = &failure.error;
    if json {
        let mut value = serde_json::json!({
            "error": {"code": err.code, "message": err.message},
            "request_id": null,
            "status": err.code.status(),
            "exit_code": failure.exit.code(),
        });
        if let Some(details) = &err.details {
            value["error"]["details"] = details.clone();
        }
        eprintln!("{value}");
    } else {
        eprintln!("wikanvas-server admin: {}", err.message);
    }
}

fn print_json(value: &impl Serialize) -> Result<(), Failure> {
    let text = serde_json::to_string_pretty(value).map_err(ApiError::internal)?;
    println!("{text}");
    Ok(())
}

fn config() -> Result<Config, Failure> {
    Config::from_env().map_err(|e| Failure {
        error: ApiError::new(ErrorCode::Validation, e.to_string()),
        exit: ExitCode::Configuration,
    })
}

async fn open_with(config: Config) -> Result<AppState, Failure> {
    AppState::open(config).await.map_err(|e| Failure {
        error: ApiError::new(ErrorCode::Internal, e.to_string()),
        exit: ExitCode::Unexpected,
    })
}

async fn open() -> Result<AppState, Failure> {
    open_with(config()?).await
}

/// The refusal of D144 for `create-owner` (exit 3, code `conflict`): files under `files/` while no owner exists.
fn files_without_owner(refused: Result<(), String>) -> Result<(), Failure> {
    refused.map_err(|message| Failure {
        error: ApiError::new(ErrorCode::Conflict, message),
        exit: ExitCode::Configuration,
    })
}

async fn dispatch(command: AdminCommand, json: bool) -> Result<(), Failure> {
    match command {
        AdminCommand::CreateOwner(args) => create_owner_command(args, json).await,
        AdminCommand::CreateToken(args) => create_token_command(args, json).await,
        AdminCommand::ListTokens => list_tokens_command(json).await,
        AdminCommand::RevokeToken { id, all, yes } => {
            revoke_token_command(id, all && yes, json).await
        }
        AdminCommand::ResetPassword {
            email,
            revoke_tokens,
        } => reset_password_command(email, revoke_tokens, json).await,
        AdminCommand::Backup { out } => backup_command(out, json).await,
        AdminCommand::Check => check_command(json).await,
    }
}

async fn create_owner_command(args: CreateOwnerArgs, json: bool) -> Result<(), Failure> {
    validate_owner(&args.email, &args.name, &args.workspace)?;
    let (password, generated) = if args.generate_password {
        let password = generate_password()?;
        (password.clone(), Some(password))
    } else {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| ApiError::validation("password", format!("reading stdin: {e}")))?;
        let password = input
            .strip_suffix('\n')
            .map(|s| s.strip_suffix('\r').unwrap_or(s))
            .unwrap_or(&input)
            .to_owned();
        (password, None)
    };
    state::check_password(&password)?;
    // D144: never the first owner over another database's files. Without a database, before one is created; with one,
    // before the owner is (an owner that exists already still gives exit 8 below).
    let config = config()?;
    files_without_owner(crate::app::refuse_files_without_database(
        &config,
        "create the owner",
    ))?;
    let state = open_with(config).await?;
    if !state.owner_exists().await? {
        files_without_owner(crate::app::refuse_files_without_owner(
            &state.config,
            "create the owner",
            "has no owner",
        ))?;
    }
    let password_hash = state.hash_password(password).await?;
    let now = state.now();
    let user = create_owner(
        &state,
        NewOwner {
            email: args.email,
            name: args.name,
            workspace: args.workspace,
            password_hash,
            must_change_password: generated.is_some(),
            seed: !args.no_seed,
        },
        now,
    )
    .await?;
    let workspace = state
        .db
        .read(load_workspace)
        .await
        .map_err(ApiError::from)?;
    if json {
        print_json(&OwnerCreated {
            user,
            workspace,
            generated_password: generated,
        })
    } else {
        println!(
            "Created the owner {} for the workspace {:?}.",
            user.email, workspace.name
        );
        if let Some(password) = generated {
            println!("Password (shown once; you must change it after signing in): {password}");
        }
        Ok(())
    }
}

async fn create_token_command(args: CreateTokenArgs, json: bool) -> Result<(), Failure> {
    let scope: Scope = args
        .scope
        .parse()
        .map_err(|_| ApiError::validation("scope", "expected read, write or admin"))?;
    let expires = parse_expires(&args.expires).map_err(|m| ApiError::validation("expires", m))?;
    let state = open().await?;
    let user_id = find_user(&state, args.email).await?;
    let new = NewToken {
        name: args.name,
        scopes: vec![scope],
        expires_in_days: Some(expires),
        password: None,
    };
    let created = create_token(&state, user_id, new, state.now()).await?;
    match args.token_out {
        Some(path) => {
            write_secret(&path, &created.token).map_err(|e| Failure {
                error: ApiError::new(
                    ErrorCode::Internal,
                    format!("{}: {e} (the token was created; revoke it)", path.display()),
                ),
                exit: ExitCode::Configuration,
            })?;
            if json {
                print_json(&TokenWritten {
                    token_file: path,
                    api_token: created.api_token,
                })
            } else {
                println!(
                    "Wrote the token {} to {}.",
                    created.api_token.prefix,
                    path.display()
                );
                Ok(())
            }
        }
        None if json => print_json(&created),
        None => {
            println!("{}", created.token);
            Ok(())
        }
    }
}

async fn list_tokens_command(json: bool) -> Result<(), Failure> {
    let state = open().await?;
    let user_id = find_user(&state, None).await?;
    let now = state.now();
    let tokens: Vec<ApiToken> = state
        .db
        .read(move |conn| wikanvas_store::tokens::list(conn, &user_id, now))
        .await
        .map_err(ApiError::from)?;
    if json {
        return print_json(&tokens);
    }
    for t in &tokens {
        let scopes: Vec<&str> = t.scopes.iter().map(|s| s.as_str()).collect();
        let expires = t
            .expires_at
            .map_or_else(|| "never".to_owned(), |e| e.to_rfc3339());
        println!(
            "{}  {}  {}  {}  expires {}",
            t.id,
            t.prefix,
            t.name,
            scopes.join(","),
            expires
        );
    }
    Ok(())
}

async fn revoke_token_command(id: Option<String>, all: bool, json: bool) -> Result<(), Failure> {
    let state = open().await?;
    let user_id = find_user(&state, None).await?;
    let now = state.now();
    let revoked = match id {
        Some(id) if !all => {
            let id = uuid::Uuid::parse_str(id.trim())
                .map_err(|_| ApiError::not_found("no such token"))?
                .to_string();
            let n = state
                .db
                .write(move |tx| wikanvas_store::tokens::revoke(tx, &user_id, &id, now))
                .await
                .map_err(ApiError::from)?;
            if n == 0 {
                return Err(ApiError::not_found("no such token (or it is revoked already)").into());
            }
            n
        }
        _ => state
            .db
            .write(move |tx| wikanvas_store::tokens::revoke_all(tx, &user_id, now))
            .await
            .map_err(ApiError::from)?,
    };
    if json {
        print_json(&Revoked { revoked })
    } else {
        println!("Revoked {revoked} token(s).");
        Ok(())
    }
}

async fn reset_password_command(
    email: String,
    revoke_tokens: bool,
    json: bool,
) -> Result<(), Failure> {
    let state = open().await?;
    let user_id = find_user(&state, Some(email)).await?;
    let password = generate_password()?;
    let hash = state.hash_password(password.clone()).await?;
    let now = state.now();
    let (user, sessions_revoked, tokens_revoked) = state
        .db
        .write(move |tx| {
            wikanvas_store::users::set_password(tx, &user_id, &hash, true, now)?;
            let sessions = wikanvas_store::sessions::delete_others(tx, &user_id, None)?;
            let tokens = if revoke_tokens {
                wikanvas_store::tokens::revoke_all(tx, &user_id, now)?
            } else {
                0
            };
            Ok((
                wikanvas_store::users::get(tx, &user_id)?.to_api()?,
                sessions,
                tokens,
            ))
        })
        .await
        .map_err(ApiError::from)?;
    if json {
        print_json(&PasswordReset {
            user,
            password,
            sessions_revoked,
            tokens_revoked,
        })
    } else {
        println!(
            "Temporary password for {} (shown once; it must be changed at the next sign-in): {password}",
            user.email
        );
        println!("Signed out {sessions_revoked} session(s); revoked {tokens_revoked} token(s).");
        Ok(())
    }
}

async fn backup_command(out: Option<PathBuf>, json: bool) -> Result<(), Failure> {
    let state = open().await?;
    let (path, size) = match out {
        None => crate::jobs::backup_now(&state).await?,
        Some(path) => {
            state.check_free_space()?;
            let db = state.db.clone();
            let target = path.clone();
            tokio::task::spawn_blocking(move || wikanvas_store::backup::to_path(&db, &target))
                .await
                .map_err(ApiError::internal)?
                .map_err(ApiError::from)?;
            let size = std::fs::metadata(&path).map_err(ApiError::internal)?.len();
            (path, size)
        }
    };
    let result = BackupResult {
        path: path.display().to_string(),
        size,
    };
    if json {
        print_json(&result)
    } else {
        println!("Backed up to {} ({size} bytes).", result.path);
        Ok(())
    }
}

/// The checks of `wikanvas-server admin check` (§5), on a maintenance connection.
pub fn check_database(
    db: &wikanvas_store::Db,
    blobs: &wikanvas_store::BlobStore,
) -> wikanvas_store::Result<CheckReport> {
    let conn = db.maintenance()?;
    let quick_check: Vec<String> = conn
        .prepare("PRAGMA quick_check")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let foreign_key_violations: i64 =
        conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    let cycles: Vec<String> = conn
        .prepare(
            "WITH RECURSIVE walk(start, id, depth) AS (
               SELECT id, parent_id, 1 FROM items WHERE parent_id IS NOT NULL
               UNION ALL
               SELECT w.start, i.parent_id, w.depth + 1 FROM walk w JOIN items i ON i.id = w.id
               WHERE i.parent_id IS NOT NULL AND w.id <> w.start AND w.depth < 10000
             )
             SELECT DISTINCT start FROM walk WHERE id = start LIMIT 100",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let fts = match conn.execute(
        "INSERT INTO item_fts(item_fts, rank) VALUES ('integrity-check', 1)",
        [],
    ) {
        Ok(_) => "ok".to_owned(),
        Err(e) => e.to_string(),
    };
    let shas: Vec<String> = conn
        .prepare("SELECT sha256 FROM blobs ORDER BY sha256")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let missing_blobs: Vec<String> = shas
        .into_iter()
        .filter(|s| {
            wikanvas_store::Sha256::parse(s).map_or(true, |sha| !blobs.path(&sha).is_file())
        })
        .collect();
    let ok = quick_check == ["ok"]
        && foreign_key_violations == 0
        && cycles.is_empty()
        && fts == "ok"
        && missing_blobs.is_empty();
    Ok(CheckReport {
        ok,
        quick_check,
        foreign_key_violations: foreign_key_violations as u64,
        cycles,
        fts,
        missing_blobs,
    })
}

async fn check_command(json: bool) -> Result<(), Failure> {
    let state = open().await?;
    let (db, blobs) = (state.db.clone(), state.blobs.clone());
    let report = tokio::task::spawn_blocking(move || check_database(&db, &blobs))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::from)?;
    if json {
        print_json(&report)?;
    } else {
        println!("quick_check: {}", report.quick_check.join("; "));
        println!("foreign key violations: {}", report.foreign_key_violations);
        println!("cycles: {}", report.cycles.len());
        println!("search index: {}", report.fts);
        println!("missing blob files: {}", report.missing_blobs.len());
    }
    if report.ok {
        Ok(())
    } else {
        Err(Failure {
            error: ApiError::new(ErrorCode::Internal, "the database check found problems"),
            exit: ExitCode::Unexpected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_values() {
        assert_eq!(parse_expires("90d"), Ok(Some(90)));
        assert_eq!(parse_expires("1d"), Ok(Some(1)));
        assert_eq!(parse_expires("never"), Ok(None));
        for bad in ["0d", "3651d", "90", "d", "1w", "-1d"] {
            assert!(parse_expires(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn generated_passwords() {
        let a = generate_password().unwrap();
        let b = generate_password().unwrap();
        assert_eq!(a.len(), 24);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
        assert!(state::check_password(&a).is_ok());
    }

    #[test]
    fn owner_fields() {
        assert!(validate_owner("a@wikanvas.invalid", "A", "W").is_ok());
        assert_eq!(
            validate_owner("nope", "A", "W")
                .unwrap_err()
                .details
                .unwrap()["field"],
            "email"
        );
        assert!(validate_owner("a b@x", "A", "W").is_err());
        assert!(validate_owner("a@x", " ", "W").is_err());
        assert!(validate_owner("a@x", "A", &"w".repeat(101)).is_err());
    }
}
