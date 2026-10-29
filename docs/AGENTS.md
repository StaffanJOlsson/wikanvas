# Wikanvas for agents: the `wikanvas` CLI

How an AI agent (or a script) reads and writes a Wikanvas wiki. The contract behind it is docs/ARCHITECTURE.md §7.6
(commands) and §7.7 (output and exit codes); `wikanvas help --json` prints the same command tree.

## 1. What the CLI is and where it runs

`wikanvas` is the command-line client of Wikanvas's HTTP API (`/api/v1`). Every API operation has a command, except a
few web-only ones, which `wikanvas api` still reaches. It never prompts, opens a browser or a pager, or prints colour:
content comes from a file argument or from stdin (`-`).

- **In the image.** The container holds both binaries. There the CLI needs no URL (it derives `http://127.0.0.1:8080`
  from `WIKANVAS_LISTEN`), so `docker compose exec -e WIKANVAS_TOKEN wikanvas wikanvas …` drives the API, and
  `docker compose exec wikanvas wikanvas admin …` runs the server's admin commands without a token.
- **Standalone.** `scripts/cli-dist.sh` builds a static binary, `dist/cli/wikanvas-x86_64-unknown-linux-musl`, for any
  x86_64 Linux (with its `.sha256`, the licences and `THIRD_PARTY.md`).

## 2. Cheat sheet

`ITEM` and `COLL` are a UUID, a `url_id` (10 letters and digits) or a Wikanvas URL (`…/doc/<slug>-<url_id>`). A position
is `--after REF` or `--before REF` (neither: last). Lists take `--limit N`, `--cursor C` and `--all` (follows
`next_cursor` up to 10,000 items, then prints the real `next_cursor` with a `truncated` warning). Items are published on
create; `item unpublish` makes a draft (D23).

