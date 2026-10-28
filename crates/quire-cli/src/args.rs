//! The clap command tree of ARCHITECTURE §7.6 (owner: W8)
//!
//! The table of §7.6 is the contract for agents (R3): every command, flag and value below is part of it, and
//! `quire help --json` prints the same tree. Environment variables and the profile file are read by `config` (the URL
//! and the token resolve as a pair), never by clap.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{ArgAction, ArgGroup, Args, ColorChoice, Parser, Subcommand};
use quire_api::cli::ExitCode;

use crate::output;

/// `quire`: the command-line client for Quire's HTTP API.
#[derive(Debug, Parser)]
#[command(
    name = "quire",
    version,
    about = "The command-line client for Quire's HTTP API (/api/v1)",
    color = ColorChoice::Never,
    subcommand_required = true,
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// The global flags of §7.6. Every one is accepted before or after the command.
#[derive(Clone, Debug, Default, Args)]
pub struct GlobalArgs {
    /// Print exactly one JSON document on stdout, and JSON Lines on stderr
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub json: bool,

    /// The Quire URL, e.g. https://wiki.example.org (overrides QUIRE_URL and the profile's url)
    #[arg(long, global = true, value_name = "URL")]
    pub url: Option<String>,

    /// The profile of the config file to use (QUIRE_URL, QUIRE_TOKEN and QUIRE_TOKEN_FILE are then ignored)
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// Read the API token from this file
    #[arg(long, global = true, value_name = "FILE")]
    pub token_file: Option<PathBuf>,

    /// Read the API token from stdin
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "token_file")]
    pub token_stdin: bool,

    /// Request timeout in seconds [default: 30; 300 for transfers]
    #[arg(long, global = true, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: Option<u64>,

    /// Retries of requests the server did not process
    #[arg(long, global = true, value_name = "N", default_value_t = 2)]
    pub retries: u32,

    /// Trust only the CA certificates in this PEM file (overrides QUIRE_CA_FILE and the system bundle)
    #[arg(long, global = true, value_name = "FILE")]
    pub ca_file: Option<PathBuf>,

    /// Print method, URL, status, duration and request id of every request to stderr
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub debug: bool,
}

/// The command table of §7.6.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in with a token, sign out, and show the current token
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Change the current user
    #[command(subcommand)]
    Me(MeCmd),
    /// Browser sessions of the current user
    #[command(subcommand)]
    Session(SessionCmd),
    /// Personal access tokens
    #[command(subcommand)]
    Token(TokenCmd),
    /// The CLI's own configuration (local)
    #[command(subcommand)]
    Config(ConfigCmd),
    /// The workspace's name and logo
    #[command(subcommand)]
    Workspace(WorkspaceCmd),
    /// The sidebar: collections, stars and the drafts count
    Sidebar,
    /// Collections
    #[command(subcommand)]
    Collection(CollectionCmd),
    /// Documents and boards
    #[command(subcommand)]
    Item(ItemCmd),
    /// A collection's tree of items
    Tree(TreeArgs),
    /// The item lists: recent, updated, drafts, starred, archive
    #[command(subcommand)]
    List(ListCmd),
    /// The trash
    #[command(subcommand)]
    Trash(TrashCmd),
    /// Full-text search, or title search with --titles
    Search(SearchArgs),
    /// Stars
    #[command(subcommand)]
    Star(StarCmd),
    /// Document content as Markdown or ProseMirror JSON
    #[command(subcommand)]
    Doc(DocCmd),
    /// Board content as the native Inkboard envelope
    #[command(subcommand)]
    Board(BoardCmd),
    /// Uploaded files
    #[command(subcommand)]
    Attachment(AttachmentCmd),
    /// Download the workspace export (a tar)
    Export(ExportArgs),
    /// Make a backup on the server (POST /admin/backup)
    Backup,
    /// Call any API route, with auth, retries and error mapping
    Api(ApiArgs),
    /// Run `quire-server admin …` (ARCHITECTURE §7.8)
    Admin(AdminArgs),
    /// The CLI's and the server's versions
    Version,
    /// Print the exit-code table (§7.7)
    ExitCodes,
    /// Help for quire or one of its commands; --json prints the whole command tree
    Help(HelpArgs),
}

