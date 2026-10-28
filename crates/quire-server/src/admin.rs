//! The admin commands of ARCHITECTURE §7.8 (owner: W2)
//!
//! They open the database in `QUIRE_DATA_DIR` the way `serve` does ([`AppState::open`]: pending migrations included,
//! with the pre-migration backup) and may run while the server is up (WAL; the writer waits on `busy_timeout`). Every
//! command takes `--json`, uses the exit codes of §7.7, and prints secrets only to stdout or `--token-out`.
//!
//! W0b: `create-owner` (owner row and workspace settings; the "Getting started" seed arrives with W2's
//! `seed::getting_started`) and `create-token`. The other commands exit 1 with "not implemented (W2)".

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use quire_api::cli::ExitCode;
use quire_api::tokens::{MAX_EXPIRY_DAYS, NewToken};
use quire_api::{CreatedToken, ErrorCode, Scope, User, Workspace};
use quire_store::tokens::CreatedVia;
use rusqlite::OptionalExtension;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{AdminArgs, AdminCommand, CreateOwnerArgs, CreateTokenArgs};
use crate::config::Config;
use crate::error::ApiError;
use crate::routes::auth::{load_user, load_workspace};
use crate::state::{self, AppState, now_ms};

/// The owner to create (`create-owner`, `TestServer`).
#[derive(Clone, Debug)]
pub struct NewOwner {
    pub email: String,
    pub name: String,
    pub workspace: String,
    /// The argon2id PHC string.
    pub password_hash: String,
    pub must_change_password: bool,
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
    pub api_token: quire_api::ApiToken,
}

/// Checks the owner's fields: an email with `@` (at most 254 bytes), a name of 1–200 characters, a workspace name
/// of 1–100 characters.
pub fn validate_owner(email: &str, name: &str, workspace: &str) -> Result<(), ApiError> {
    let email_ok = email.len() <= 254
        && !email.contains(char::is_whitespace)
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty());
    if !email_ok {
        return Err(ApiError::validation("email", "not an email address"));
    }
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

/// Creates the owner and the workspace settings in one writer transaction: 409 `already_setup` when an owner exists.
pub async fn create_owner(state: &AppState, owner: NewOwner, now: i64) -> Result<User, ApiError> {
    validate_owner(&owner.email, &owner.name, &owner.workspace)?;
    let id = Uuid::now_v7().to_string();
    let user = state
        .db
        .write(move |tx| {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'owner')",
                [],
                |r| r.get(0),
            )?;
            if exists {
                return Err(quire_store::Error::new(
                    ErrorCode::AlreadySetup,
                    "an owner already exists",
                ));
            }
            tx.execute(
                "INSERT INTO users (id, email, name, password_hash, role, theme, must_change_password, created_at,
                                    updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'owner', 'system', ?5, ?6, ?6)",
                rusqlite::params![
                    id,
                    owner.email.trim(),
                    owner.name.trim(),
                    owner.password_hash,
                    owner.must_change_password,
                    now
                ],
            )?;
            let name = serde_json::to_string(owner.workspace.trim())?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('workspace.name', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [name],
            )?;
            let instance = serde_json::to_string(&Uuid::now_v7().to_string())?;
            tx.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES ('instance.id', ?1), ('instance.created_at', ?2)",
                rusqlite::params![instance, now.to_string()],
            )?;
            Ok(load_user(tx, &id)?.0)
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
                Some(e) => conn
                    .query_row("SELECT id FROM users WHERE email = ?1", [e.trim()], |r| {
                        r.get(0)
                    })
                    .optional()?,
                None => conn
                    .query_row("SELECT id FROM users WHERE role = 'owner'", [], |r| {
                        r.get(0)
                    })
                    .optional()?,
            })
        })
        .await?;
    id.ok_or_else(|| {
        if by_email {
            ApiError::not_found("no user has this email")
        } else {
            ApiError::new(
                ErrorCode::SetupRequired,
                "no owner exists yet: run quire-server admin create-owner first",
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
            quire_store::tokens::create(tx, &user_id, &new, CreatedVia::Admin, None, now)
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

/// Runs `quire-server admin …` and returns the process exit code.
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
        eprintln!("quire-server admin: {}", err.message);
    }
}

fn print_json(value: &impl Serialize) -> Result<(), Failure> {
    let text = serde_json::to_string_pretty(value).map_err(ApiError::internal)?;
    println!("{text}");
    Ok(())
}

async fn open() -> Result<AppState, Failure> {
    let config = Config::from_env().map_err(|e| Failure {
        error: ApiError::new(ErrorCode::Validation, e.to_string()),
        exit: ExitCode::Configuration,
    })?;
    AppState::open(config).await.map_err(|e| Failure {
        error: ApiError::new(ErrorCode::Internal, e.to_string()),
        exit: ExitCode::Unexpected,
    })
}

async fn dispatch(command: AdminCommand, json: bool) -> Result<(), Failure> {
    match command {
        AdminCommand::CreateOwner(args) => create_owner_command(args, json).await,
        AdminCommand::CreateToken(args) => create_token_command(args, json).await,
        AdminCommand::ListTokens
        | AdminCommand::RevokeToken { .. }
        | AdminCommand::ResetPassword { .. }
        | AdminCommand::Backup { .. }
        | AdminCommand::Check => Err(Failure {
            error: ApiError::new(ErrorCode::Internal, "not implemented (W2)"),
            exit: ExitCode::Unexpected,
        }),
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
    let state = open().await?;
    let password_hash = state.hash_password(password).await?;
    let user = create_owner(
        &state,
        NewOwner {
            email: args.email,
            name: args.name,
            workspace: args.workspace,
            password_hash,
            must_change_password: generated.is_some(),
        },
        now_ms(),
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
    let created = create_token(&state, user_id, new, now_ms()).await?;
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
        assert!(validate_owner("a@quire.invalid", "A", "W").is_ok());
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