| Group | Commands and main flags | `--json` prints |
|---|---|---|
| auth | `auth login --url U (--token-stdin \| --token-file F) [--profile P]`, `auth logout [--revoke]`, `auth whoami` | `Me` (with `auth.token`: prefix, scopes, expiry); logout: the ok object |
| me, session | `me update [--name N] [--email E] [--theme light\|dark\|system]`, `session list`, `session revoke (ID \| --others)` | `{user}`; the list; ok |
| token | `token list`, `token create --name N --scope read\|write\|admin [--expires 30d\|never] [--token-out F]`, `token revoke ID` | `[ApiToken]`; `{token, api_token}` or `{token_file, api_token}`; ok |
| user | `user list`, `user set-role USER --role admin\|member\|viewer`, `user disable USER`, `user enable USER` (`USER`: an id or an email) | `[UserInfo]` `{id, email, name, role, created_at, disabled_at}`; `UserInfo` |
| invite | `invite create --role admin\|member\|viewer [--expires 7d] [--note T] [--url-out F]`, `invite list`, `invite revoke ID` | `{invite, url}` or `{url_file, invite}`; `[Invite]` (pending, never a link); ok |
| config | `config show`, `config path` (local) | `{profile, url, url_source, token_source, token_prefix}`; `{path}` |
| workspace | `workspace get`, `workspace set [--name N] [--logo ATT \| --no-logo]`, `sidebar` | the body |
| collection | `collection list \| get COLL \| create --name N [--description D] [--icon I] [--color '#RRGGBB'] \| update COLL [--sort manual\|title\|updated\|created] \| move COLL \| delete COLL \| restore COLL \| updated COLL` | the body |
| item | `item get ITEM`, `item create --collection COLL --kind doc\|board [--parent ITEM] [--title T] [--md F\|- [--title-h1]]`, `item update ITEM [--title T] [--full-width true\|false] [--if-match E]`, `item move ITEM [--parent ITEM \| --top] [--collection COLL] [--after REF \| --before REF]` (neither `--parent` nor `--top`: the item keeps its parent, a reorder; `--collection` needs one of them), `item duplicate\|publish [--with-ancestors]\|unpublish\|archive\|unarchive\|trash [--discard-empty]\|restore\|children\|text ITEM` | `ItemSummary` (`item get`: `ItemDetail`; `create --md`: `{item, doc}`) |
| tree | `tree COLL` | `{collection, items}` (flat; `parent_id` links them) |
| list, trash | `list recent\|updated\|drafts\|starred\|archive`, `trash list`, `trash purge (ITEM \| --collection COLL \| --all) --yes` | a `Page` `{items, next_cursor}`; the trash; ok |
| search | `search QUERY [--collection COLL] [--kind doc\|board] [--updated-after T] [--include-archived] [--no-drafts] [--sort relevance\|updated]`, `search --titles Q [--limit N]` | `Page<SearchHit>`; `[{item, path}]` |
| star | `star add ITEM`, `star remove ITEM` | ok |
| doc | `doc get ITEM [--format md\|json] [--title-h1] [--out F]`, `doc put ITEM (--md F\|- \| --json F\|-) (--if-match E \| --force) [--merge] [--title-h1] [--attach-images [--base-dir D]]`, `doc schema` | `{item_id, format, etag, content}`; `DocWriteResult` `{seq, etag, changed, merged, warnings}` |
| board | `board get ITEM [--out F]`, `board put ITEM F\|- (--if-match E \| --force)`, `board preview ITEM [--page N] --out F` | `{item_id, format: "inkboard", etag, content}`; `{rev, etag, updated_at, meta}`; `{path, bytes, sha256, content_type}` |
| attachment | `attachment upload F\|- [--item ITEM] [--filename N]`, `attachment get ID --out F` | `{id, url, sha256, mime, size, width, height}`; `{path, bytes, sha256, content_type}` |
| export, backup | `export --out F` (a tar), `backup` (a file on the server) | `{path, bytes, sha256, content_type}`; `{path, size}` |
| api | `api METHOD PATH [--input F\|-] [--header K:V]… [--out F]` (PATH under `/api/`, no `..`, no other host) | the body unchanged |
| admin | `admin …`: runs `wikanvas-server admin …` (`create-owner`, `create-token`, `list-tokens [--email E]`, `revoke-token [--email E]`, `reset-password`, `list-users`, `create-user`, `set-role`, `disable-user`, `enable-user`, `backup`, `check`) | `create-owner`: `{user, workspace, generated_password}`; `create-token`: `{token, api_token}` or `{token_file, api_token}`; `list-tokens`: `[ApiToken]`; `revoke-token`: `{revoked}`; `reset-password`: `{user, password, sessions_revoked, tokens_revoked}`; `list-users`: `[UserInfo]`; `create-user`: `{user, generated_password}`; `set-role`, `enable-user`: `{user}`; `disable-user`: `{user, sessions_revoked, tokens_revoked, invites_revoked}`; `backup`: `{path, size}`; `check`: `{ok, quick_check, foreign_key_violations, cycles, fts, missing_blobs}` (exit 1 when not ok) |
| version, help | `version`, `exit-codes`, `help [COMMAND…]` | `{cli, server}`; the table below; the command tree |

```sh
wikanvas --json config show
wikanvas --json collection get "$C"
wikanvas --json tree "$C"
wikanvas --json item move "$ID" --top
wikanvas --json item move "$ID" --before "$SIBLING"
wikanvas --json list drafts --all
wikanvas --json trash list
wikanvas --json attachment upload diagram.png --item "$ID"
wikanvas --json export --out export.tar
wikanvas --json api GET /api/v1/sidebar
wikanvas --json user list
wikanvas --json user set-role bea@example.org --role viewer
wikanvas --json invite create --role member --expires 7d --url-out invite.url
wikanvas --json version
wikanvas exit-codes --json
wikanvas help --json doc put
```

Content writes need `--if-match ETAG` (`d42`, `"d42"` and `W/"d42"` all work) or `--force`, and `trash purge` needs
`--yes`; without them the command exits 2 instead of guessing. Binary output (`export`, `attachment get`,
`board preview`) needs `--out F` with `--json`.

