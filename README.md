# Quire

A self-hosted wiki: a sidebar of collections holding documents and
[Inkboard](https://github.com/VictorLetirand/inkboard) whiteboards, nested to any depth. The server is Rust (axum) on
SQLite; the frontend is React with the TipTap editor; boards run Inkboard compiled to WebAssembly. A board can hold
documents as cards: each is a real page of the tree, edited in place on the board with a double-click (§18). It has a
versioned HTTP API (`/api/v1`, with personal access tokens and an OpenAPI 3.1 spec) and a command-line client, `quire`.

**Status:** milestone 1, single owner, not released yet. The design is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
(cited below as §n) and the implementation plan in [`docs/PLAN.md`](docs/PLAN.md).

Contents: [Install](#install) · [Configuration](#configuration) · [Runtime limits](#runtime-limits) ·
[HTTPS](#https) · [Ports 80 or 443 taken](#when-port-80-or-443-is-taken) ·
[Behind an existing reverse proxy](#behind-an-existing-reverse-proxy) · [Backups](#backups-and-restore) ·
[Admin commands](#admin-commands) · [The API](#the-api) · [The CLI](#the-cli) · [Building](#building-and-the-build-caps)
· [Licence](#licence)

## Install

Quire runs as two containers in one compose project: the app (`quire`) and [Caddy](https://caddyserver.com), its
HTTPS front. The app has no published port; Caddy publishes 80 and 443 and terminates TLS (D57).

You need Linux on x86_64, Docker Engine with the Compose v2 plugin and buildx (the test overlays need Compose 2.24 or
later), git, and disk space for the build cache (the builder's garbage collection keeps it under 30 GB). Nothing is compiled on the host: every build runs in
resource-capped containers (see [Building](#building-and-the-build-caps)).

1. Get the source, with the Inkboard submodule:

   ```sh
   git clone --recurse-submodules <repository> quire && cd quire
   # or, in an existing clone: git submodule update --init vendor/inkboard
   ```

2. Build the image. `scripts/image.sh` creates the capped builder `quire-limited` the first time (a BuildKit container
   with 6 CPUs and 12 GB), then builds the web client, the Inkboard engine, the server and the CLI and tags
   `quire:<commit>` and `quire:latest`. The first build takes a while; later ones reuse the cache.

   ```sh
   scripts/image.sh
   ```

3. Optionally write a `.env` next to `compose.yaml` ([Configuration](#configuration)). Without one, Quire serves
   `https://localhost` on this machine only, with certificates from Caddy's local CA.

4. Start it:

   ```sh
   docker compose up -d
   ```

5. Trust Caddy's local CA once ([Trusting the local CA](#trusting-the-local-ca)), then open `https://localhost` (or
   `https://<QUIRE_DOMAIN>`, with `:<QUIRE_HTTPS_PORT>` when that is not 443).

6. Set up the owner account. Until an owner exists, the server prints a one-time setup code in its log:

   ```sh
   docker compose logs quire | grep 'First-run setup'
   # … First-run setup: https://localhost/setup code ABCD-EFGH-JKMN
   ```

   Open the link, enter the code, the workspace name, your name, email and a password (12 to 1024 bytes). Quire seeds
   a "Getting started" collection. Instead of the web form you can create the owner from the shell:

   ```sh
   docker compose exec -T quire quire admin create-owner --email you@example.org --name "Your Name" \
     --workspace "My wiki" --password-stdin < password.txt
   # or --generate-password: prints a password once and asks for a new one at the first sign-in
   ```

Upgrading is `git pull`, `git submodule update`, `scripts/image.sh` and `docker compose up -d`. The server migrates
the database at startup and writes a pre-migration backup first (§12).

## Configuration

Compose reads the variables below from `.env` next to `compose.yaml` (or from the shell). Every one is optional, and an
empty value counts as unset. A malformed value stops the server with `VAR: message` in `docker compose logs quire`.

| Variable | Default | Meaning |
|---|---|---|
| `QUIRE_DOMAIN` | `localhost` | The name Quire is served under. Caddy serves only this name; Quire allows it as `Host` with any port and derives the public URL from it |
| `QUIRE_TLS` | `internal` | `internal`: Caddy's local CA (localhost or a LAN name). `acme`: public certificates; Quire then also sends HSTS ([HTTPS](#https)) |
| `QUIRE_BIND` | `127.0.0.1` | The host address Caddy's ports bind to: `127.0.0.1` for this machine only, `0.0.0.0` for the LAN (required for `acme`) |
| `QUIRE_HTTP_PORT`, `QUIRE_HTTPS_PORT` | `80`, `443` | The host ports Caddy publishes. Change them only when 80 or 443 are taken ([below](#when-port-80-or-443-is-taken)) |
| `QUIRE_PUBLIC_URL` | derived | `https://<QUIRE_DOMAIN>`, plus `:<QUIRE_HTTPS_PORT>` unless 443. Set it (`scheme://host[:port]`, no path) only when clients reach Quire under another URL, for example behind your own proxy |
| `QUIRE_ALLOWED_HOSTS` | none | More `Host` values to accept, comma-separated `host` or `host:port`, for clients that reach Quire under another name, such as another container on the compose network (`quire`) |
| `QUIRE_BACKUP_INTERVAL` | `24h` | Scheduled backups: a number with `s`, `m`, `h` or `d` |
| `QUIRE_BACKUP_KEEP` | `7` | How many `quire-*.db` backups to keep; `0` turns scheduled backups off |
| `QUIRE_MAX_UPLOAD_MB` | `25` | Largest attachment upload |
| `QUIRE_MAX_BOARD_MB` | `50` | Largest board save (the web client warns at 15 MB) |
| `QUIRE_TOKEN_RATE` | `600` | Requests per minute per API token (burst 120) |
| `QUIRE_TOKEN_BYTES_PER_HOUR` | `1073741824` | Request-body bytes per API token per hour |
| `QUIRE_MIN_FREE_MB` | `1024` | The free-space floor on `/data`, capped at 10 % of the filesystem: below it, uploads, board saves, export and backups answer 507 |
| `QUIRE_CORS_ORIGINS` | none | Exact origins (`https://tool.example.org`, comma-separated) whose browser pages may call the API with a token. `*` is refused; cookies never work cross-origin |
| `QUIRE_CPUS`, `QUIRE_MEM` | `2`, `1g` | The app's CPU and memory caps ([Runtime limits](#runtime-limits)) |
| `QUIRE_CADDY_CPUS`, `QUIRE_CADDY_MEM` | `0.5`, `128m` | Caddy's caps |
| `QUIRE_NET_SUBNET`, `QUIRE_NET_RANGE`, `QUIRE_PROXY_IP` | `172.31.240.0/28`, `172.31.240.8/29`, `172.31.240.2` | The project network and Caddy's fixed address. Change all three together, only if the subnet collides with another network: the proxy address must lie inside the subnet and outside the range |
| `QUIRE_IMAGE` | `quire:latest` | The image to run, for example `quire:<commit>` to go back to an earlier build |
| `RUST_LOG` | `info` | The server's log filter |

`compose.yaml` passes no other variable to the app. The server reads a few more (`QUIRE_SETUP_TOKEN`, a fixed setup
code; `QUIRE_MAX_DOC_MB`, default 16, the largest stored document; `QUIRE_SYNC=FULL`, SQLite's `synchronous`); set those
in a `compose.override.yaml` under `services.quire.environment` if you need them. `QUIRE_TRUSTED_PROXIES` comes from
`QUIRE_PROXY_IP` (Caddy's address); see [Behind an existing reverse proxy](#behind-an-existing-reverse-proxy).

## Runtime limits

Both containers are capped in `compose.yaml`, and neither may swap (`memswap_limit` equals `mem_limit`):

| Container | CPUs | Memory | Other |
|---|---|---|---|
| `quire` | `QUIRE_CPUS=2` | `QUIRE_MEM=1g` | 256 processes, read-only root filesystem, every capability dropped, a 32 MB `/tmp` |
| `caddy` | `QUIRE_CADDY_CPUS=0.5` | `QUIRE_CADDY_MEM=128m` | 128 processes, read-only root filesystem, only `NET_BIND_SERVICE` |

Change them in `.env` and run `docker compose up -d` again. The defaults are the owner's requirement, and the server's
memory budget is built for 1 GiB (D25): the board limit, the number of documents loaded at once and the thread pools
(2 worker threads, `QUIRE_WORKER_THREADS` in `compose.yaml`) are sized for it. More memory adds headroom; it does not
raise any limit by itself.

## HTTPS

Caddy terminates HTTPS in front of Quire (`docker/Caddyfile`, §13.4). It forwards the client's address and scheme to
Quire, passes Quire's precompressed responses through untouched, and adds no headers of its own: every security
header, HSTS included, comes from Quire.

### Local or LAN (default: `QUIRE_TLS=internal`)

Caddy signs certificates with its own local CA, kept in the `caddy-data` volume (the root is valid for ten years; leaf
certificates renew by themselves). Use it for `localhost` or a LAN name:

```sh
# .env for a LAN name: the LAN's DNS (or each device's /etc/hosts) must resolve quire.lan to this host
QUIRE_DOMAIN=quire.lan
QUIRE_BIND=0.0.0.0
```

Quire sends **no HSTS** in this mode: HSTS on `localhost` or a private name would pin every device to HTTPS for a
year, including devices that do not trust the root yet.

### Trusting the local CA

Browsers and tools refuse the local certificates until they trust the root. Copy it out once per CA (a new
`caddy-data` volume means a new root, which every device has to trust again):

```sh
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt ./quire-local-ca.crt
```

- **This machine's system bundle** (curl, the `quire` CLI, most tools; Debian and Ubuntu):
  `sudo install -m 0644 quire-local-ca.crt /usr/local/share/ca-certificates/ && sudo update-ca-certificates`.
- **Chrome and Chromium** read their own NSS store (per user, no root; `certutil` is in `libnss3-tools`):
  `certutil -d "sql:$HOME/.pki/nssdb" -A -t "C,," -n "Quire local CA" -i quire-local-ca.crt`.
- **Firefox:** Settings → Privacy & Security → Certificates → View Certificates → Authorities → Import, and tick
  "Trust this CA to identify websites".
- **Other devices** import the same `root.crt` (on phones, as a CA profile or certificate).
- **The `quire` CLI** uses the system bundle once the root is in it; otherwise pass `--ca-file quire-local-ca.crt`
  (or set `QUIRE_CA_FILE`, or `ca_file` in its profile).

Only `root.crt` ever leaves the volume. **Keep `root.key` secret** (it stays in `caddy-data`): whoever holds it can
impersonate any site to every device that trusts the root.

### Public (`QUIRE_TLS=acme`)

```sh
QUIRE_TLS=acme
QUIRE_DOMAIN=wiki.example.org
QUIRE_BIND=0.0.0.0
```

The domain needs a DNS A record to this host, and ports 80 and 443 must be reachable from the Internet: Caddy gets
Let's Encrypt certificates through its automatic HTTPS and keeps them, with the ACME account, in `caddy-data` (keep the
volume: ACME has rate limits). In this mode Quire sends `Strict-Transport-Security: max-age=31536000` on every HTTPS
response (no `includeSubDomains`, no `preload`).

### The `caddy-data` volume

`caddy-data` holds the local CA's root and private key, the certificates and the ACME account. Back it up like a
secret ([Backups](#backups-and-restore)). **`docker compose down -v` deletes it**, and with it the local CA every
device trusts, as well as `quire-data`, the wiki itself. Use `docker compose down` (without `-v`) to stop the project.

### When port 80 or 443 is taken

`docker compose up` fails with "address already in use" when another web server or proxy holds port 80 or 443 on
the host (on the owner's machine, his own Caddy holds 443). There are two ways out.

**Other host ports.** Let Caddy publish other ports:

```sh
# .env
QUIRE_HTTPS_PORT=8443
QUIRE_HTTP_PORT=8088        # only if 80 is taken too
```

Then open `https://localhost:8443`. Quire's derived public URL includes the port (`https://localhost:8443`), and its
`Host` check accepts `QUIRE_DOMAIN` with any port. Caddy's plain-HTTP redirect still names port 443, so always use the
HTTPS URL with its port. In `acme` mode the certificate authority still connects to ports 80 and 443 of your public
address: forward them on the router to `QUIRE_HTTP_PORT` and `QUIRE_HTTPS_PORT`, or run Quire behind the proxy that
already owns them (next section).

**Your own proxy in front.** If a reverse proxy already serves 443 for other sites, let it serve Quire too, and do not
start the bundled Caddy.

### Behind an existing reverse proxy

Quire trusts `X-Forwarded-For` and `X-Forwarded-Proto` only from the addresses in `QUIRE_TRUSTED_PROXIES`. In the
bundled setup that is exactly Caddy's fixed address, so a client that reaches Quire any other way can neither choose its
rate-limit key nor claim HTTPS. A deployment that swaps Caddy for another proxy **must** set `QUIRE_TRUSTED_PROXIES` to
the address Quire sees that proxy connect from: every per-IP limit (login, setup, failed token attempts), the access
log, the Secure session cookie and HSTS depend on it. Without it every request looks like plain HTTP from one address.

1. Create `compose.override.yaml` next to `compose.yaml` (`docker compose` merges it automatically):

   ```yaml
   services:
     quire:
       ports: ["127.0.0.1:8085:8080"]            # loopback only: your proxy connects here
       environment:
         QUIRE_DOMAIN: wiki.example.org          # the name your proxy serves; Quire allows it as Host
         QUIRE_TLS: acme                         # HSTS on HTTPS requests; keep internal for a private name
         QUIRE_TRUSTED_PROXIES: 172.31.240.8/29  # where the proxy's connections appear to come from (step 3)
     caddy:
       profiles: [bundled-caddy]                 # not started unless asked for
   ```

   `QUIRE_DOMAIN` here sets the public URL, `https://wiki.example.org`; if your proxy serves another port or scheme, set
   `QUIRE_PUBLIC_URL` too.

2. Point the proxy at `127.0.0.1:8085`. It must keep the client's `Host`, send `X-Forwarded-For` (Quire takes the
   rightmost address) and `X-Forwarded-Proto: https`, and must not compress or decompress responses (Quire sends
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
   header (oauth2-proxy), keeps working: Quire ignores any `Authorization` value that is not a Quire token (D43).

3. Start only the app with `docker compose up -d`, send one request through the proxy, and check which address Quire
   saw:

   ```sh
   docker compose logs quire | grep -o 'client_ip=[^ ]*' | tail -n 3
   ```

   It should be the real client's address. If it is an address of the compose network instead, Quire did not trust
   the proxy: set `QUIRE_TRUSTED_PROXIES` to that address (or the network's range) and restart. With a loopback port
   as above, Docker hands the proxy's connections to Quire from inside the project network (on Docker 29 from an
   address in `QUIRE_NET_RANGE`, `172.31.240.8/29` by default), so trusting that range means: any process on this host
   that can reach `127.0.0.1:8085` may set `X-Forwarded-*`. A proxy that runs as a container can join the compose
   network instead (`quire_default`), with a fixed address outside the range, and proxy to `quire:8080`; trust that one
   address.

## Backups and restore

Everything lives in the `quire-data` volume, mounted at `/data`: `quire.db` (SQLite, WAL), `files/` (attachments and
board images, content-addressed), `backups/` and `tmp/`.

- **Scheduled backups** run every `QUIRE_BACKUP_INTERVAL` (24 h) and keep `QUIRE_BACKUP_KEEP` (7) copies as
  `/data/backups/quire-YYYYMMDDTHHMMZ.db`, made with SQLite's `VACUUM INTO` and checked with `quick_check`. The first
  one after a start is due an interval after the newest existing backup, and at least a minute after the start, so
  frequent restarts never postpone it. `QUIRE_BACKUP_KEEP=0` turns them off.
- **On demand:** Settings → Data → "Back up now", or `docker compose exec quire quire admin backup` (`--out PATH`
  writes elsewhere inside the container, under `/data`).
- **A backup holds the database only.** Attachments and board images are in `/data/files`. Pre-migration backups
  (`pre-migrate-v*.db`) are kept apart from the count.

**Keep an off-host copy.** `/data/backups` is on the same disk as the wiki; a lost disk takes both. Copy the backups and
the files out regularly, for example:

```sh
docker compose cp quire:/data/backups ./quire-backup/backups
docker compose cp quire:/data/files   ./quire-backup/files
docker compose cp caddy:/data         ./quire-backup/caddy-data   # the CA key and ACME account: keep it private
```

(or point restic or rsync at the volumes). An **export** (Settings → Data → Export, `quire export --out F`, or
`GET /api/v1/export` with an admin token) is a tar of `manifest.json`, a copy of `quire.db` and `files/`, without
sessions, API tokens or password hashes.

**Restore a backup.** Stop the app, put the backup in place as `quire.db`, delete the WAL files, start again. The image
has no shell, so use a throwaway container on the volume (the volume is `<project>_quire-data`, `quire_quire-data` by
default):

```sh
docker compose stop quire
docker run --rm -v quire_quire-data:/data --user 65532:65532 busybox \
  sh -c 'cp /data/backups/quire-20261028T0300Z.db /data/quire.db && rm -f /data/quire.db-wal /data/quire.db-shm'
docker compose start quire
```

If `/data/files` was lost too, copy the off-host `files/` back in first.

**Restore an export.** Unpack the tar into the stopped volume: `quire.db` into `/data` and `files/` as `/data/files`
(same throwaway container, mounting the unpacked directory read-only). An export has no password hashes and no tokens,
so set a password afterwards and create new tokens:

```sh
docker compose exec quire quire admin reset-password --email you@example.org
docker compose exec quire quire admin create-token --name cli --scope write --expires 30d
```

**Bind mounts.** The app runs as uid 65532. Named volumes take that ownership from the image; if you replace
`quire-data` with a bind mount, `chown -R 65532:65532` the host directory first.

## Admin commands

`quire-server admin …` works directly on the database, with no token, and may run while the server is up. Reach it
through the CLI in the container (`--json` gives machine-readable output; exit codes as in docs/AGENTS.md):

```sh
docker compose exec quire quire admin reset-password --email you@example.org          # prints a temporary password
docker compose exec quire quire admin create-owner --email … --name … --workspace … --generate-password
docker compose exec -T quire quire admin create-token --name ci --scope write --expires 1d > ci.token
docker compose exec quire quire admin list-tokens
docker compose exec quire quire admin revoke-token <id>                             # and the tokens it created
docker compose exec quire quire admin backup
docker compose exec quire quire admin check                                         # integrity, FTS, missing files
```

`reset-password` sets a random temporary password, prints it once, requires a new one at the next sign-in and signs out
every session. `create-token` prints the token once (`--token-out F` writes it to a 0600 file inside the container);
`--expires` is `<days>d` or `never`, 90 days by default.

**If a password or a token may be compromised:**

```sh
docker compose exec quire quire admin reset-password --email you@example.org --revoke-tokens   # also every token
docker compose exec quire quire admin revoke-token --all --yes                                # every token, now
```

A password change in the web client signs out the other sessions but leaves tokens alone; they are separate
credentials.

## The API

Everything the web client does goes through the public, versioned API under `/api/v1` (D39); there are no private
routes. Within v1, changes are additive only.

- **Authentication:** the session cookie (the browser), or a personal access token: `Authorization: Bearer qpat_…`.
  Create tokens in Settings → API tokens, with `quire token create`, or with `quire admin create-token`. A token is
  shown once; the server keeps only its hash.
- **Scopes:** `read` (every GET except tokens, sessions and export), `write` (content and structure), `admin`
  (tokens, sessions, account and workspace settings, permanent deletion, export, backup). A token can create tokens
  with no broader scope that expire no later than itself, and never an `admin` token.
- **Spec:** `GET /api/v1/openapi.json` (OpenAPI 3.1); the same file is committed as
  [`docs/api/openapi.json`](docs/api/openapi.json). Documents are readable and writable as Markdown and ProseMirror
  JSON, boards as native Inkboard JSON (§7.5).
- **Limits:** per token, `QUIRE_TOKEN_RATE` requests per minute and `QUIRE_TOKEN_BYTES_PER_HOUR` of request body;
  over either, 429 with `Retry-After`. Writes answer 507 below `QUIRE_MIN_FREE_MB`. CORS is off unless
  `QUIRE_CORS_ORIGINS` lists an origin.
- **Clients on a Docker network:** Quire checks `Host` on every request. A container that calls `http://quire:8080`
  on the compose network sends `Host: quire`, so add `QUIRE_ALLOWED_HOSTS=quire`. Such a request arrives as plain HTTP
  from an untrusted address, which is fine for a token client.

## The CLI

`quire` is the command-line client of the API: every command takes `--json`, exit codes are stable, and it never
prompts. [`docs/AGENTS.md`](docs/AGENTS.md) is its guide (commands, output, exit codes, a complete session), written for
AI agents and scripts.

- **In the container** the CLI needs no URL (it talks to the server next to it):
  `docker compose exec -e QUIRE_TOKEN quire quire --json sidebar` with `QUIRE_TOKEN` exported in your shell, and
  `docker compose exec quire quire admin …` with no token.
- **Standalone:** `scripts/cli-dist.sh` builds a static binary for any x86_64 Linux in `dist/cli/`:
  `quire-x86_64-unknown-linux-musl`, its `.sha256`, `LICENSE` and `THIRD_PARTY.md`.

  ```sh
  scripts/cli-dist.sh
  install -m 0755 dist/cli/quire-x86_64-unknown-linux-musl ~/.local/bin/quire
  quire --json auth login --url https://wiki.example.org --token-stdin < agent.token   # saves a profile (0600)
  quire --json auth whoami
  ```

  Tokens come from `--token-stdin`, `--token-file`, `QUIRE_TOKEN`, `QUIRE_TOKEN_FILE` or the profile, never from the
  command line. On a local CA, add `--ca-file quire-local-ca.crt` unless the root is in the system bundle.

## Building and the build caps

**Builds never run on the host.** Every cargo, node and pnpm command runs in the `quire-toolchain` container through
`scripts/dev.sh` (for example `scripts/dev.sh cargo test --workspace --locked`), capped at 6 CPUs and 12 GB of memory
with no swap, pinned to the host's last 6 cores, at low priority. Images are built only by `scripts/image.sh` and
`scripts/toolchain.sh` with the capped builder `quire-limited`; `compose.yaml` has no `build:` key. The caps may be
lowered (`QUIRE_BUILD_CPUS`, `QUIRE_BUILD_MEM`, `QUIRE_BUILD_CPUSET`), never raised. The build scripts take one
per-user lock, so builds queue ("waiting for another Quire build…"). [`CLAUDE.md`](CLAUDE.md) lists the rules for
contributors and agents; §13.1 has the details.

**The build slice** caps the total of all build containers at 6 CPUs and 12 GB, even when several run at once. It is
installed once, as root:

```sh
sudo scripts/install-build-slice.sh       # installs docker/quire.slice and docker/quire-build.slice
scripts/install-build-slice.sh --check    # no root: shows whether it is installed and holds its values
```

After installing it, run the `docker buildx rm quire-limited; docker rm -f quire-buildkit; scripts/toolchain.sh` line
that `scripts/toolchain.sh --builder-only` prints, so the builder joins the slice. The slice is optional for anyone
else building Quire: without it every script prints "quire-build.slice not installed: per-container caps only" and
keeps the per-container caps.

Tests run through the same wrappers: `scripts/dev.sh cargo test --workspace --locked`,
`scripts/dev.sh pnpm --dir web test`, `scripts/e2e.sh smoke` (Playwright against the image),
`scripts/image-smoke.sh` (the compose project with Caddy, over HTTPS) and `scripts/agent-test.sh` (CLI scenarios
against the image). PLAN W7 "Acceptance" lists the full release gate.

## Licence

Quire is licensed under the MIT License ([`LICENSE`](LICENSE)). Third-party software, Inkboard included, is listed in [`THIRD_PARTY.md`](THIRD_PARTY.md); the image
also shows the generated list, with every licence text, in Settings → About (`/licenses.txt`).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Quire by you shall be
licensed under the MIT License as above, without any additional terms or conditions.