// ------------------------------------------------------------------------------------------------- shared arguments

/// A position among siblings: "position" in §7.6. Neither flag means last.
#[derive(Clone, Debug, Default, Args)]
pub struct PositionArgs {
    /// Place it right after this sibling
    #[arg(long, value_name = "REF", conflicts_with = "before")]
    pub after: Option<String>,
    /// Place it right before this sibling
    #[arg(long, value_name = "REF")]
    pub before: Option<String>,
}

/// `--limit`, `--cursor` and `--all` of the list commands.
#[derive(Clone, Debug, Default, Args)]
pub struct ListArgs {
    /// Page size, 1-100 (default 25; for search --titles 1-20, default 10)
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: Option<u32>,
    /// The next_cursor of an earlier page
    #[arg(long, value_name = "CURSOR")]
    pub cursor: Option<String>,
    /// Follow next_cursor and print one merged page (at most 10,000 items)
    #[arg(long)]
    pub all: bool,
}

/// The `--if-match ETAG | --force` pair of the content writes (§7.6 "Non-interactive").
#[derive(Clone, Debug, Default, Args)]
#[group(id = "precondition", required = true, multiple = false)]
pub struct Precondition {
    /// The ETag the change is based on: d42, "d42" and W/"d42" are all accepted
    #[arg(long, value_name = "ETAG")]
    pub if_match: Option<String>,
    /// Overwrite whatever is current (If-Match: *)
    #[arg(long)]
    pub force: bool,
}

// ------------------------------------------------------------------------------------------------- auth, me, session

#[derive(Debug, Subcommand)]
pub enum AuthCmd {
    /// Check a token (--url, and --token-stdin or --token-file) and save it in a profile (default "default")
    Login,
    /// Remove the profile's token; --revoke first revokes it on the server
    Logout(LogoutArgs),
    /// Show the current user and token
    Whoami,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Revoke the token on the server first (DELETE /tokens/current)
    #[arg(long)]
    pub revoke: bool,
}

#[derive(Debug, Subcommand)]
pub enum MeCmd {
    /// Change your name, email (a session only) or theme
    Update(MeUpdate),
}