Accounts join only through invite links (there is no sign-up). `invite create` prints the link once and it works
once: send it to the person, or write it to a 0600 file with `--url-out`, and never log it. `user` and `invite` need an
`admin` token of the owner or of an admin; only the owner invites or makes an admin, nobody changes the owner's role
or their own, and disabling an account ends its sessions, tokens and pending invites at once. Accepting an invite is
web-only (the invite page signs the new person in). Until per-collection permissions arrive (docs/multi-user.md phase
3), every member and viewer can read every collection, and members can change them.

## 3. Getting a token

A token is `qpat_` plus 46 characters, shown once. Get one in one of these ways:

- the web app: Settings → API tokens;
- on the server host, as the owner, with no token at all (stdout holds only the token):

```sh
umask 077
docker compose exec -T wikanvas wikanvas admin create-token --name agent --scope write --expires 1d > agent.token
```

  (`--token-out /tmp/agent.token` writes it to a 0600 file instead, inside the container);
- with an admin token you already have:

```sh
umask 077; wikanvas --json token create --name agent --scope write --expires 1d | jq -r .token > agent.token
```

Then pass it through the environment or a profile, never as an argument (there is no `--token` flag) or in shell
history:

```sh
export WIKANVAS_URL=https://wiki.example.org WIKANVAS_TOKEN_FILE="$PWD/agent.token"
wikanvas --json auth whoami
wikanvas --json auth login --url https://wiki.example.org --token-stdin < agent.token
```

