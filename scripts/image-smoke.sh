#!/usr/bin/env bash
# Smoke test of the production image in its compose project, Caddy included (docs/ARCHITECTURE.md §13.3, §14).
#   scripts/image-smoke.sh                              # QUIRE_SMOKE_LEVEL=full (the default)
#   QUIRE_SMOKE_LEVEL=basic scripts/image-smoke.sh      # also passes with an INKBOARD_BUILD=stub image
# Runs compose.yaml + compose.smoke.yaml as project quire-smoke on its own subnet (172.31.240.48/28, range .56/29,
# Caddy at .50), Caddy in internal mode for localhost on 127.0.0.1:18443 only, with throwaway tmpfs volumes, so the
# caps checked are the compose ones. Neither the owner's .env (--env-file /dev/null) nor the owner-facing variables of
# the shell reach the project. The image is ${QUIRE_IMAGE:-quire:latest}; build it with scripts/image.sh first.
#   basic: quire-server --version and quire --version run inside the image; the quire container publishes no port
#          (Caddy only 127.0.0.1:18443); Caddy's local root CA, read from its tmpfs, verifies
#          https://localhost:18443/healthz (200), which fails without it; NanoCpus, Memory and MemorySwap of both
#          containers are the compose caps (and their cgroup parent the build slice when it is installed).
#   full:  basic, plus through Caddy: the SPA CSP on /; the frame CSP on frame.html; the engine's .wasm sent as br,
#          application/wasm, with exactly the manifest's br byte count and byte for byte (its sha256 is Quire's ETag);
#          manifest.json; setup with smoke-setup-code sets __Host-quire_session with Secure; no
#          Strict-Transport-Security; docker stats memory stays under both limits for the whole run.
# Takes the build lock and calls quire_slice: the app joins quire-build.slice when it is installed. The project is
# removed on exit (down -v), also after a failure, after printing the last log lines.
# Exit codes: 0 every check passed, 1 a check failed, 2 usage, 3 a prerequisite is missing (docker, curl, jq, the
# image), 4 the project did not come up healthy. Host prerequisites: Docker with Compose >= 2.24, curl, jq.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

usage() {
  sed -n '2,21s/^# \{0,1\}//p' "$root/scripts/image-smoke.sh"
}
case "$#:${1:-}" in
  0:) ;;
  1:-h | 1:--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
level="${QUIRE_SMOKE_LEVEL:-full}"
case "$level" in
  basic | full) ;;
  *)
    echo "QUIRE_SMOKE_LEVEL=$level: must be basic or full" >&2
    exit 2
    ;;
esac
for tool in docker curl jq; do
  if ! command -v "$tool" >/dev/null; then
    echo "image-smoke: missing host prerequisite: $tool" >&2
    exit 3
  fi
done

# The owner-facing variables (§13.3) and the other settings compose.yaml reads never leak into the smoke project:
# its caps and settings are always the defaults of compose.yaml. QUIRE_IMAGE (the image under test) and RUST_LOG are
# kept.
unset QUIRE_DOMAIN QUIRE_TLS QUIRE_BIND QUIRE_HTTP_PORT QUIRE_HTTPS_PORT QUIRE_CPUS QUIRE_MEM QUIRE_CADDY_CPUS \
  QUIRE_CADDY_MEM QUIRE_NET_SUBNET QUIRE_NET_RANGE QUIRE_PROXY_IP
unset QUIRE_PUBLIC_URL QUIRE_ALLOWED_HOSTS QUIRE_BACKUP_INTERVAL QUIRE_BACKUP_KEEP QUIRE_MAX_UPLOAD_MB \
  QUIRE_MAX_BOARD_MB QUIRE_TOKEN_RATE QUIRE_TOKEN_BYTES_PER_HOUR QUIRE_MIN_FREE_MB QUIRE_CORS_ORIGINS
