#!/usr/bin/env bash
# 03: documents in Markdown and JSON: ETags, a stale PUT exits 9, --merge, --title-h1, then W4's Markdown goldens
# (fixtures/markdown/*.md → .expected.md and .warnings.json).
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
  q item create --collection "$SCRATCH" --kind doc --md - <"$f"
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
  jq -j .content "$OUT_FILE" | cmp -s - "$stem.expected.md" || _fail "doc get --format md of $name differs from its .expected.md"
  ok "golden $name round-trips with its warnings"
done
