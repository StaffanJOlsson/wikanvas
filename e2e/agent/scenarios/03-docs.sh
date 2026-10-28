#!/usr/bin/env bash
# 03: documents in Markdown and JSON: ETags, a stale PUT exits 9, --merge, --title-h1, then W4's Markdown goldens
# (fixtures/markdown/*.md → .expected.md and .warnings.json), with the goldens' items created on the server.
. "$AGENT_ROOT/e2e/agent/lib.sh"

scratch_collection
q item create --collection "$SCRATCH" --kind doc --md - <<<$'First paragraph.\n\nSecond paragraph.'
expect_rc 0
id="$(val .item.id)"
base="$(val .doc.etag)"
q doc get "$id" --format json
expect_rc 0
expect '.format == "json" and .content.schema_version == 1'
ok "doc get --format json has schema_version 1"

q doc put "$id" --md - --if-match "$base" <<<$'First paragraph, edited by A.\n\nSecond paragraph.'
expect_rc 0
q doc put "$id" --md - --if-match "$base" <<<$'First paragraph.\n\nSecond paragraph, edited by B.'
expect_rc 9
expect_err '.error.code == "rev_mismatch" and .exit_code == 9'
ok "a PUT with an ETag from before a second PUT exits 9"

q doc put "$id" --md - --if-match "$base" --merge <<<$'First paragraph.\n\nSecond paragraph, edited by B.'
expect_rc 0
expect '.merged == true'
q doc get "$id"
expect_rc 0
expect '(.content | contains("edited by A")) and (.content | contains("edited by B"))'
q doc put "$id" --md - --if-match "$base" --merge <<<$'First paragraph.\n\nSecond paragraph, edited by B.'
expect_rc 0
expect '.changed == false'
ok "the same PUT with --merge succeeds with both edits; repeating it changes nothing"

q item create --collection "$SCRATCH" --kind doc --md - --title-h1 <<<$'# From the heading\n\nBody.'
expect_rc 0
expect '.item.title == "From the heading"'
ok "--title-h1 sets the title"

# The goldens were made against W4's GoldenLookup (crates/quire-core/src/convert/tests.rs): the documents "Doc A" and
# "Doc B", the board "Board C", one attachment, and the public origin https://wiki.example.com. Their counterparts are
# created here, and every golden id, url_id and the origin are mapped to the live ones in the input and in the expected
# Markdown, so a golden checks link, mention, embed and image resolution against real items. Ids no golden item has
# (an unknown item, a missing attachment) stay unknown.
golden_map="$AGENT_RESULTS/golden.sed"
: >"$golden_map"
golden_item() { # golden-id golden-url-id kind title
  q item create --collection "$SCRATCH" --kind "$3" --title "$4"
  expect_rc 0
  printf 's|%s|%s|g\ns|%s|%s|g\n' "$1" "$(val .id)" "$2" "$(val .url_id)" >>"$golden_map"
}
golden_item 0192f0a0-1111-7000-8000-00000000000a DocA00000a doc "Doc A"
golden_item 0192f0a0-2222-7000-8000-00000000000b DocB00000b doc "Doc B"
golden_item 0192f0a0-3333-7000-8000-00000000000c Board0000c board "Board C"
png="$(fixture pixel.png)" || exit 1
q attachment upload - --filename pixel.png <"$png"
expect_rc 0
printf 's|%s|%s|g\n' 0192f0a0-4444-7000-8000-000000000001 "$(val .id)" >>"$golden_map"
# The server's public URL (QUIRE_PUBLIC_URL of the project, compose.e2e.yaml), which item URLs in Markdown name.
public="$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$AGENT_CID" | sed -n 's/^QUIRE_PUBLIC_URL=//p')"
[ -n "$public" ] || _fail "the quire container has no QUIRE_PUBLIC_URL"
printf 's|https://wiki\\.example\\.com|%s|g\n' "$public" >>"$golden_map"
ok "the goldens' documents, board and attachment exist on the server"

shopt -s nullglob
goldens=()
for f in "$AGENT_ROOT"/fixtures/markdown/*.md; do
  case "$f" in *.expected.md) continue ;; esac
  goldens+=("$f")
done
[ "${#goldens[@]}" -gt 0 ] || _fail "no Markdown goldens in fixtures/markdown (W4 adds them)"
for f in "${goldens[@]}"; do
  stem="${f%.md}"
  name="$(basename "$f")"
  [ -f "$stem.expected.md" ] || _fail "$name has no .expected.md"
  sed -f "$golden_map" "$f" >"$AGENT_RESULTS/golden.md"
  sed -f "$golden_map" "$stem.expected.md" >"$AGENT_RESULTS/golden.expected.md"
  q item create --collection "$SCRATCH" --kind doc --md - <"$AGENT_RESULTS/golden.md"
  expect_rc 0
  gid="$(val .item.id)"
  if [ -f "$stem.warnings.json" ]; then
    want="$(jq -c '[.[] | if type == "object" then .code else . end] | sort' "$stem.warnings.json")"
  else
    want='[]'
  fi
  expect "([.doc.warnings[].code] | sort) == $want"
  q doc get "$gid" --format md
  expect_rc 0
  jq -j .content "$OUT_FILE" | cmp -s - "$AGENT_RESULTS/golden.expected.md" ||
    _fail "doc get --format md of $name differs from its .expected.md (golden ids mapped to the live items)"
  ok "golden $name round-trips with its warnings"
done