export QUIRE_DOMAIN=localhost QUIRE_TLS=internal QUIRE_HTTPS_PORT=18443
export QUIRE_NET_SUBNET=172.31.240.48/28 QUIRE_NET_RANGE=172.31.240.56/29 QUIRE_PROXY_IP=172.31.240.50
project=quire-smoke
image="${QUIRE_IMAGE:-quire:latest}"
url="https://localhost:$QUIRE_HTTPS_PORT"
resolve="localhost:$QUIRE_HTTPS_PORT:127.0.0.1"
# Caps as compose.yaml sets them by default (§13.3, D25, D57).
want_quire="2000000000 1073741824 1073741824"
want_caddy="500000000 134217728 134217728"

quire_lock || exit 1
quire_slice

if ! docker image inspect "$image" >/dev/null 2>&1; then
  echo "image-smoke: $image not found: run scripts/image.sh first" >&2
  exit 3
fi

compose() {
  docker compose --env-file /dev/null -p "$project" -f "$root/compose.yaml" -f "$root/compose.smoke.yaml" "$@"
}

work="$(mktemp -d)"
: >"$work/stats"
sampler_pid=""
cleanup() {
  local rc=$?
  if [ -n "$sampler_pid" ]; then
    kill "$sampler_pid" 2>/dev/null || true
    wait "$sampler_pid" 2>/dev/null || true
  fi
  if [ "$rc" -ne 0 ]; then
    echo "--- last log lines of $project ---" >&2
    compose logs --no-color --tail 40 >&2 2>/dev/null || true
  fi
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$work"
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

failures=0
pass() { echo "ok - $*"; }
fail() {
  echo "not ok - $*"
  failures=$((failures + 1))
}

# curl through Caddy over HTTPS with the smoke's own local root. -q ignores ~/.curlrc.
hcurl() { curl -q -sS --max-time 15 --cacert "$work/root.crt" --resolve "$resolve" "$@"; }
# The value of a response header (case-insensitive name, CR stripped) from a curl -D file.
# A missing file (curl got no response) gives an empty value.
header() {
  [ -f "$1" ] || return 0
  sed -n "s/^$2: *//Ip" "$1" | tr -d '\r' | tail -n 1
}

# A leftover project from an interrupted run would keep its old containers and data.
compose down -v --remove-orphans >/dev/null 2>&1 || true
echo "starting $project ($image, level $level)" >&2
if ! compose up -d --wait --wait-timeout 180; then
  echo "image-smoke: $project did not come up healthy" >&2
  exit 4
fi
qid="$(compose ps -q quire)"
cid="$(compose ps -q caddy)"
if [ -z "$qid" ] || [ -z "$cid" ]; then
  echo "image-smoke: $project has no running quire or caddy container" >&2
  exit 4
fi

# full: sample docker stats in the background for the whole run (each sample takes about a second).
if [ "$level" = full ]; then
  (
    while :; do
      docker stats --no-stream --format '{{.ID}} {{.MemUsage}} {{.CPUPerc}}' "$qid" "$cid" >>"$work/stats" 2>/dev/null || true
    done
  ) &
  sampler_pid=$!
fi

# --- basic -----------------------------------------------------------------------------------------------------
for bin in quire-server quire; do
  if out="$(compose exec -T quire "$bin" --version 2>&1)"; then
    pass "$bin --version inside the image: $out"
  else
    fail "$bin --version inside the image: $out"
  fi
done

qports="$(docker port "$qid" 2>/dev/null || true)"
qbind="$(docker inspect -f '{{json .HostConfig.PortBindings}}' "$qid")"
if [ -z "$qports" ] && { [ "$qbind" = '{}' ] || [ "$qbind" = null ]; }; then
  pass "the quire container publishes no port"
else
  fail "the quire container publishes ports: ${qports:-$qbind}"
fi
cports="$(docker port "$cid" 2>/dev/null || true)"
if [ "$cports" = "443/tcp -> 127.0.0.1:$QUIRE_HTTPS_PORT" ]; then
  pass "caddy publishes only 127.0.0.1:$QUIRE_HTTPS_PORT"
else
  fail "caddy publishes '$cports', not only 443/tcp -> 127.0.0.1:$QUIRE_HTTPS_PORT"
fi

# Caddy's local CA lives on a tmpfs, which docker cp cannot read: cat it through exec (bounded poll; Caddy creates it
# right after start).
got_root=0
for _ in $(seq 1 30); do
  if compose exec -T caddy cat /data/caddy/pki/authorities/local/root.crt >"$work/root.crt" 2>/dev/null &&
    grep -q -- '-----BEGIN CERTIFICATE-----' "$work/root.crt"; then
    got_root=1
    break
  fi
  sleep 1
done
if [ "$got_root" = 1 ]; then
  pass "read Caddy's local root CA from its tmpfs"
else
  fail "Caddy's local root CA did not appear within 30 s"
fi

code=000
if [ "$got_root" = 1 ]; then
  # The leaf certificate may still be on its way: bounded retries.
  for _ in $(seq 1 20); do
    code="$(hcurl -o "$work/healthz" -D "$work/healthz.h" -w '%{http_code}' "$url/healthz" 2>/dev/null)" || code=000
    [ "$code" = 200 ] && break
    sleep 1
  done
fi
if [ "$code" = 200 ] && jq -e '.status == "ok"' "$work/healthz" >/dev/null 2>&1; then
  pass "GET $url/healthz through Caddy with the local CA: 200 {\"status\":\"ok\"}"
else
  fail "GET $url/healthz through Caddy with the local CA: HTTP $code"
fi
rc=0
curl -q -sS --max-time 15 --resolve "$resolve" -o /dev/null "$url/healthz" 2>/dev/null || rc=$?
if [ "$rc" = 60 ] || [ "$rc" = 35 ]; then
  pass "without --cacert the certificate is refused (curl exit $rc)"
else
  fail "without --cacert curl exited $rc (want 60: certificate not trusted)"
fi

got="$(docker inspect -f '{{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}' "$qid")"
if [ "$got" = "$want_quire" ]; then
  pass "quire caps: NanoCpus Memory MemorySwap = $got"
else
  fail "quire caps: NanoCpus Memory MemorySwap = $got (want $want_quire)"
fi
got="$(docker inspect -f '{{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}' "$cid")"
if [ "$got" = "$want_caddy" ]; then
  pass "caddy caps: NanoCpus Memory MemorySwap = $got"
else
  fail "caddy caps: NanoCpus Memory MemorySwap = $got (want $want_caddy)"
fi
for id in "$qid" "$cid"; do
  parent="$(docker inspect -f '{{.HostConfig.CgroupParent}}' "$id")"
  name="$(docker inspect -f '{{.Name}}' "$id")"
  if [ "$parent" = "$QUIRE_CGROUP_PARENT" ]; then
    pass "${name#/} cgroup parent: ${parent:-none (slice not installed)}"
  else
    fail "${name#/} cgroup parent '$parent' (want '$QUIRE_CGROUP_PARENT')"
  fi
done

# --- full ------------------------------------------------------------------------------------------------------
if [ "$level" = full ]; then
  code="$(hcurl -o /dev/null -D "$work/index.h" -w '%{http_code}' "$url/" 2>/dev/null)" || code=000
  csp="$(header "$work/index.h" content-security-policy)"
  if [ "$code" = 200 ] && [[ "$csp" == *"default-src 'none'"* ]] && [[ "$csp" == *"frame-ancestors 'none'"* ]]; then
    pass "GET / has the SPA CSP"
  else
    fail "GET /: HTTP $code, Content-Security-Policy '$csp'"
  fi

  code="$(hcurl -o "$work/config.json" -D "$work/config.h" -w '%{http_code}' "$url/api/v1/config" 2>/dev/null)" ||
    code=000
  base="$(jq -r '.inkboard.base // empty' "$work/config.json" 2>/dev/null || true)"
  want_br="$(jq -r '.inkboard.wasm_bytes.br // empty' "$work/config.json" 2>/dev/null || true)"
  if [ "$code" != 200 ]; then
    fail "GET /api/v1/config: HTTP $code"
  elif ! [[ "$base" =~ ^/inkboard/[0-9a-f]+/$ ]] || ! [[ "$want_br" =~ ^[0-9]+$ ]]; then
    fail "config.inkboard has no engine (base '$base', wasm_bytes.br '$want_br'): an INKBOARD_BUILD=stub image passes only QUIRE_SMOKE_LEVEL=basic"
  else
    code="$(hcurl -H 'Accept-Encoding: br' -o "$work/wasm.br" -D "$work/wasm.h" -w '%{http_code} %{size_download}' \
      "$url${base}inkboard_bg.wasm" 2>/dev/null)" || code="000 0"
    enc="$(header "$work/wasm.h" content-encoding)"
    type="$(header "$work/wasm.h" content-type)"
    if [ "$code" = "200 $want_br" ] && [ "$enc" = br ] && [ "$type" = application/wasm ]; then
      pass "${base}inkboard_bg.wasm: br, application/wasm, $want_br bytes as in the manifest"
    else
      fail "${base}inkboard_bg.wasm: HTTP and bytes '$code' (want '200 $want_br'), Content-Encoding '$enc', Content-Type '$type'"
    fi
    # Byte for byte: Quire's ETag is the first 16 bytes of the sha256 of the embedded variant it sends (§11), so the
    # bytes that came through Caddy must hash to it.
    etag="$(header "$work/wasm.h" etag)"
    got_sha="$(sha256sum "$work/wasm.br" 2>/dev/null | cut -c1-32)"
    if [[ "$etag" =~ ^\"[0-9a-f]{32}\"$ ]] && [ "\"$got_sha\"" = "$etag" ]; then
      pass "${base}inkboard_bg.wasm through Caddy byte for byte: sha256 ${got_sha}… is the ETag of Quire's embedded .br"
    else
      fail "${base}inkboard_bg.wasm through Caddy: sha256 '${got_sha}…' does not match Quire's ETag '$etag'"
    fi
    # The frame page has its own CSP: wasm allowed, framed only by Quire's own pages (§11).
    code="$(hcurl -o /dev/null -D "$work/frame.h" -w '%{http_code}' "$url${base}frame.html" 2>/dev/null)" || code=000
    csp="$(header "$work/frame.h" content-security-policy)"
    xfo="$(header "$work/frame.h" x-frame-options)"
    if [ "$code" = 200 ] && [[ "$csp" == *"script-src 'self' 'wasm-unsafe-eval'"* ]] &&
      [[ "$csp" == *"frame-ancestors 'self'"* ]] && [ "$xfo" = SAMEORIGIN ]; then
      pass "${base}frame.html has the frame CSP and X-Frame-Options: SAMEORIGIN"
    else
      fail "${base}frame.html: HTTP $code, Content-Security-Policy '$csp', X-Frame-Options '$xfo'"
    fi
    # The frame page reads manifest.json next to frame.html before it downloads the engine (§11).
    build="${base#/inkboard/}"
    build="${build%/}"
    code="$(hcurl -H 'Accept-Encoding: br, gzip' -o "$work/manifest.json" -D "$work/manifest.h" -w '%{http_code}' \
      "$url${base}manifest.json" 2>/dev/null)" || code=000
    enc="$(header "$work/manifest.h" content-encoding)"
    type="$(header "$work/manifest.h" content-type)"
    if [ "$code" = 200 ] && [ -z "$enc" ] && [[ "$type" == application/json* ]] &&
      jq -e --arg build "$build" '.build == $build and (.files["inkboard_bg.wasm"].raw | type) == "number"' \
        "$work/manifest.json" >/dev/null 2>&1; then
      pass "${base}manifest.json: identity application/json of build $build"
    else
      fail "${base}manifest.json: HTTP $code, Content-Encoding '$enc', Content-Type '$type' (want 200, identity, the manifest of build $build)"
    fi
  fi

  body="$(jq -nc '{code: "smoke-setup-code", workspace_name: "Smoke", name: "Smoke Owner",
    email: "smoke@quire.invalid", password: "smoke-password-0123456789"}')"
  code="$(hcurl -X POST -H 'Content-Type: application/json' -H "Origin: $url" --data "$body" \
    -o "$work/setup.json" -D "$work/setup.h" -w '%{http_code}' "$url/api/v1/setup" 2>/dev/null)" || code=000
  cookie=""
  if [ -f "$work/setup.h" ]; then
    cookie="$(sed -n 's/^set-cookie: *\(__Host-quire_session=.*\)$/\1/Ip' "$work/setup.h" | tr -d '\r' | tail -n 1)"
  fi
  if [ "$code" = 201 ] && [ -n "$cookie" ] && grep -qiE ';[[:space:]]*secure([[:space:]]*;|$)' <<<"$cookie"; then
    pass "POST /api/v1/setup with smoke-setup-code: 201, __Host-quire_session with Secure"
  else
    fail "POST /api/v1/setup: HTTP $code, session cookie '${cookie%%=*}' (want 201 and __Host-quire_session; Secure)"
  fi

  hsts=""
  for h in "$work"/*.h; do
    if [ -n "$(header "$h" strict-transport-security)" ]; then hsts="$hsts $(basename "$h" .h)"; fi
  done
  if [ -z "$hsts" ]; then
    pass "no Strict-Transport-Security in internal mode"
  else
    fail "Strict-Transport-Security sent in internal mode on:$hsts"
  fi

  # One more sample after the checks, then the peaks against the limits.
  docker stats --no-stream --format '{{.ID}} {{.MemUsage}} {{.CPUPerc}}' "$qid" "$cid" >>"$work/stats" 2>/dev/null || true
  kill "$sampler_pid" 2>/dev/null || true
  wait "$sampler_pid" 2>/dev/null || true
  sampler_pid=""
  for pair in "quire $qid 1073741824" "caddy $cid 134217728"; do
    read -r name id limit <<<"$pair"
    # Lines look like "<id12> 45.3MiB / 1GiB 0.52%"; docker prints sizes with binary (KiB) or decimal (kB) units.
    peak="$(LC_ALL=C awk -v id="${id:0:12}" '
      function bytes(s,   n, u) {
        n = s + 0; u = s; sub(/^[0-9.]+/, "", u)
        if (u == "KiB") n *= 1024; else if (u == "MiB") n *= 1048576; else if (u == "GiB") n *= 1073741824
        else if (u == "kB") n *= 1000; else if (u == "MB") n *= 1000000; else if (u == "GB") n *= 1000000000
        return n
      }
      substr($1, 1, 12) == id { m = bytes($2); if (m > max) max = m; c = $5 + 0; if (c > cpu) cpu = c; seen = 1 }
      END { if (seen) printf "%d %.1f\n", max, cpu }' "$work/stats")"
    if [ -z "$peak" ]; then
      fail "docker stats: no samples for $name"
      continue
    fi
    read -r mem cpu <<<"$peak"
    if [ "$mem" -lt "$limit" ]; then
      pass "docker stats: $name peak memory $((mem / 1048576)) MiB < $((limit / 1048576)) MiB (peak CPU $cpu%)"
    else
      fail "docker stats: $name peak memory $mem bytes reached its limit $limit"
    fi
  done
fi

if [ "$failures" -gt 0 ]; then
  echo "image-smoke ($level): $failures check(s) failed" >&2
  exit 1
fi
echo "image-smoke ($level): every check passed" >&2
