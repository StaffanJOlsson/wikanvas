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

# A stale --if-match with the stored board again would be a no-op (§6.2 step 4: the hash check comes first), so the
# stale PUT sends a changed board.
changed="$AGENT_RESULTS/changed.inkboard"
jq -c '.document.x_future.note = "changed"' "$board" >"$changed" || _fail "jq could not write $changed"
q board put "$id" - --if-match r1 <"$changed"
expect_rc 9
expect_err '.error.code == "rev_mismatch"'
q board get "$id"
expect_rc 0
expect '.content.document.x_future.note == "kept"'
q board put "$id" - --force <"$changed"
expect_rc 0
q board get "$id"
expect_rc 0
expect '.content.document.x_future.note == "changed"'
ok "a stale --if-match with a changed board exits 9 and writes nothing; --force stores it"
