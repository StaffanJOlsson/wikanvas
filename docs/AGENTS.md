# Quire for agents: the `quire` CLI

How an AI agent (or a script) reads and writes a Quire wiki. The contract behind it is docs/ARCHITECTURE.md §7.6
(commands) and §7.7 (output and exit codes); `quire help --json` prints the same command tree.

## 1. What the CLI is and where it runs

`quire` is the command-line client of Quire's HTTP API (`/api/v1`). Every API operation has a command, except a few
web-only ones, which `quire api` still reaches. It never prompts, opens a browser or a pager, or prints colour: content
comes from a file argument or from stdin (`-`).

- **In the image.** The container holds both binaries. There the CLI needs no URL (it derives `http://127.0.0.1:8080`
  from `QUIRE_LISTEN`), so `docker compose exec -e QUIRE_TOKEN quire quire …` drives the API, and
  `docker compose exec quire quire admin …` runs the server's admin commands without a token.
- **Standalone.** `scripts/cli-dist.sh` builds a static binary, `dist/cli/quire-x86_64-unknown-linux-musl`, for any
  x86_64 Linux (with its `.sha256`, the licences and `THIRD_PARTY.md`).

## 2. Cheat sheet

`ITEM` and `COLL` are a UUID, a `url_id` (10 letters and digits) or a Quire URL (`…/doc/<slug>-<url_id>`). A position
is `--after REF` or `--before REF` (neither: last). Lists take `--limit N`, `--cursor C` and `--all` (follows
`next_cursor` up to 10,000 items, then prints the real `next_cursor` with a `truncated` warning). Items are published on
create; `item unpublish` makes a draft (D23).

| Group | Commands and main flags | `--json` prints |
|---|---|---|
| auth | `auth login --url U (--token-stdin \| --token-file F) [--profile P]`, `auth logout [--revoke]`, `auth whoami` | `Me` (with `auth.token`: prefix, scopes, expiry); logout: the ok object |
| me, session | `me update [--name N] [--email E] [--theme light\|dark\|system]`, `session list`, `session revoke (ID \| --others)` | `{user}`; the list; ok |
| token | `token list`, `token create --name N --scope read\|write\|admin [--expires 30d\|never] [--token-out F]`, `token revoke ID` | `[ApiToken]`; `{token, api_token}` or `{token_file, api_token}`; ok |
| config | `config show`, `config path` (local) | `{profile, url, url_source, token_source, token_prefix}`; `{path}` |
| workspace | `workspace get`, `workspace set [--name N] [--logo ATT \| --no-logo]`, `sidebar` | the body |
| collection | `collection list \| get COLL \| create --name N [--description D] [--icon I] [--color '#RRGGBB'] \| update COLL [--sort manual\|title\|updated\|created] \| move COLL \| delete COLL \| restore COLL \| updated COLL` | the body |
| item | `item get ITEM`, `item create --collection COLL --kind doc\|board [--parent ITEM] [--title T] [--md F\|- [--title-h1]]`, `item update ITEM [--title T] [--full-width true\|false] [--if-match E]`, `item move ITEM (--parent ITEM \| --top) [--collection COLL]`, `item duplicate\|publish [--with-ancestors]\|unpublish\|archive\|unarchive\|trash [--discard-empty]\|restore\|children\|text ITEM` | `ItemSummary` (`item get`: `ItemDetail`; `create --md`: `{item, doc}`) |
| tree | `tree COLL` | `{collection, items}` (flat; `parent_id` links them) |
| list, trash | `list recent\|updated\|drafts\|starred\|archive`, `trash list`, `trash purge (ITEM \| --collection COLL \| --all) --yes` | a `Page` `{items, next_cursor}`; the trash; ok |
| search | `search QUERY [--collection COLL] [--kind doc\|board] [--updated-after T] [--include-archived] [--no-drafts] [--sort relevance\|updated]`, `search --titles Q [--limit N]` | `Page<SearchHit>`; `[{item, path}]` |
| star | `star add ITEM`, `star remove ITEM` | ok |
| doc | `doc get ITEM [--format md\|json] [--title-h1] [--out F]`, `doc put ITEM (--md F\|- \| --json F\|-) (--if-match E \| --force) [--merge] [--title-h1] [--attach-images [--base-dir D]]`, `doc schema` | `{item_id, format, etag, content}`; `DocWriteResult` `{seq, etag, changed, merged, warnings}` |
| board | `board get ITEM [--out F]`, `board put ITEM F\|- (--if-match E \| --force)`, `board preview ITEM [--page N] --out F` | `{item_id, format: "inkboard", etag, content}`; `{rev, etag, updated_at, meta}`; `{path, bytes, sha256, content_type}` |
| attachment | `attachment upload F\|- [--item ITEM] [--filename N]`, `attachment get ID --out F` | `{id, url, sha256, mime, size, width, height}`; `{path, bytes, sha256, content_type}` |
| export, backup | `export --out F` (a tar), `backup` (a file on the server) | `{path, bytes, sha256, content_type}`; `{path, size}` |
| api | `api METHOD PATH [--input F\|-] [--header K:V]… [--out F]` (PATH under `/api/`, no `..`, no other host) | the body unchanged |
| admin | `admin …`: runs `quire-server admin …` (`create-owner`, `create-token`, `list-tokens`, `revoke-token`, `reset-password`, `backup`, `check`) | as the server's admin commands |
| version, help | `version`, `exit-codes`, `help [COMMAND…]` | `{cli, server}`; the table below; the command tree |