#[derive(Debug, Args)]
pub struct MeUpdate {
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    #[arg(long, value_name = "EMAIL")]
    pub email: Option<String>,
    #[arg(long, value_parser = ["light", "dark", "system"])]
    pub theme: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum SessionCmd {
    /// List your browser sessions
    List,
    /// Revoke one session, or every other one with --others
    Revoke(SessionRevoke),
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").required(true).args(["id", "others"])))]
pub struct SessionRevoke {
    /// The session's id
    pub id: Option<String>,
    /// Revoke every session but the current one (from a token: every session)
    #[arg(long)]
    pub others: bool,
}

// ------------------------------------------------------------------------------------------------- token, config

#[derive(Debug, Subcommand)]
pub enum TokenCmd {
    /// List your tokens (never their secrets)
    List,
    /// Create a token and print its secret once
    Create(TokenCreate),
    /// Revoke a token and the tokens it created
    Revoke(IdArg),
}

#[derive(Debug, Args)]
pub struct TokenCreate {
    #[arg(long, value_name = "NAME")]
    pub name: String,
    #[arg(long, value_parser = ["read", "write", "admin"])]
    pub scope: String,
    /// <days>d (1-3650) or never [default: 90d]
    #[arg(long, value_name = "30d|never")]
    pub expires: Option<String>,
    /// Write the secret to this file (mode 0600) instead of stdout
    #[arg(long, value_name = "FILE")]
    pub token_out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct IdArg {
    pub id: String,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Show the URL and token the CLI would use, and where they come from
    Show,
    /// Print the profile file's path
    Path,
}

// ------------------------------------------------------------------------------------------------- workspace

#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd {
    /// Show the workspace's name and logo
    Get,
    /// Change the workspace's name or logo
    Set(WorkspaceSet),
}

#[derive(Debug, Args)]
pub struct WorkspaceSet {
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// An attachment id (from `quire attachment upload`)
    #[arg(long, value_name = "ATTACHMENT", conflicts_with = "no_logo")]
    pub logo: Option<String>,
    /// Remove the logo
    #[arg(long)]
    pub no_logo: bool,
}

// ------------------------------------------------------------------------------------------------- collections

#[derive(Debug, Subcommand)]
pub enum CollectionCmd {
    /// List the collections
    List,
    /// Show one collection
    Get(CollectionArg),
    /// Create a collection
    Create(CollectionCreate),
    /// Change a collection
    Update(CollectionUpdate),
    /// Move a collection in the sidebar
    Move(CollectionMove),
    /// Move a collection and its items to the trash
    Delete(CollectionArg),
    /// Restore a collection from the trash
    Restore(CollectionArg),
    /// The collection's items, newest change first
    Updated(CollectionUpdated),
}

#[derive(Debug, Args)]
pub struct CollectionArg {
    /// A UUID, url_id or Quire URL
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
}

#[derive(Debug, Args)]
pub struct CollectionCreate {
    #[arg(long, value_name = "NAME")]
    pub name: String,
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    /// A lucide icon name
    #[arg(long, value_name = "ICON")]
    pub icon: Option<String>,
    /// #RRGGBB
    #[arg(long, value_name = "#RRGGBB")]
    pub color: Option<String>,
    #[command(flatten)]
    pub position: PositionArgs,
}

#[derive(Debug, Args)]
pub struct CollectionUpdate {
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    #[arg(long, value_name = "ICON")]
    pub icon: Option<String>,
    #[arg(long, value_name = "#RRGGBB")]
    pub color: Option<String>,
    /// How the tree orders the items
    #[arg(long, value_parser = ["manual", "title", "updated", "created"])]
    pub sort: Option<String>,
}

#[derive(Debug, Args)]
pub struct CollectionMove {
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
    #[command(flatten)]
    pub position: PositionArgs,
}

#[derive(Debug, Args)]
pub struct CollectionUpdated {
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
    #[command(flatten)]
    pub list: ListArgs,
}

// ------------------------------------------------------------------------------------------------- items

#[derive(Debug, Subcommand)]
pub enum ItemCmd {
    /// Show an item with its collection, breadcrumbs and state
    Get(ItemArg),
    /// Create a document or a board (published on create; `item unpublish` makes a draft)
    Create(ItemCreate),
    /// Change an item's title or width
    Update(ItemUpdate),
    /// Move an item under another parent, to the top level, or to another collection
    Move(ItemMove),
    /// Copy an item (not its children)
    Duplicate(ItemArg),
    /// Publish an item
    Publish(ItemPublish),
    /// Make an item a draft
    Unpublish(ItemArg),
    /// Archive an item and its subtree
    Archive(ItemArg),
    /// Unarchive an item
    Unarchive(ItemArg),
    /// Move an item and its subtree to the trash
    Trash(ItemTrash),
    /// Restore an item from the trash
    Restore(ItemArg),
    /// The item's children
    Children(ItemArg),
    /// The plain text that search indexes
    Text(ItemArg),
}

#[derive(Debug, Args)]
pub struct ItemArg {
    /// A UUID, url_id or Quire URL
    #[arg(value_name = "ITEM")]
    pub item: String,
}

#[derive(Debug, Args)]
pub struct ItemCreate {
    #[arg(long, value_name = "COLLECTION")]
    pub collection: String,
    #[arg(long, value_parser = ["doc", "board"])]
    pub kind: String,
    #[arg(long, value_name = "ITEM")]
    pub parent: Option<String>,
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    #[command(flatten)]
    pub position: PositionArgs,
    /// Fill the new document with this Markdown file (- for stdin)
    #[arg(long, value_name = "FILE")]
    pub md: Option<String>,
    /// Take the title from the Markdown's leading level-1 heading
    #[arg(long, requires = "md")]
    pub title_h1: bool,
}

#[derive(Debug, Args)]
pub struct ItemUpdate {
    #[arg(value_name = "ITEM")]
    pub item: String,
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    #[arg(long, value_name = "BOOL", value_parser = clap::value_parser!(bool))]
    pub full_width: Option<bool>,
    /// The item's metadata ETag (m3, "m3" or W/"m3"); a stale one exits 9
    #[arg(long, value_name = "ETAG")]
    pub if_match: Option<String>,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").required(true).args(["parent", "top"])))]
pub struct ItemMove {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// The new parent item
    #[arg(long, value_name = "ITEM")]
    pub parent: Option<String>,
    /// Move to the top level of the collection
    #[arg(long)]
    pub top: bool,
    /// Move to another collection
    #[arg(long, value_name = "COLLECTION")]
    pub collection: Option<String>,
    #[command(flatten)]
    pub position: PositionArgs,
}

#[derive(Debug, Args)]
pub struct ItemPublish {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// Publish draft ancestors too
    #[arg(long)]
    pub with_ancestors: bool,
}

#[derive(Debug, Args)]
pub struct ItemTrash {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// Delete permanently if the item is new, untitled, empty and childless (otherwise exit 8)
    #[arg(long)]
    pub discard_empty: bool,
}

#[derive(Debug, Args)]
pub struct TreeArgs {
    /// A UUID, url_id or Quire URL
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
}

// ------------------------------------------------------------------------------------------------- lists, trash

#[derive(Debug, Subcommand)]
pub enum ListCmd {
    /// Recently viewed items
    Recent(ListArgs),
    /// Recently changed items
    Updated(ListArgs),
    /// Unpublished items
    Drafts(ListArgs),
    /// Starred items
    Starred(ListArgs),
    /// Archived items
    Archive(ListArgs),
}

#[derive(Debug, Subcommand)]
pub enum TrashCmd {
    /// The trashed collections and items (roots only)
    List,
    /// Delete one trashed item, one trashed collection, or the whole trash, permanently (needs --yes and admin)
    Purge(TrashPurge),
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").required(true).args(["item", "collection", "all"])))]
pub struct TrashPurge {
    /// A trashed item
    #[arg(value_name = "ITEM")]
    pub item: Option<String>,
    /// A trashed collection
    #[arg(long, value_name = "COLLECTION")]
    pub collection: Option<String>,
    /// Everything in the trash
    #[arg(long)]
    pub all: bool,
    /// Confirm the permanent deletion
    #[arg(long, required = true)]
    pub yes: bool,
}

// ------------------------------------------------------------------------------------------------- search, stars

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// The query: "…" keeps a phrase, -word excludes a word
    #[arg(value_name = "QUERY")]
    pub query: String,
    /// Search titles only (GET /search/titles)
    #[arg(long)]
    pub titles: bool,
    #[arg(long, value_name = "COLLECTION")]
    pub collection: Option<String>,
    #[arg(long, value_parser = ["doc", "board"])]
    pub kind: Option<String>,
    /// An RFC 3339 time
    #[arg(long, value_name = "TIME")]
    pub updated_after: Option<String>,
    #[arg(long)]
    pub include_archived: bool,
    /// Leave out unpublished items
    #[arg(long)]
    pub no_drafts: bool,
    #[arg(long, value_parser = ["relevance", "updated"])]
    pub sort: Option<String>,
    #[command(flatten)]
    pub list: ListArgs,
}

#[derive(Debug, Subcommand)]
pub enum StarCmd {
    /// Star an item
    Add(StarAdd),
    /// Remove an item's star
    Remove(ItemArg),
}

#[derive(Debug, Args)]
pub struct StarAdd {
    #[arg(value_name = "ITEM")]
    pub item: String,
    #[command(flatten)]
    pub position: PositionArgs,
}

// ------------------------------------------------------------------------------------------------- doc, board

#[derive(Debug, Subcommand)]
pub enum DocCmd {
    /// Get a document as Markdown (default) or ProseMirror JSON, with its ETag
    Get(DocGet),
    /// Replace a document's content from Markdown or ProseMirror JSON
    Put(DocPut),
    /// The document schema (nodes and marks of ProseMirror JSON)
    Schema,
}

#[derive(Debug, Args)]
pub struct DocGet {
    #[arg(value_name = "ITEM")]
    pub item: String,
    #[arg(long, value_parser = ["md", "json"], default_value = "md")]
    pub format: String,
    /// Markdown: start with "# <title>"
    #[arg(long)]
    pub title_h1: bool,
    /// Write the content to this file
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("input").required(true).args(["md", "json_input"])))]
pub struct DocPut {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// Markdown file (- for stdin)
    #[arg(long, value_name = "FILE")]
    pub md: Option<String>,
    /// ProseMirror JSON file (- for stdin): {"schema_version": 1, "doc": …} or a bare {"type": "doc", …}. Also
    /// spelled `--json FILE` after the ITEM (the global --json takes no value)
    #[arg(id = "json_input", long = "json-input", value_name = "FILE")]
    pub json: Option<String>,
    #[command(flatten)]
    pub precondition: Precondition,
    /// Merge with newer changes instead of failing on a stale ETag
    #[arg(long)]
    pub merge: bool,
    /// Markdown: a leading level-1 heading becomes the title
    #[arg(long, requires = "md")]
    pub title_h1: bool,
    /// Markdown: upload the local images it references and rewrite their URLs
    #[arg(long, requires = "md")]
    pub attach_images: bool,
    /// The directory image paths are relative to (default: the Markdown file's)
    #[arg(long, value_name = "DIR", requires = "attach_images")]
    pub base_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum BoardCmd {
    /// Get a board's Inkboard envelope with its ETag
    Get(BoardGet),
    /// Replace a board with an Inkboard envelope (FILE or - for stdin)
    Put(BoardPut),
    /// A page's SVG preview
    Preview(BoardPreview),
}

#[derive(Debug, Args)]
pub struct BoardGet {
    #[arg(value_name = "ITEM")]
    pub item: String,
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BoardPut {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// The envelope file (- for stdin)
    #[arg(value_name = "FILE")]
    pub file: String,
    #[command(flatten)]
    pub precondition: Precondition,
}

#[derive(Debug, Args)]
pub struct BoardPreview {
    #[arg(value_name = "ITEM")]
    pub item: String,
    /// The page index
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub page: u32,
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

// ------------------------------------------------------------------------------------------------- files, export, api

#[derive(Debug, Subcommand)]
pub enum AttachmentCmd {
    /// Upload a file (FILE or - for stdin), sent as application/octet-stream
    Upload(AttachmentUpload),
    /// Download an attachment
    Get(AttachmentGet),
}

#[derive(Debug, Args)]
pub struct AttachmentUpload {
    /// The file (- for stdin)
    #[arg(value_name = "FILE")]
    pub file: String,
    /// The item it is for
    #[arg(long, value_name = "ITEM")]
    pub item: Option<String>,
    /// The stored file name (default: FILE's name)
    #[arg(long, value_name = "NAME")]
    pub filename: Option<String>,
}

#[derive(Debug, Args)]
pub struct AttachmentGet {
    pub id: String,
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ApiArgs {
    /// GET, POST, PUT, PATCH, DELETE or HEAD
    #[arg(value_name = "METHOD")]
    pub method: String,
    /// A path under /api/, with its query string
    #[arg(value_name = "PATH")]
    pub path: String,
    /// The request body (FILE or - for stdin), sent as application/json unless --header sets Content-Type
    #[arg(long, value_name = "FILE")]
    pub input: Option<String>,
    /// An extra request header (not Authorization, Cookie or Host)
    #[arg(long, value_name = "K:V")]
    pub header: Vec<String>,
    /// Write a non-JSON response to this file
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct AdminArgs {
    /// The arguments of `quire-server admin`, e.g. create-token --name ci --scope write
    #[arg(value_name = "ARGS", trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
    pub args: Vec<OsString>,
}

#[derive(Debug, Args)]
pub struct HelpArgs {
    /// The command to describe, e.g. `doc put`
    #[arg(value_name = "COMMAND")]
    pub command: Vec<String>,
}

/// Global flags that take a value (their value is never a positional).
const GLOBAL_VALUE_FLAGS: &[&str] = &[
    "--url",
    "--profile",
    "--token-file",
    "--timeout",
    "--retries",
    "--ca-file",
];

/// `doc put <i> --json F` (§7.6) shares its name with the global `--json`, and clap cannot hold both. Inside
/// `doc put`, a `--json` that follows the ITEM and is followed by a value (`-` or a word not starting with `-`), or
/// `--json=F`, is the input flag and becomes `--json-input F`; any other `--json` is the global flag.
pub fn rewrite_doc_put_json(argv: Vec<OsString>) -> Vec<OsString> {
    let words: Vec<Option<&str>> = argv.iter().map(|a| a.to_str()).collect();
    // Find `doc put` as the subcommand path (skipping global flags and their values).
    let mut i = 1;
    let mut path = Vec::new();
    while i < words.len() && path.len() < 2 {
        match words[i] {
            Some(w) if GLOBAL_VALUE_FLAGS.contains(&w) => i += 2,
            Some(w) if w.starts_with('-') => i += 1,
            Some(w) => {
                path.push(w);
                i += 1;
            }
            None => break,
        }
    }
    if path != ["doc", "put"] {
        return argv;
    }
    let value_flags = ["--md", "--json-input", "--if-match", "--base-dir"];
    let mut out: Vec<OsString> = argv[..i].to_vec();
    let mut seen_item = false;
    let mut j = i;
    while j < argv.len() {
        let w = words[j];
        match w {
            Some(f) if GLOBAL_VALUE_FLAGS.contains(&f) || value_flags.contains(&f) => {
                out.extend(argv[j..(j + 2).min(argv.len())].iter().cloned());
                j += 2;
                continue;
            }
            Some(f) if f.starts_with("--json=") => {
                out.push(OsString::from(format!(
                    "--json-input={}",
                    &f["--json=".len()..]
                )));
            }
            Some("--json")
                if seen_item
                    && words
                        .get(j + 1)
                        .copied()
                        .flatten()
                        .is_some_and(|v| v == "-" || !v.starts_with('-')) =>
            {
                out.push(OsString::from("--json-input"));
                out.push(argv[j + 1].clone());
                j += 2;
                continue;
            }
            Some("--") => {
                out.extend(argv[j..].iter().cloned());
                break;
            }
            Some(f) if f.starts_with('-') && f != "-" => out.push(argv[j].clone()),
            _ => {
                seen_item = true;
                out.push(argv[j].clone());
            }
        }
        j += 1;
    }
    out
}

/// Parses an argv (the program name first) as the binary does, with the `doc put --json F` rewrite.
pub fn try_parse_from(argv: Vec<OsString>) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(rewrite_doc_put_json(argv))
}

/// Parses the process arguments. A usage error is printed (as a JSON line when `--json` appears anywhere in argv,
/// §7.7) and returned as its exit code; `--help` and `--version` print to stdout and return success.
pub fn parse() -> Result<Cli, ExitCode> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let json = output::json_in_argv(&argv);
    try_parse_from(argv).map_err(|err| output::clap_error(&err, json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse_from(list: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("quire").chain(list.iter().copied()))
    }

    #[test]
    fn command_tree_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_before_and_after_the_command() {
        for list in [
            &["--json", "--debug", "--retries", "0", "exit-codes"][..],
            &["exit-codes", "--json", "--debug", "--retries", "0"][..],
        ] {
            let cli = parse_from(list).expect("parses");
            assert!(cli.global.json);
            assert!(cli.global.debug);
            assert_eq!(cli.global.retries, 0);
            assert!(matches!(cli.command, Command::ExitCodes));
        }
    }

    #[test]
    fn defaults() {
        let g = parse_from(&["exit-codes"]).expect("parses").global;
        assert!(!g.json);
        assert!(!g.debug);
        assert!(!g.token_stdin);
        assert_eq!(g.retries, 2);
        assert_eq!(g.timeout, None);
        assert_eq!(g.url, None);
        assert_eq!(g.profile, None);
        assert_eq!(g.token_file, None);
        assert_eq!(g.ca_file, None);
    }

    #[test]
    fn every_global_flag_parses() {
        let g = parse_from(&[
            "exit-codes",
            "--url",
            "https://wiki.example.org",
            "--profile",
            "home",
            "--token-file",
            "/run/secrets/quire-token",
            "--timeout",
            "5",
            "--retries",
            "4",
            "--ca-file",
            "ca.pem",
        ])
        .expect("parses")
        .global;
        assert_eq!(g.url.as_deref(), Some("https://wiki.example.org"));
        assert_eq!(g.profile.as_deref(), Some("home"));
        assert_eq!(
            g.token_file.as_deref(),
            Some(std::path::Path::new("/run/secrets/quire-token"))
        );
        assert_eq!(g.timeout, Some(5));
        assert_eq!(g.retries, 4);
        assert_eq!(g.ca_file.as_deref(), Some(std::path::Path::new("ca.pem")));
        assert!(
            parse_from(&["--token-stdin", "exit-codes"])
                .expect("parses")
                .global
                .token_stdin
        );
    }

    #[test]
    fn usage_errors() {
        let cases: [(&[&str], ErrorKind); 11] = [
            (
                &["--token-stdin", "--token-file", "t", "exit-codes"],
                ErrorKind::ArgumentConflict,
            ),
            (
                &["exit-codes", "--token-file", "t", "--token-stdin"],
                ErrorKind::ArgumentConflict,
            ),
            (
                &["exit-codes", "--timeout", "0"],
                ErrorKind::ValueValidation,
            ),
            (
                &["exit-codes", "--retries", "-1"],
                ErrorKind::UnknownArgument,
            ),
            (&["exit-codes", "--bogus"], ErrorKind::UnknownArgument),
            (&["no-such-command"], ErrorKind::InvalidSubcommand),
            (&["--json"], ErrorKind::MissingSubcommand),
            (
                &["doc", "put", "x", "--md", "-"],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                &["board", "put", "x", "f", "--if-match", "r1", "--force"],
                ErrorKind::ArgumentConflict,
            ),
            (&["trash", "purge", "x"], ErrorKind::MissingRequiredArgument),
            (
                &["list", "recent", "--limit", "101"],
                ErrorKind::ValueValidation,
            ),
        ];
        for (list, kind) in cases {
            let err = parse_from(list)
                .err()
                .unwrap_or_else(|| panic!("{list:?} parsed"));
            assert_eq!(err.kind(), kind, "{list:?}");
            assert_eq!(err.exit_code(), 2, "{list:?}");
        }
        // No token flag takes the secret from argv (§7.6).
        assert!(parse_from(&["exit-codes", "--token", "qpat_x"]).is_err());
    }

    #[test]
    fn doc_put_json_input_and_global_json() {
        let parse = |list: &[&str]| {
            try_parse_from(
                std::iter::once("quire")
                    .chain(list.iter().copied())
                    .map(OsString::from)
                    .collect(),
            )
            .expect("parses")
        };
        let put = |cli: &Cli| match &cli.command {
            Command::Doc(DocCmd::Put(p)) => (p.md.clone(), p.json.clone()),
            other => panic!("{other:?}"),
        };
        let cli = parse(&["doc", "put", "X", "--json", "f.json", "--force"]);
        assert_eq!(put(&cli), (None, Some("f.json".into())));
        assert!(!cli.global.json);
        let cli = parse(&[
            "--json",
            "doc",
            "put",
            "X",
            "--json",
            "-",
            "--if-match",
            "d1",
            "--json",
        ]);
        assert_eq!(put(&cli), (None, Some("-".into())));
        assert!(cli.global.json);
        let cli = parse(&["doc", "put", "X", "--md", "a.md", "--force", "--json"]);
        assert_eq!(put(&cli), (Some("a.md".into()), None));
        assert!(cli.global.json);
        let cli = parse(&[
            "--url",
            "https://h",
            "doc",
            "put",
            "X",
            "--json=f.json",
            "--force",
        ]);
        assert_eq!(put(&cli), (None, Some("f.json".into())));
        // Before the ITEM, --json is the global flag and the next word is the ITEM.
        let cli = parse(&["doc", "put", "--json", "X", "--md", "-", "--force"]);
        assert!(cli.global.json);
        // Other commands are untouched.
        let cli = parse(&["doc", "get", "X", "--json"]);
        assert!(cli.global.json);
    }

    #[test]
    fn admin_takes_everything_after_it() {
        let cli = parse_from(&["admin", "create-token", "--name", "ci", "--scope", "write"])
            .expect("parses");
        let Command::Admin(admin) = cli.command else {
            panic!("admin")
        };
        assert_eq!(
            admin.args,
            ["create-token", "--name", "ci", "--scope", "write"].map(OsString::from)
        );
    }
}
