#!/usr/bin/env bash
# 04: boards as native JSON: unknown fields kept, document.name owned by the server, the SVG preview, 412, --force.
. "$AGENT_ROOT/e2e/agent/lib.sh"

board="$(fixture future.inkboard)" || exit 1
scratch_collection
q item create --collection "$SCRATCH" --kind board --title "Agent board"
expect_rc 0
id="$(val .id)"
q board put "$id" - --if-match r1 <"$board"
expect_rc 0
expect '(.etag | startswith("\"r")) and .rev >= 2'
ok "board put of future.inkboard with --if-match r1"

q board get "$id"
expect_rc 0
expect '.format == "inkboard" and .content.document.x_future.note == "kept" and .content.document.pages[0].x_future == 42
  and .content.document.name == "Agent board"'
ok "board get keeps both x_future values and document.name equals the title"

for _ in $(seq 1 30); do
  q board preview "$id" --page 0 --out /tmp/agent-p0.svg
  if [ "$RC" = 0 ] && jq -e '(.content_type // "") | startswith("image/svg+xml")' "$OUT_FILE" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
expect_rc 0
expect '(.content_type | startswith("image/svg+xml")) and .bytes > 0'
ok "board preview --page 0 gives image/svg+xml"

q board put "$id" - --if-match r1 <"$board"
expect_rc 9
expect_err '.error.code == "rev_mismatch"'
q board put "$id" - --force <"$board"
expect_rc 0
ok "a stale --if-match exits 9; --force succeeds"