```sh
quire --json config show
quire --json collection get "$C"
quire --json tree "$C"
quire --json item move "$ID" --top
quire --json list drafts --all
quire --json trash list
quire --json attachment upload diagram.png --item "$ID"
quire --json export --out export.tar
quire --json api GET /api/v1/sidebar
quire --json version
quire exit-codes --json
quire help --json doc put
```

Content writes need `--if-match ETAG` (`d42`, `"d42"` and `W/"d42"` all work) or `--force`, and `trash purge` needs
`--yes`; without them the command exits 2 instead of guessing. Binary output (`export`, `attachment get`,
`board preview`) needs `--out F` with `--json`.

## 3. Getting a token

A token is `qpat_` plus 46 characters, shown once. Get one in one of these ways:

- the web app: Settings → API tokens;
- on the server host, as the owner, with no token at all (stdout holds only the token):

```sh
umask 077
docker compose exec -T quire quire admin create-token --name agent --scope write --expires 1d > agent.token
```

  (`--token-out /tmp/agent.token` writes it to a 0600 file instead, inside the container);
- with an admin token you already have:

```sh
umask 077; quire --json token create --name agent --scope write --expires 1d | jq -r .token > agent.token
```

Then pass it through the environment or a profile, never as an argument (there is no `--token` flag) or in shell
history:

```sh
export QUIRE_URL=https://wiki.example.org QUIRE_TOKEN_FILE="$PWD/agent.token"
quire --json auth whoami
quire --json auth login --url https://wiki.example.org --token-stdin < agent.token
```

`auth login` saves a profile (`~/.config/quire/config.toml`, 0600; `QUIRE_CONFIG` moves it). The URL and the token are
resolved as a pair: a profile's token is never sent to a URL from somewhere else (that is exit 3), and `--profile P`
ignores `QUIRE_URL`, `QUIRE_TOKEN` and `QUIRE_TOKEN_FILE`. Scopes: `read` (every GET except tokens, sessions and
export), `write` (content and structure), `admin` (tokens, sessions, `me update`, workspace settings, purge, export,
backup). A token never creates an `admin` token, and never one that outlives it: pass a shorter `--expires`.
`auth logout --revoke` revokes the profile's own token at the profile's URL, never a `QUIRE_TOKEN` you exported. For a
LAN install on Caddy's local CA, add `--ca-file quire-local-ca.crt`; plain `http://` works only for this machine (or
with `QUIRE_ALLOW_HTTP=1`).

## 4. Always use `--json`