`auth login` saves a profile (`~/.config/wikanvas/config.toml`, 0600; `WIKANVAS_CONFIG` moves it). The URL and the token
are resolved as a pair: a profile's token is never sent to a URL from somewhere else (that is exit 3), and `--profile P`
ignores `WIKANVAS_URL`, `WIKANVAS_TOKEN` and `WIKANVAS_TOKEN_FILE`. Scopes: `read` (every GET except tokens, sessions
and export), `write` (content and structure), `admin` (tokens, sessions, `me update`, workspace settings, accounts and
invites, purge, export, backup). A token is also limited by its account's role: a `member`'s token never purges, a
`viewer`'s never writes, whatever its scope. A token never creates an `admin` token, and never one that outlives it:
pass a shorter `--expires`.
`auth logout --revoke` revokes the profile's own token at the profile's URL, never a `WIKANVAS_TOKEN` you exported. For
a LAN install on Caddy's local CA, add `--ca-file wikanvas-local-ca.crt`; plain `http://` works only for this machine
(or with `WIKANVAS_ALLOW_HTTP=1`).

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
wikanvas --json item get "$ID" 2> err.jsonl || tail -n 1 err.jsonl | jq -r .error.code
```

## 5. Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | Success | Read stdout. |
| 1 | Unexpected error | A CLI bug or an unparseable response: report it, do not retry blindly. |
| 2 | Usage | Fix the command: flags, a missing `--yes`, `--if-match`/`--force`, or a refused `api` path or header. |
| 3 | Configuration | Fix the URL or token: none set, malformed, not bound to the URL, an unsafe profile file, plain HTTP, no CA bundle, a redirect (use the URL the error names), `wikanvas-server` missing, or 421. For `admin create-owner`: the data directory holds files but no owner; follow the message (put the wiki's database in place, or move the files away). |
| 4 | Network | If `error.details.sent` is false, retry. If true after a create, list first (the item may exist), then retry. |
| 5 | Not authenticated | The token is unknown, expired or revoked: get a new one. |
| 6 | Forbidden | The token lacks the scope (`details.required`), the action needs a session, or the account's role does not allow it (`forbidden`): use another token or account. |
| 7 | Not found | Check the reference; list or search for it. |
| 8 | Conflict | Read `error.code` (`cycle`, `parent_invalid`, `read_only`, `not_empty`, …) and change the request. |
| 9 | Precondition failed | Stale ETag: GET again and redo the change, or resend with `--merge` (documents). |
| 10 | Invalid input | Fix the body: `error.details` names the field, path or limit. |
| 11 | Rate limited | Wait (the CLI already honoured `Retry-After`), then retry more slowly. |
| 12 | Server error | Retry later; report it if it persists (keep `request_id`). |

`wikanvas exit-codes --json` prints the same table.

## 6. A complete session

```sh
export WIKANVAS_URL=https://wiki.example.org WIKANVAS_TOKEN_FILE="$PWD/agent.token"
C=$(wikanvas --json collection create --name "Agent scratch" | jq -r .id)
printf '# Plan\n\nFirst step.\n' > plan.md
ID=$(wikanvas --json item create --collection "$C" --kind doc --md plan.md --title-h1 | jq -r .item.id)
ETAG=$(wikanvas --json doc get "$ID" | jq -r .etag)
wikanvas --json doc get "$ID" | jq -j .content > plan.md
printf '\nSecond step.\n' >> plan.md
wikanvas --json doc put "$ID" --md plan.md --if-match "$ETAG"
printf '\nA third step, from another edit.\n' >> plan.md
wikanvas --json doc put "$ID" --md plan.md --if-match "$ETAG"
wikanvas --json doc put "$ID" --md plan.md --if-match "$ETAG" --merge
wikanvas --json search "third step" | jq -r '.items[0].item.title'
wikanvas --json star add "$ID"
wikanvas --json item trash "$ID"
wikanvas --json item restore "$ID"
```

`item create` prints `{"item": {…, "title": "Plan"}, "doc": {"etag": "\"d1\"", …}}`. The first `doc put` prints
`{"seq": 2, "etag": "\"d2\"", "changed": true, "merged": false, "warnings": []}`. The second reuses the old ETag and
exits 9 with `{"error": {"code": "rev_mismatch", "details": {"etag": "\"d2\"", "mergeable": true}}, …}` on stderr.
The third merges (`"merged": true`). `star add` prints the ok object; `item trash` prints `{"trashed": 1}`; `item
restore` brings it back. When done, trash what you created:

```sh
wikanvas --json collection delete "$C"
```

## 7. Markdown, and JSON for exact structure

- Wikanvas Markdown 1 is CommonMark with GFM tables, task lists, strikethrough, autolinks, `==highlight==` and GitHub
  alerts (`> [!NOTE]`); raw HTML stays literal text except `<u>…</u>`.
- The title is not in the body: `--title-h1` takes it from a leading `# Heading` (PUT) or prints it (GET).
- Images are `![alt](/api/v1/attachments/ID)`; `doc put --attach-images` uploads the local images under the file's
  directory (or `--base-dir`) and rewrites their URLs. Item links are `wikanvas://item/<uuid>` or the item's Wikanvas
  URL.
- Some constructs are lossy (image widths, table spans, empty paragraphs, …): a PUT reports them in `warnings`, a GET
  in the `warnings` list. ARCHITECTURE §6.4 has the table.
- For exact structure, write ProseMirror JSON: `wikanvas doc schema` lists the nodes and marks, `doc get --format json`
  reads it, and `doc put ITEM --json F --if-match E` writes `{"schema_version": 1, "doc": …}` or a bare
  `{"type": "doc", …}`.

```sh
wikanvas --json doc get "$ID" --format json | jq .content > doc.json
wikanvas --json doc put "$ID" --json doc.json --if-match "$ETAG"
```

## 8. Boards

Boards are read and written as the native Inkboard envelope (`{"format": "inkboard", "version": 2, "document": …}`),
never a converted format. Keep every field you do not understand: the server keeps unknown fields, and it owns
`document.name` (the item title). Inkboard's model is described in `vendor/inkboard/docs/ARCHITECTURE.md` ("Model")
and `crates/inkboard-core/src/{model,serial}.rs`. There is no import from draw.io, Excalidraw, Mermaid or other formats
in M1 (D52): build the envelope yourself and PUT it.

