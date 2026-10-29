# Wikanvas

**An open-source, self-hosted wiki where documents and whiteboards live in one tree.** A self-hostable alternative to
Confluence and Miro: a sidebar of collections holding rich-text documents and infinite
[Inkboard](https://github.com/VictorLetirand/inkboard) whiteboards, nested to any depth.

> [!WARNING]
> **Early alpha.** Wikanvas is days old. Expect bugs, missing features and breaking changes between versions,
> including to how data is stored. Don't use it for anything you can't afford to lose yet, and keep backups (it makes
> them for you, daily).

> [!NOTE]
> Wikanvas was called Quire until v0.1.0-alpha.1, when it was renamed because of existing trademarks on that name.

The server is Rust (axum) on SQLite; the frontend is React with the TipTap editor; boards run Inkboard compiled to
WebAssembly. A board can hold documents as cards: each is a real page of the tree, edited in place on the board
with a double-click (§18). It has a versioned HTTP API (`/api/v1`, with personal access tokens and an OpenAPI 3.1 spec) and a
command-line client, `wikanvas`.

**Status:** milestone 1, single owner. The design is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (cited below
as §n) and the implementation plan in [`docs/PLAN.md`](docs/PLAN.md). Wikanvas is not affiliated with Atlassian,
Miro or Canva.

Contents: [Install](#install) · [Configuration](#configuration) · [Runtime limits](#runtime-limits) ·
[HTTPS](#https) · [Ports 80 or 443 taken](#when-port-80-or-443-is-taken) ·
[Behind an existing reverse proxy](#behind-an-existing-reverse-proxy) · [Backups](#backups-and-restore) ·
[Admin commands](#admin-commands) · [The API](#the-api) · [The CLI](#the-cli) · [Building](#building-and-the-build-caps)
· [Licence](#licence)

## Install

Wikanvas runs as two containers in one compose project: the app (`wikanvas`) and [Caddy](https://caddyserver.com), its
HTTPS front. The app has no published port; Caddy publishes 80 and 443 and terminates TLS (D57).

You need Linux on x86_64, Docker Engine with the Compose v2 plugin and buildx (the test overlays need Compose 2.24 or
later), git, and disk space for the build cache (the builder's garbage collection keeps it under 30 GB). Nothing is compiled on the host: every build runs in
resource-capped containers (see [Building](#building-and-the-build-caps)).

1. Get the source, with the Inkboard submodule:

   ```sh
   git clone --recurse-submodules <repository> wikanvas && cd wikanvas
   # or, in an existing clone: git submodule update --init vendor/inkboard
   ```

2. Build the image. `scripts/image.sh` creates the capped builder `wikanvas-limited` the first time (a BuildKit
   container with 6 CPUs and 12 GB), then builds the web client, the Inkboard engine, the server and the CLI and tags
   `wikanvas:<commit>` and `wikanvas:latest`. The first build takes a while; later ones reuse the cache.

   ```sh
   scripts/image.sh
   ```

3. Optionally write a `.env` next to `compose.yaml` ([Configuration](#configuration)). Without one, Wikanvas serves
   `https://localhost` on this machine only, with certificates from Caddy's local CA.

4. Start it:

   ```sh
   docker compose up -d
   ```

5. Trust Caddy's local CA once ([Trusting the local CA](#trusting-the-local-ca)), then open `https://localhost` (or
   `https://<WIKANVAS_DOMAIN>`, with `:<WIKANVAS_HTTPS_PORT>` when that is not 443).

6. Set up the owner account. Until an owner exists, the server prints a one-time setup code in its log:

   ```sh
   docker compose logs wikanvas | grep 'First-run setup'
   # … First-run setup: https://localhost/setup code ABCD-EFGH-JKMN
   ```

   Open the link, enter the code, the workspace name, your name, email and a password (12 to 1024 bytes). Wikanvas seeds
   a "Getting started" collection. Instead of the web form you can create the owner from the shell:

   ```sh
   docker compose exec -T wikanvas wikanvas admin create-owner --email you@example.org --name "Your Name" \
     --workspace "My wiki" --password-stdin < password.txt
   # or --generate-password: prints a password once and asks for a new one at the first sign-in
   ```

Upgrading is `git pull`, `git submodule update`, `scripts/image.sh` and `docker compose up -d`. The server migrates
the database at startup and writes a pre-migration backup first (§12).

## Configuration

Compose reads the variables below from `.env` next to `compose.yaml` (or from the shell). Every one is optional, and an
empty value counts as unset. A malformed value stops the server with `VAR: message` in `docker compose logs wikanvas`.

| Variable | Default | Meaning |
|---|---|---|
| `WIKANVAS_DOMAIN` | `localhost` | The name Wikanvas is served under. Caddy serves only this name; Wikanvas allows it as `Host` with any port and derives the public URL from it |
| `WIKANVAS_TLS` | `internal` | `internal`: Caddy's local CA (localhost or a LAN name). `acme`: public certificates; Wikanvas then also sends HSTS ([HTTPS](#https)) |
| `WIKANVAS_BIND` | `127.0.0.1` | The host address Caddy's ports bind to: `127.0.0.1` for this machine only, `0.0.0.0` for the LAN (required for `acme`) |
| `WIKANVAS_HTTP_PORT`, `WIKANVAS_HTTPS_PORT` | `80`, `443` | The host ports Caddy publishes. Change them only when 80 or 443 are taken ([below](#when-port-80-or-443-is-taken)) |
| `WIKANVAS_PUBLIC_URL` | derived | `https://<WIKANVAS_DOMAIN>`, plus `:<WIKANVAS_HTTPS_PORT>` unless 443. Set it (`scheme://host[:port]`, no path) only when clients reach Wikanvas under another URL, for example behind your own proxy |
| `WIKANVAS_ALLOWED_HOSTS` | none | More `Host` values to accept, comma-separated `host` or `host:port`, for clients that reach Wikanvas under another name, such as another container on the compose network (`wikanvas`) |
| `WIKANVAS_BACKUP_INTERVAL` | `24h` | Scheduled backups: a number with `s`, `m`, `h` or `d` |
| `WIKANVAS_BACKUP_KEEP` | `7` | How many `wikanvas-*.db` backups to keep; `0` turns scheduled backups off |
| `WIKANVAS_MAX_UPLOAD_MB` | `25` | Largest attachment upload |
| `WIKANVAS_MAX_BOARD_MB` | `50` | Largest board save (the web client warns at 15 MB) |
| `WIKANVAS_TOKEN_RATE` | `600` | Requests per minute per API token (burst 120) |
| `WIKANVAS_TOKEN_BYTES_PER_HOUR` | `1073741824` | Request-body bytes per API token per hour |
| `WIKANVAS_MIN_FREE_MB` | `1024` | The free-space floor on `/data`, capped at 10 % of the filesystem: below it, uploads, board saves, export and backups answer 507 |
| `WIKANVAS_CORS_ORIGINS` | none | Exact origins (`https://tool.example.org`, comma-separated) whose browser pages may call the API with a token. `*` is refused; cookies never work cross-origin |
| `WIKANVAS_CPUS`, `WIKANVAS_MEM` | `2`, `1g` | The app's CPU and memory caps ([Runtime limits](#runtime-limits)) |
| `WIKANVAS_CADDY_CPUS`, `WIKANVAS_CADDY_MEM` | `0.5`, `128m` | Caddy's caps |
| `WIKANVAS_NET_SUBNET`, `WIKANVAS_NET_RANGE`, `WIKANVAS_PROXY_IP` | `172.31.240.0/28`, `172.31.240.8/29`, `172.31.240.2` | The project network and Caddy's fixed address. Change all three together, only if the subnet collides with another network: the proxy address must lie inside the subnet and outside the range |
| `WIKANVAS_IMAGE` | `wikanvas:latest` | The image to run, for example `wikanvas:<commit>` to go back to an earlier build |
| `RUST_LOG` | `info` | The server's log filter |

`compose.yaml` passes no other variable to the app. The server reads a few more (`WIKANVAS_SETUP_TOKEN`, a fixed setup
code; `WIKANVAS_MAX_DOC_MB`, default 16, the largest stored document; `WIKANVAS_SYNC=FULL`, SQLite's `synchronous`); set
those in a `compose.override.yaml` under `services.wikanvas.environment` if you need them. `WIKANVAS_TRUSTED_PROXIES`
comes from `WIKANVAS_PROXY_IP` (Caddy's address); see [Behind an existing reverse
proxy](#behind-an-existing-reverse-proxy).

## Runtime limits

Both containers are capped in `compose.yaml`, and neither may swap (`memswap_limit` equals `mem_limit`):

| Container | CPUs | Memory | Other |
|---|---|---|---|
| `wikanvas` | `WIKANVAS_CPUS=2` | `WIKANVAS_MEM=1g` | 256 processes, read-only root filesystem, every capability dropped, a 32 MB `/tmp` |
| `caddy` | `WIKANVAS_CADDY_CPUS=0.5` | `WIKANVAS_CADDY_MEM=128m` | 128 processes, read-only root filesystem, only `NET_BIND_SERVICE` |

Change them in `.env` and run `docker compose up -d` again. The defaults are deliberately small, and the server's
memory budget is built for 1 GiB (D25): the board limit, the number of documents loaded at once and the thread pools
(2 worker threads, `WIKANVAS_WORKER_THREADS` in `compose.yaml`) are sized for it. More memory adds headroom; it does not
raise any limit by itself.

## HTTPS

Caddy terminates HTTPS in front of Wikanvas (`docker/Caddyfile`, §13.4). It forwards the client's address and scheme to
Wikanvas, passes Wikanvas's precompressed responses through untouched, and adds no headers of its own: every security
header, HSTS included, comes from Wikanvas.

### Local or LAN (default: `WIKANVAS_TLS=internal`)

Caddy signs certificates with its own local CA, kept in the `caddy-data` volume (the root is valid for ten years; leaf
certificates renew by themselves). Use it for `localhost` or a LAN name:

```sh
# .env for a LAN name: the LAN's DNS (or each device's /etc/hosts) must resolve wikanvas.lan to this host
WIKANVAS_DOMAIN=wikanvas.lan
WIKANVAS_BIND=0.0.0.0
```

Wikanvas sends **no HSTS** in this mode: HSTS on `localhost` or a private name would pin every device to HTTPS for a
year, including devices that do not trust the root yet.

### Trusting the local CA

Browsers and tools refuse the local certificates until they trust the root. Copy it out once per CA (a new
`caddy-data` volume means a new root, which every device has to trust again):

```sh
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt ./wikanvas-local-ca.crt
```

- **This machine's system bundle** (curl, the `wikanvas` CLI, most tools; Debian and Ubuntu):
  `sudo install -m 0644 wikanvas-local-ca.crt /usr/local/share/ca-certificates/ && sudo update-ca-certificates`.
- **Chrome and Chromium** read their own NSS store (per user, no root; `certutil` is in `libnss3-tools`):
  `certutil -d "sql:$HOME/.pki/nssdb" -A -t "C,," -n "Wikanvas local CA" -i wikanvas-local-ca.crt`.
- **Firefox:** Settings → Privacy & Security → Certificates → View Certificates → Authorities → Import, and tick
  "Trust this CA to identify websites".
- **Other devices** import the same `root.crt` (on phones, as a CA profile or certificate).
- **The `wikanvas` CLI** uses the system bundle once the root is in it; otherwise pass `--ca-file wikanvas-local-ca.crt`
  (or set `WIKANVAS_CA_FILE`, or `ca_file` in its profile).

Only `root.crt` ever leaves the volume. **Keep `root.key` secret** (it stays in `caddy-data`): whoever holds it can
impersonate any site to every device that trusts the root.

### Public (`WIKANVAS_TLS=acme`)

```sh
WIKANVAS_TLS=acme
WIKANVAS_DOMAIN=wiki.example.org
WIKANVAS_BIND=0.0.0.0
```

The domain needs a DNS A record to this host, and ports 80 and 443 must be reachable from the Internet: Caddy gets
Let's Encrypt certificates through its automatic HTTPS and keeps them, with the ACME account, in `caddy-data` (keep the
volume: ACME has rate limits). In this mode Wikanvas sends `Strict-Transport-Security: max-age=31536000` on every HTTPS
response (no `includeSubDomains`, no `preload`).

### The `caddy-data` volume

`caddy-data` holds the local CA's root and private key, the certificates and the ACME account. Back it up like a
secret ([Backups](#backups-and-restore)). **`docker compose down -v` deletes it**, and with it the local CA every
device trusts, as well as `wikanvas-data`, the wiki itself. Use `docker compose down` (without `-v`) to stop the
project.

### When port 80 or 443 is taken

`docker compose up` fails with "address already in use" when another web server or proxy holds port 80 or 443 on
the host. There are two ways out.

**Other host ports.** Let Caddy publish other ports:

```sh
# .env
WIKANVAS_HTTPS_PORT=8443
WIKANVAS_HTTP_PORT=8088        # only if 80 is taken too
```

Then open `https://localhost:8443`. Wikanvas's derived public URL includes the port (`https://localhost:8443`), and its
`Host` check accepts `WIKANVAS_DOMAIN` with any port. Caddy's plain-HTTP redirect still names port 443, so always use
the HTTPS URL with its port. In `acme` mode the certificate authority still connects to ports 80 and 443 of your public
address: forward them on the router to `WIKANVAS_HTTP_PORT` and `WIKANVAS_HTTPS_PORT`, or run Wikanvas behind the proxy
that already owns them (next section).

**Your own proxy in front.** If a reverse proxy already serves 443 for other sites, let it serve Wikanvas too, and do
not start the bundled Caddy.

### Behind an existing reverse proxy

Wikanvas trusts `X-Forwarded-For` and `X-Forwarded-Proto` only from the addresses in `WIKANVAS_TRUSTED_PROXIES`. In the
bundled setup that is exactly Caddy's fixed address, so a client that reaches Wikanvas any other way can neither choose
its rate-limit key nor claim HTTPS. A deployment that swaps Caddy for another proxy **must** set
`WIKANVAS_TRUSTED_PROXIES` to the address Wikanvas sees that proxy connect from: every per-IP limit (login, setup,
failed token attempts), the access log, the Secure session cookie and HSTS depend on it. Without it every request looks
like plain HTTP from one address.

1. Create `compose.override.yaml` next to `compose.yaml` (`docker compose` merges it automatically):

   ```yaml
   services:
     wikanvas:
       ports: ["127.0.0.1:8085:8080"]            # loopback only: your proxy connects here
       environment:
         WIKANVAS_DOMAIN: wiki.example.org          # the name your proxy serves; Wikanvas allows it as Host
         WIKANVAS_TLS: acme                         # HSTS on HTTPS requests; keep internal for a private name
         WIKANVAS_TRUSTED_PROXIES: 172.31.240.8/29  # where the proxy's connections appear to come from (step 3)
     caddy:
       profiles: [bundled-caddy]                 # not started unless asked for
   ```

   `WIKANVAS_DOMAIN` here sets the public URL, `https://wiki.example.org`; if your proxy serves another port or scheme,
   set `WIKANVAS_PUBLIC_URL` too.

2. Point the proxy at `127.0.0.1:8085`. It must keep the client's `Host`, send `X-Forwarded-For` (Wikanvas takes the
   rightmost address) and `X-Forwarded-Proto: https`, and must not compress or decompress responses (Wikanvas sends
   precompressed br and gzip). For a Caddy you already run:

   ```caddyfile
   wiki.example.org {
   	reverse_proxy 127.0.0.1:8085 {
   		transport http {
   			compression off
   		}
   	}
   }
   ```

   For nginx: `proxy_set_header Host $host; proxy_set_header X-Forwarded-For $remote_addr; proxy_set_header
   X-Forwarded-Proto $scheme;`, `client_max_body_size 64m;` (boards up to 50 MB), no `gzip` on that location, and a
   long `proxy_read_timeout` for exports. A proxy that adds HTTP Basic auth, or forwards its own `Authorization: Bearer`
   header (oauth2-proxy), keeps working: Wikanvas ignores any `Authorization` value that is not a Wikanvas token (D43).

3. Start only the app with `docker compose up -d`, send one request through the proxy, and check which address Wikanvas
   saw:

   ```sh
   docker compose logs wikanvas | grep -o 'client_ip=[^ ]*' | tail -n 3
   ```

   It should be the real client's address. If it is an address of the compose network instead, Wikanvas did not trust
   the proxy: set `WIKANVAS_TRUSTED_PROXIES` to that address (or the network's range) and restart. With a loopback port
   as above, Docker hands the proxy's connections to Wikanvas from inside the project network (on Docker 29 from an
   address in `WIKANVAS_NET_RANGE`, `172.31.240.8/29` by default), so trusting that range means: any process on this
   host that can reach `127.0.0.1:8085` may set `X-Forwarded-*`. A proxy that runs as a container can join the compose
   network instead (`wikanvas_default`), with a fixed address outside the range, and proxy to `wikanvas:8080`; trust
   that one address.

## Backups and restore

Everything lives in the `wikanvas-data` volume, mounted at `/data`: `wikanvas.db` (SQLite, WAL), `files/` (attachments
and board images, content-addressed), `backups/` and `tmp/`.

- **Scheduled backups** run every `WIKANVAS_BACKUP_INTERVAL` (24 h) and keep `WIKANVAS_BACKUP_KEEP` (7) copies as
  `/data/backups/wikanvas-YYYYMMDDTHHMMZ.db`, made with SQLite's `VACUUM INTO` and checked with `quick_check`. The first
  one after a start is due an interval after the newest existing backup, and at least a minute after the start, so
  frequent restarts never postpone it. `WIKANVAS_BACKUP_KEEP=0` turns them off.
- **On demand:** Settings → Data → "Back up now", or `docker compose exec wikanvas wikanvas admin backup` (`--out PATH`
  writes elsewhere inside the container, under `/data`).
- **A backup holds the database only.** Attachments and board images are in `/data/files`. Pre-migration backups
  (`pre-migrate-v*.db`) are kept apart from the count.

**Keep an off-host copy.** `/data/backups` is on the same disk as the wiki; a lost disk takes both. Copy the backups and
the files out regularly, for example:

```sh
docker compose cp wikanvas:/data/backups ./wikanvas-backup/backups
docker compose cp wikanvas:/data/files   ./wikanvas-backup/files
docker compose cp caddy:/data            ./wikanvas-backup/caddy-data   # the CA key and ACME account: keep it private
```

(or point restic or rsync at the volumes). An **export** (Settings → Data → Export, `wikanvas export --out F`, or
`GET /api/v1/export` with an admin token) is a tar of `manifest.json`, a copy of `wikanvas.db` and `files/`, without
sessions, API tokens or password hashes.

**Restore a backup.** Stop the app, put the backup in place as `wikanvas.db`, delete the WAL files, start again. The
image has no shell, so use a throwaway container on the volume (the volume is `<project>_wikanvas-data`,
`wikanvas_wikanvas-data` by default):

```sh
docker compose stop wikanvas
docker run --rm -v wikanvas_wikanvas-data:/data --user 65532:65532 busybox \
  sh -c 'cp /data/backups/wikanvas-20261028T0300Z.db /data/wikanvas.db && rm -f /data/wikanvas.db-wal /data/wikanvas.db-shm'
docker compose start wikanvas
```

If `/data/files` was lost too, copy the off-host `files/` back in first.

**Restore an export.** Unpack the tar into the stopped volume: `wikanvas.db` into `/data` and `files/` as `/data/files`
(same throwaway container, mounting the unpacked directory read-only). An export has no password hashes and no tokens,
so set a password afterwards and create new tokens:

```sh
docker compose exec wikanvas wikanvas admin reset-password --email you@example.org
docker compose exec wikanvas wikanvas admin create-token --name cli --scope write --expires 30d
```

**Data from before v0.1.0-alpha.1.** A data volume, backup or export made before the rename (the note at the top)
keeps its database under the old name, which no later release reads. Put that file in place as `wikanvas.db` before the
first start: in a volume, rename the old `.db` file together with its `-wal` and `-shm` files; from a backup or an
export, copy its `.db` file as above. A server that finds files in `/data/files` but no owner (no `wikanvas.db`, or
an empty one that an earlier start created) refuses to start, before it shows the first-run setup, and leaves both
untouched: the app keeps restarting, and `docker compose logs wikanvas` shows an error with the database path and the
number of files. `wikanvas admin create-owner` refuses there too (exit 3, the same error), creating neither a database
nor an owner. Either put the right `wikanvas.db` in place (over an empty one: delete its `-wal` and `-shm` files too),
or, to start a new wiki on purpose, move `/data/files` away. The cleanup that deletes files the database does not list
(at startup and every 6 h) never runs while there is no owner. Old backups in `/data/backups` are neither listed nor
pruned under the old name: rename them to `wikanvas-<time>.db` to have them counted.

**Bind mounts.** The app runs as uid 65532. Named volumes take that ownership from the image; if you replace
`wikanvas-data` with a bind mount, `chown -R 65532:65532` the host directory first.

## Admin commands

`wikanvas-server admin …` works directly on the database, with no token, and may run while the server is up. Reach it
through the CLI in the container (`--json` gives machine-readable output; exit codes as in docs/AGENTS.md):

```sh
docker compose exec wikanvas wikanvas admin reset-password --email you@example.org    # prints a temporary password
docker compose exec wikanvas wikanvas admin create-owner --email … --name … --workspace … --generate-password
docker compose exec -T wikanvas wikanvas admin create-token --name ci --scope write --expires 1d > ci.token
docker compose exec wikanvas wikanvas admin list-tokens
docker compose exec wikanvas wikanvas admin revoke-token <id>                       # and the tokens it created
docker compose exec wikanvas wikanvas admin backup
docker compose exec wikanvas wikanvas admin check                                   # integrity, FTS, missing files
```

`reset-password` sets a random temporary password, prints it once, requires a new one at the next sign-in and signs out
every session. `create-token` prints the token once (`--token-out F` writes it to a 0600 file inside the container);
`--expires` is `<days>d` or `never`, 90 days by default.

**If a password or a token may be compromised:**

```sh
docker compose exec wikanvas wikanvas admin reset-password --email you@example.org --revoke-tokens   # also every token
docker compose exec wikanvas wikanvas admin revoke-token --all --yes                                # every token, now
```

A password change in the web client signs out the other sessions but leaves tokens alone; they are separate
credentials.

## The API

Everything the web client does goes through the public, versioned API under `/api/v1` (D39); there are no private
routes. Within v1, changes are additive only.

- **Authentication:** the session cookie (the browser), or a personal access token: `Authorization: Bearer qpat_…`.
  Create tokens in Settings → API tokens, with `wikanvas token create`, or with `wikanvas admin create-token`. A token
  is shown once; the server keeps only its hash.
- **Scopes:** `read` (every GET except tokens, sessions and export), `write` (content and structure), `admin`
  (tokens, sessions, account and workspace settings, permanent deletion, export, backup). A token can create tokens
  with no broader scope that expire no later than itself, and never an `admin` token.
- **Spec:** `GET /api/v1/openapi.json` (OpenAPI 3.1); the same file is committed as
  [`docs/api/openapi.json`](docs/api/openapi.json). Documents are readable and writable as Markdown and ProseMirror
  JSON, boards as native Inkboard JSON (§7.5).
- **Limits:** per token, `WIKANVAS_TOKEN_RATE` requests per minute and `WIKANVAS_TOKEN_BYTES_PER_HOUR` of request body;
  over either, 429 with `Retry-After`. Writes answer 507 below `WIKANVAS_MIN_FREE_MB`. CORS is off unless
  `WIKANVAS_CORS_ORIGINS` lists an origin.
- **Clients on a Docker network:** Wikanvas checks `Host` on every request. A container that calls
  `http://wikanvas:8080` on the compose network sends `Host: wikanvas`, so add `WIKANVAS_ALLOWED_HOSTS=wikanvas`. Such a
  request arrives as plain HTTP from an untrusted address, which is fine for a token client.

## The CLI

`wikanvas` is the command-line client of the API: every command takes `--json`, exit codes are stable, and it never
prompts. [`docs/AGENTS.md`](docs/AGENTS.md) is its guide (commands, output, exit codes, a complete session), written for
AI agents and scripts.

- **In the container** the CLI needs no URL (it talks to the server next to it):
  `docker compose exec -e WIKANVAS_TOKEN wikanvas wikanvas --json sidebar` with `WIKANVAS_TOKEN` exported in your shell,
  and `docker compose exec wikanvas wikanvas admin …` with no token.
- **Standalone:** `scripts/cli-dist.sh` builds a static binary for any x86_64 Linux in `dist/cli/`:
  `wikanvas-x86_64-unknown-linux-musl`, its `.sha256`, `LICENSE` and `THIRD_PARTY.md`.

  ```sh
  scripts/cli-dist.sh
  install -m 0755 dist/cli/wikanvas-x86_64-unknown-linux-musl ~/.local/bin/wikanvas
  wikanvas --json auth login --url https://wiki.example.org --token-stdin < agent.token   # saves a profile (0600)
  wikanvas --json auth whoami
  ```

  Tokens come from `--token-stdin`, `--token-file`, `WIKANVAS_TOKEN`, `WIKANVAS_TOKEN_FILE` or the profile, never from
  the command line. On a local CA, add `--ca-file wikanvas-local-ca.crt` unless the root is in the system bundle.

## Building and the build caps

**Builds never run on the host.** Every cargo, node and pnpm command runs in the `wikanvas-toolchain` container through
`scripts/dev.sh` (for example `scripts/dev.sh cargo test --workspace --locked`), capped at 6 CPUs and 12 GB of memory
with no swap, pinned to the host's last 6 cores, at low priority. Images are built only by `scripts/image.sh` and
`scripts/toolchain.sh` with the capped builder `wikanvas-limited`; `compose.yaml` has no `build:` key. The caps may be
lowered (`WIKANVAS_BUILD_CPUS`, `WIKANVAS_BUILD_MEM`, `WIKANVAS_BUILD_CPUSET`), never raised. The build scripts take one
per-user lock, so builds queue ("waiting for another Wikanvas build…"). [`CLAUDE.md`](CLAUDE.md) lists the rules for
contributors and agents; §13.1 has the details.

**The build slice** caps the total of all build containers at 6 CPUs and 12 GB, even when several run at once. It is
installed once, as root:

```sh
sudo scripts/install-build-slice.sh       # installs docker/wikanvas.slice and docker/wikanvas-build.slice
scripts/install-build-slice.sh --check    # no root: shows whether it is installed and holds its values
```

After installing it, run the `docker buildx rm wikanvas-limited; docker rm -f wikanvas-buildkit; scripts/toolchain.sh`
line that `scripts/toolchain.sh --builder-only` prints, so the builder joins the slice. The slice is optional for anyone
else building Wikanvas: without it every script prints "wikanvas-build.slice not installed: per-container caps only" and
keeps the per-container caps.

Tests run through the same wrappers: `scripts/dev.sh cargo test --workspace --locked`,
`scripts/dev.sh pnpm --dir web test`, `scripts/e2e.sh smoke` (Playwright against the image),
`scripts/image-smoke.sh` (the compose project with Caddy, over HTTPS) and `scripts/agent-test.sh` (CLI scenarios
against the image). PLAN W7 "Acceptance" lists the full release gate.

## Licence

Wikanvas is licensed under the MIT License ([`LICENSE`](LICENSE)). Third-party software, Inkboard included, is listed in [`THIRD_PARTY.md`](THIRD_PARTY.md); the image
also shows the generated list, with every licence text, in Settings → About (`/licenses.txt`).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Wikanvas by you shall
be licensed under the MIT License as above, without any additional terms or conditions.