- stdout is exactly one JSON value: the API's body unchanged (keep unknown fields). A body-less success prints the ok
  object `{"ok": true, "status": 204, "request_id": "…", "id": "…"}`.
- stderr is JSON Lines, so `jq` can read it: zero or more `{"warning": {"code", "message", "details"?}}` lines
  (`truncated`, `attach_skipped`, `api_version_older`, `token_file_permissions`, `discard_failed`, `item_created`,
  `server_unavailable`), `{"debug": {…}}` lines with `--debug` (never a header), and, on failure, a last line
  `{"error": {"code", "message", "details"?}, "request_id", "status", "exit_code"}`. stdout is then empty.
- Branch on the exit code first, then on `error.code` (stable; `message` is for people). Errors the CLI raises itself
  have `request_id: null` and the codes `cli_usage`, `cli_config`, `network`, `timeout`, `tls`, `bad_response` or
  `cli_io`.

```sh
quire --json item get "$ID" 2> err.jsonl || tail -n 1 err.jsonl | jq -r .error.code
```

## 5. Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | Success | Read stdout. |
| 1 | Unexpected error | A CLI bug or an unparseable response: report it, do not retry blindly. |
| 2 | Usage | Fix the command: flags, a missing `--yes`, `--if-match`/`--force`, or a refused `api` path or header. |
| 3 | Configuration | Fix the URL or token: none set, malformed, not bound to the URL, an unsafe profile file, plain HTTP, no CA bundle, a redirect (use the URL the error names), `quire-server` missing, or 421. |
| 4 | Network | If `error.details.sent` is false, retry. If true after a create, list first (the item may exist), then retry. |
| 5 | Not authenticated | The token is unknown, expired or revoked: get a new one. |
| 6 | Forbidden | The token lacks the scope (`details.required`), or the action needs a session: use another token. |
| 7 | Not found | Check the reference; list or search for it. |
| 8 | Conflict | Read `error.code` (`cycle`, `parent_invalid`, `read_only`, `not_empty`, …) and change the request. |
| 9 | Precondition failed | Stale ETag: GET again and redo the change, or resend with `--merge` (documents). |
| 10 | Invalid input | Fix the body: `error.details` names the field, path or limit. |
| 11 | Rate limited | Wait (the CLI already honoured `Retry-After`), then retry more slowly. |
| 12 | Server error | Retry later; report it if it persists (keep `request_id`). |

`quire exit-codes --json` prints the same table.

## 6. A complete session

```sh
export QUIRE_URL=https://wiki.example.org QUIRE_TOKEN_FILE="$PWD/agent.token"
C=$(quire --json collection create --name "Agent scratch" | jq -r .id)
printf '# Plan\n\nFirst step.\n' > plan.md
ID=$(quire --json item create --collection "$C" --kind doc --md plan.md --title-h1 | jq -r .item.id)
ETAG=$(quire --json doc get "$ID" | jq -r .etag)
quire --json doc get "$ID" | jq -j .content > plan.md
printf '\nSecond step.\n' >> plan.md
quire --json doc put "$ID" --md plan.md --if-match "$ETAG"
printf '\nA third step, from another edit.\n' >> plan.md
quire --json doc put "$ID" --md plan.md --if-match "$ETAG"
quire --json doc put "$ID" --md plan.md --if-match "$ETAG" --merge
quire --json search "third step" | jq -r '.items[0].item.title'
quire --json star add "$ID"
quire --json item trash "$ID"
quire --json item restore "$ID"
```

`item create` prints `{"item": {…, "title": "Plan"}, "doc": {"etag": "\"d1\"", …}}`. The first `doc put` prints
`{"seq": 2, "etag": "\"d2\"", "changed": true, "merged": false, "warnings": []}`. The second reuses the old ETag and
exits 9 with `{"error": {"code": "rev_mismatch", "details": {"etag": "\"d2\"", "mergeable": true}}, …}` on stderr.
The third merges (`"merged": true`). `star add` prints the ok object; `item trash` prints `{"trashed": 1}`; `item
restore` brings it back. When done, trash what you created:

```sh
quire --json collection delete "$C"
```