```sh
wikanvas --json board get "$B" | jq .content > board.json
wikanvas --json board put "$B" board.json --if-match r3
wikanvas --json board preview "$B" --page 0 --out page0.svg
```

### Embedded pages

A board can show documents: each one is a **card**, an ordinary Inkboard box with `"geom": "rect"` whose `kind` holds
`"embed": {"item": "<document id>"}`. The web client draws the document's content over the box and edits it in place
(double-click); the document stays an ordinary item, in the tree and in search, openable on its own, and one document
can be on several boards. A card as the web client writes it (pretty-printed here; the engine writes it compact, in
this field order):

```json
{
  "id": "k3j2h1g0f9",
  "kind": {
    "type": "box",
    "geom": "rect",
    "frame": { "x": -240.0, "y": -180.0, "w": 480.0, "h": 360.0 },
    "label": {
      "text": [ { "text": "Meeting notes", "bold": true, "color": "#1d2430" } ],
      "family": "sans",
      "size": 24.0,
      "align": "center"
    },
    "embed": { "item": "0192f0e0-0000-7000-8000-000000000001" }
  },
  "style": { "stroke": "#c4cad4", "fill": "#ffffff", "width": 1.0, "dash": "solid", "opacity": 1.0, "roundness": 0.04, "shadow": true },
  "layer": "main"
}
```

- **What is a card**: an object whose `kind.type` is `"box"`, `kind.geom` is `"rect"`, and `kind.embed` is a JSON object
  with a string `item`. Anything else (an `embed` on an ellipse, `"embed": 5`, an object without `item`) is a plain
  box: the server keeps it byte for byte and links nothing, and the web client's engine drops the malformed `embed`
  the next time it saves the board. No value of `embed` is refused; only a box `kind` that holds the key `embed` twice
  is (exit 10, `invalid_board`).
- **`item`** is the document's id, a UUID in canonical form (36 characters, lowercase, with hyphens: `item get`'s
  `.item.id`). Any other string is kept but shows "Page not found". Only documents (`kind: "doc"`) are shown; a board
  id shows "Page not found" too. `embed` holds only `item`: other fields in it are dropped by the engine.
- **The label is a title snapshot**: one bold span coloured `#1d2430` (sans, size 24, centred) holding the document's
  title without leading and trailing spaces, `"Untitled"` when nothing else is left (as the web client writes it; it
  also cuts a title longer than 200 characters to its first 200). Previews (`board preview`), the board's search text
  (`item text`, `search`) and engines without Wikanvas's patch show it; the web client shows the live title and
  refreshes the snapshot when it next saves the board. Searching for a document's title also finds the boards it is on;
  a board's text never holds the documents' bodies (each document is its own search hit).
- **Keep `embed`** when you rewrite a board through your own model of Inkboard's JSON: a model without the field drops
  it, and the card becomes a plain box. Edit the JSON (as with `jq` below) rather than rebuild it.
- **Object ids** must be unique on their page: the engine keeps the first object with a given id and drops the rest.
- New cards are 480 × 360 world units, centred where they are placed, on the page's first visible, unlocked layer.
  Any size works; the web client draws the content once the card is large enough on screen.
- **Documents and boards stay separate items.** Trashing, restoring, archiving, moving or purging a board never
  touches a document it shows, except its own children ("Text doc" in the web client makes the new document a child of
  the board), which follow the board as any child does. `item duplicate` of a board copies its cards, not its
  children, so the copy's cards show the original's documents: trashing or purging the original then trashes or
  deletes its children for the copy too (move them out of the board first to keep them). Deleting a card never deletes
  its document, and the server keeps every document that a card shows even when it is new and empty:
  `item trash --discard-empty` then exits 8 (`not_empty`).

A card for a new child document of the board `$B` (in collection `$C`), with Markdown from `notes.md`:

```sh
DOC=$(wikanvas --json item create --collection "$C" --kind doc --parent "$B" --title "Meeting notes" --md notes.md | jq -r .item.id)
```