## 7. Markdown, and JSON for exact structure

- Quire Markdown 1 is CommonMark with GFM tables, task lists, strikethrough, autolinks, `==highlight==` and GitHub
  alerts (`> [!NOTE]`); raw HTML stays literal text except `<u>…</u>`.
- The title is not in the body: `--title-h1` takes it from a leading `# Heading` (PUT) or prints it (GET).
- Images are `![alt](/api/v1/attachments/ID)`; `doc put --attach-images` uploads the local images under the file's
  directory (or `--base-dir`) and rewrites their URLs. Item links are `quire://item/<uuid>` or the item's Quire URL.
- Some constructs are lossy (image widths, table spans, empty paragraphs, …): a PUT reports them in `warnings`, a GET
  in the `warnings` list. ARCHITECTURE §6.4 has the table.
- For exact structure, write ProseMirror JSON: `quire doc schema` lists the nodes and marks, `doc get --format json`
  reads it, and `doc put ITEM --json F --if-match E` writes `{"schema_version": 1, "doc": …}` or a bare
  `{"type": "doc", …}`.

```sh
quire --json doc get "$ID" --format json | jq .content > doc.json
quire --json doc put "$ID" --json doc.json --if-match "$ETAG"
```

## 8. Boards

Boards are read and written as the native Inkboard envelope (`{"format": "inkboard", "version": 2, "document": …}`),
never a converted format. Keep every field you do not understand: the server keeps unknown fields, and it owns
`document.name` (the item title). Inkboard's model is described in `vendor/inkboard/docs/ARCHITECTURE.md` ("Model")
and `crates/inkboard-core/src/{model,serial}.rs`. There is no import from draw.io, Excalidraw, Mermaid or other formats
in M1 (D52): build the envelope yourself and PUT it.

```sh
quire --json board get "$B" | jq .content > board.json
quire --json board put "$B" board.json --if-match r3
quire --json board preview "$B" --page 0 --out page0.svg
```

## 9. The agent test harness

`scripts/agent-test.sh` runs CLI scenarios (`e2e/agent/scenarios/NN-*.sh`) against the production image in a
throwaway project (`quire-agent`, only the app, its data in memory). It never builds: run `scripts/image.sh` first.

- `scripts/agent-test.sh [run] [--scenario '03-*'] [--keep] [--allow-stale]`: prints TAP and writes
  `e2e/agent/results/report.json` (every token masked as `qpat_***`).
- `scripts/agent-test.sh up --ttl 10`: leaves the app on `127.0.0.1:18080` (`QUIRE_AGENT_PORT`) for at most 15 minutes
  and writes `.cache/agent-test/env` (`QUIRE_URL`, `QUIRE_TOKEN_FILE`) beside `read.token`, `write.token` and
  `admin.token`; `status` shows the time left; `down` stops it. It holds the build lock meanwhile, so build a host CLI
  with `scripts/cli-dist.sh` before `up`; `QUIRE_AGENT_CLI=dist/cli/quire-x86_64-unknown-linux-musl` makes `run` use it
  (with none of your `QUIRE_*` variables and no profile file).
- Exit codes: 0 every step passed, 1 a scenario failed, 2 usage, 3 a prerequisite is missing or the image is stale
  (rebuild, or `--allow-stale`), 4 the app did not become healthy.

```sh
scripts/image.sh && scripts/agent-test.sh --scenario '0[1-3]-*'
scripts/agent-test.sh up --ttl 10 && set -a && . .cache/agent-test/env && set +a
quire --json sidebar
scripts/agent-test.sh down
```

## 10. Rules for agents

- Build and test only through the wrappers: `scripts/dev.sh` for cargo, node and pnpm, `scripts/image.sh`,
  `scripts/cli-dist.sh`, `scripts/agent-test.sh`. Never run them on the host directly.
- Never print, log or commit a token. Keep it in a 0600 file or the environment; `--debug` never shows headers.
- Work in a scratch collection, and trash what you create when you are done.
- Never purge (`trash purge`) outside the harness; there it only purges the scenario's own items.