Then append a card for the document `$DOC` to page 0 of `$B`, to the right of the boxes already there (the same
commands work for any existing document):

<!-- e2e/agent/scenarios/12-board-embeds.sh runs the next block as it is written: keep it a complete recipe. -->
```sh
wikanvas --json board get "$B" > got.json
TITLE=$(wikanvas --json item get "$DOC" | jq -r .item.title)
jq --arg doc "$DOC" --arg title "$TITLE" '
  .content
  | .document.pages[0] as $page
  | ([$page.objects[].kind.frame | select(. != null) | .x + .w] | max // -280) as $right
  | ($title | sub("^\\s+"; "") | sub("\\s+$"; "")) as $label
  | .document.pages[0].objects += [{
      id: "card-\(now * 1000000 | floor)",
      kind: {
        type: "box",
        geom: "rect",
        frame: {x: ($right + 40), y: -180, w: 480, h: 360},
        label: {
          text: [{text: (if $label == "" then "Untitled" else $label end), bold: true, color: "#1d2430"}],
          family: "sans", size: 24, align: "center"
        },
        embed: {item: $doc}
      },
      style: {stroke: "#c4cad4", fill: "#ffffff", width: 1, dash: "solid", opacity: 1, roundness: 0.04, shadow: true},
      layer: ([$page.layers[] | select(.visible != false and .locked != true) | .id] | first // $page.layers[0].id // "main")
    }]' got.json > board.json
wikanvas --json board put "$B" - --if-match "$(jq -r .etag got.json)" < board.json
```

Exit 9 (`rev_mismatch`) means the board changed since the `board get`: run the recipe again. To remove a card, delete
its object the same way (`.document.pages[0].objects |= map(select(.kind.embed | type == "object" and .item == $doc | not))`);
the document stays.

## 9. The agent test harness

`scripts/agent-test.sh` runs CLI scenarios (`e2e/agent/scenarios/NN-*.sh`) against the production image in a
throwaway project (`wikanvas-agent`, only the app, its data in memory). It never builds: run `scripts/image.sh` first.

- `scripts/agent-test.sh [run] [--scenario '03-*'] [--keep] [--allow-stale]`: prints TAP and writes
  `e2e/agent/results/report.json` (every token masked as `qpat_***`).
- `scripts/agent-test.sh up --ttl 10`: leaves the app on `127.0.0.1:18080` (`WIKANVAS_AGENT_PORT`) for at most 15
  minutes and writes `.cache/agent-test/env` (`WIKANVAS_URL`, `WIKANVAS_TOKEN_FILE`) beside `read.token`, `write.token`
  and `admin.token`; `status` shows the time left; `down` stops it. It holds the build lock meanwhile, so build a host
  CLI with `scripts/cli-dist.sh` before `up`; `WIKANVAS_AGENT_CLI=dist/cli/wikanvas-x86_64-unknown-linux-musl` makes
  `run` use it (with none of your `WIKANVAS_*` variables and no profile file).
- Exit codes: 0 every step passed, 1 a scenario failed, 2 usage, 3 a prerequisite is missing or the image is stale
  (rebuild, or `--allow-stale`), 4 the app did not become healthy.

```sh
scripts/image.sh && scripts/agent-test.sh --scenario '0[1-3]-*'
scripts/agent-test.sh up --ttl 10 && set -a && . .cache/agent-test/env && set +a
wikanvas --json sidebar
scripts/agent-test.sh down
```

## 10. Rules for agents

- Build and test only through the wrappers: `scripts/dev.sh` for cargo, node and pnpm, `scripts/image.sh`,
  `scripts/cli-dist.sh`, `scripts/agent-test.sh`. Never run them on the host directly.
- Never print, log or commit a token. Keep it in a 0600 file or the environment; `--debug` never shows headers.
- Work in a scratch collection, and trash what you create when you are done.
- Never purge (`trash purge`) outside the harness; there it only purges the scenario's own items.
