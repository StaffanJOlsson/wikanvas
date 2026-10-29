#!/usr/bin/env bash
# 12: board document embeds (ARCHITECTURE §18, PLAN M1.1 E2): cards kept byte for byte through the CLI, malformed
# embeds accepted and kept, the title snapshots in the text, search and preview, duplicate and the tree leaving the
# cards alone, AGENTS.md's jq recipe run as it is written, and an embedded empty document never discarded.
. "$AGENT_ROOT/e2e/agent/lib.sh"

fixture_board="$AGENT_ROOT/fixtures/boards/embed.inkboard"
[ -f "$fixture_board" ] || _fail "no fixture $fixture_board"
P1=0192f0e0-0000-7000-8000-000000000001
P2=0192f0e0-0000-7000-8000-000000000002
work="$AGENT_RESULTS/12-board-embeds"
rm -rf "$work"
mkdir -p "$work" || _fail "cannot make $work"

# The targets of a board's cards, in drawing order (§18.2: a rect box whose kind.embed is an object with a string item).
cards='[.document.pages[].objects[]
  | select(.kind.type == "box" and .kind.geom == "rect" and (.kind.embed | type) == "object"
           and (.kind.embed.item | type) == "string")
  | .kind.embed.item]'
# Of those, the canonical UUIDs (what the server links), distinct and sorted.
linked="$cards | map(select(test(\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\$\"))) | unique"

# 1. A board, a child document of it, and a document at the top level.
scratch_collection
q item create --collection "$SCRATCH" --kind board --title "Embed board"
expect_rc 0
board="$(val .id)"
q item create --collection "$SCRATCH" --kind doc --parent "$board" --title "Embedded notes" --md - <<<"A kangaroo in the notes."
expect_rc 0
expect ".item.parent_id == \"$board\" and .item.title == \"Embedded notes\""
child="$(val .item.id)"
q item create --collection "$SCRATCH" --kind doc --title "Shared page"
expect_rc 0
shared="$(val .id)"
ok "a board, a child document of the board (item create --parent --md) and a top-level document"

# 2. The fixture with its two placeholder ids replaced.
sent="$work/sent.inkboard"
jq -c --arg p1 "$P1" --arg p2 "$P2" --arg a "$child" --arg b "$shared" \
  'walk(if . == $p1 then $a elif . == $p2 then $b else . end)' "$fixture_board" >"$sent" ||
  _fail "jq could not write $sent"
q board put "$board" - --if-match r1 <"$sent"
expect_rc 0
expect '.rev == 2 and .etag == "\"r2\""'
ok "board put of the embed fixture, its targets replaced, with --if-match r1"

# 3. Both cards come back, the plain box has no embed, the name is the title, the rest is what was sent.
q board get "$board"
expect_rc 0
expect ".content | $cards == [\"$child\", \"$shared\"]"
expect '.content.document.pages[0].objects[2].kind | has("embed") | not'
expect '.content.document.name == "Embed board"'
jq -S '.content' "$OUT_FILE" >"$work/got.json"
jq -S '.document.name = "Embed board"' "$sent" >"$work/want.json"
cmp -s "$work/got.json" "$work/want.json" || _fail "board get differs from the board sent (apart from the name)"
etag="$(val .etag)"
ok "board get keeps both cards' kind.embed.item and the plain box without one; document.name is the title"

# 4. The title snapshots are the board's text.
q item text "$board"
expect_rc 0
expect '(.body | contains("First page")) and (.body | contains("Second page"))'
q search "Second page" --collection "$SCRATCH"
expect_rc 0
expect "any(.items[]; .item.id == \"$board\")"
ok "item text of the board holds both title snapshots, and search \"Second page\" finds the board"

# 5. No value of embed is refused, and every one is kept as it was sent.
malformed="$work/malformed.inkboard"
jq -c --arg p "$shared" '
  .document.pages[0].objects[0] as $card
  | .document.pages[0].objects += [
      ($card | .id = "e12-number" | .kind.embed = 5 | .kind.frame.y = 400),
      ($card | .id = "e12-not-uuid" | .kind.embed = {item: "not-a-uuid"} | .kind.frame.y = 800),
      ($card | .id = "e12-ellipse" | .kind.geom = "ellipse" | .kind.embed = {item: $p} | .kind.frame.y = 1200)
    ]' "$sent" >"$malformed" || _fail "jq could not write $malformed"
q board put "$board" - --if-match "$etag" <"$malformed"
expect_rc 0
q board get "$board"
expect_rc 0
for oid in e12-number e12-not-uuid e12-ellipse; do
  got="$(jq -cS --arg i "$oid" '.content.document.pages[0].objects[] | select(.id == $i)' "$OUT_FILE")"
  want="$(jq -cS --arg i "$oid" '.document.pages[0].objects[] | select(.id == $i)' "$malformed")"
  [ -n "$want" ] && [ "$got" = "$want" ] || _fail "the object $oid is not kept as it was sent"
done
expect '[.content.document.pages[0].objects[] | select(.id == "e12-number") | .kind.embed] == [5]'
jq -S '.content' "$OUT_FILE" >"$work/got.json"
jq -S '.document.name = "Embed board"' "$malformed" >"$work/want.json"
cmp -s "$work/got.json" "$work/want.json" || _fail "board get differs from the malformed board sent"
etag="$(val .etag)"
jq -S '.content' "$OUT_FILE" >"$work/board-5.json"
ok "a board with \"embed\": 5, {\"item\": \"not-a-uuid\"} and an ellipse with a valid embed is stored and kept exactly"

# 6. The preview shows a card as its titled box.
for _ in $(seq 1 30); do
  q board preview "$board" --page 0 --out /tmp/agent-e12-p0.svg
  if [ "$RC" = 0 ] && jq -e '(.content_type // "") | startswith("image/svg+xml")' "$OUT_FILE" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
expect_rc 0
expect '(.content_type | startswith("image/svg+xml")) and .bytes > 0'
q_raw e12-p0.svg board preview "$board" --page 0
expect_rc 0
grep -q "First page" "$AGENT_RESULTS/e12-p0.svg" || _fail "the page 0 preview does not show \"First page\""
ok "board preview --page 0 is image/svg+xml and holds the card's title \"First page\""

# 7. A duplicate's cards show the same documents (its children are not copied).
q item duplicate "$board"
expect_rc 0
expect '.child_count == 0'
copy="$(val .id)"
q board get "$copy"
expect_rc 0
expect ".content | $linked == ([\"$child\", \"$shared\"] | sort)"
jq -S '.content | del(.document.name)' "$OUT_FILE" >"$work/copy.json"
jq -S 'del(.document.name)' "$work/board-5.json" >"$work/original.json"
cmp -s "$work/copy.json" "$work/original.json" || _fail "the copy's board differs from the original's"
ok "item duplicate of the board: the copy's board get has the same two targets (and the same objects)"

# 8. Trash and restore follow the tree: the child goes with the board, the shared page stays.
q item trash "$board"
expect_rc 0
q item get "$child"
expect_rc 0
expect '.trashed_at != null'
q item get "$shared"
expect_rc 0
expect '.trashed_at == null'
q item restore "$board"
expect_rc 0
q item get "$child"
expect_rc 0
expect '.trashed_at == null'
ok "item trash of the board trashes the child, not the shared page; item restore brings the child back"

# 9. Moving the child out of the board leaves the board as it was.
q item move "$child" --top
expect_rc 0
expect '.parent_id == null'
q board get "$board"
expect_rc 0
expect ".etag == $(jq -n --arg e "$etag" '$e')"
jq -S '.content' "$OUT_FILE" >"$work/after-move.json"
cmp -s "$work/after-move.json" "$work/board-5.json" || _fail "moving the child changed the board"
ok "item move of the child to the top level: board get is unchanged"

# 10. AGENTS.md §8's jq recipe, run as it is written, with the scenario's ids.
recipe="$work/recipe.sh"
awk '/runs the next block as it is written/ {f = 1; next}
     f && /^```sh$/ {b = 1; next}
     b && /^```$/ {exit}
     b' "$AGENT_ROOT/docs/AGENTS.md" >"$recipe"
grep -q 'board put "\$B"' "$recipe" || _fail "docs/AGENTS.md has no recipe block after its marker"
# The recipe's `wikanvas --json …` is the harness's CLI (q's runner): stdout printed, the exit code returned.
wikanvas() {
  [ "${1:-}" = --json ] || {
    echo "the recipe runs wikanvas without --json" >&2
    return 2
  }
  shift
  _run 1 "$work/.recipe-out" "$@"
  cat "$work/.recipe-out"
  return "$RC"
}
# run_recipe BOARD DOC: the recipe in $work, its last stdout (the board put's) in $work/recipe-put.json.
run_recipe() {
  (
    cd "$work" || exit 1
    B="$1"
    DOC="$2"
    # shellcheck disable=SC1090
    . "$recipe"
  ) >"$work/recipe-put.json" 2>"$work/recipe.err"
}
q board get "$board"
expect_rc 0
jq -cS '.content.document.pages[0].objects' "$OUT_FILE" >"$work/before-recipe.json"
run_recipe "$board" "$shared" || _fail "the AGENTS.md recipe failed: $(tail -n 3 "$work/recipe.err")"
jq -e '.rev > 0' "$work/recipe-put.json" >/dev/null 2>&1 || _fail "the recipe's board put printed no rev"
q board get "$board"
expect_rc 0
expect ".content.document.pages[0].objects[-1] | .kind.type == \"box\" and .kind.geom == \"rect\"
  and .kind.embed == {item: \"$shared\"} and .kind.label.text[0].text == \"Shared page\""
[ "$(jq -cS '.content.document.pages[0].objects[:-1]' "$OUT_FILE")" = "$(cat "$work/before-recipe.json")" ] ||
  _fail "the recipe changed an object that was already on the board"
expect ".content | $linked == ([\"$child\", \"$shared\"] | sort)"
ok "AGENTS.md's jq recipe appends a card for the shared page; every earlier object of page 0 is unchanged"

# 10b. The recipe's title snapshot is the one the web client writes (§18.7: the title trimmed, else "Untitled").
q item create --collection "$SCRATCH" --kind doc --title "  Padded  title  "
expect_rc 0
expect '.title == "  Padded  title  "'
padded="$(val .id)"
run_recipe "$board" "$padded" || _fail "the AGENTS.md recipe failed for the padded title: $(tail -n 3 "$work/recipe.err")"
q board get "$board"
expect_rc 0
expect ".content.document.pages[0].objects[-1] | .kind.embed.item == \"$padded\" and .kind.label.text[0].text == \"Padded  title\""
ok "the recipe's title snapshot leaves out the title's leading and trailing spaces, as the web client's does"

# 11. A card keeps a new, empty, untitled document from being discarded; without the card it goes.
q item create --collection "$SCRATCH" --kind doc
expect_rc 0
expect '.title == ""'
spare="$(val .id)"
run_recipe "$board" "$spare" || _fail "the AGENTS.md recipe failed for the spare document: $(tail -n 3 "$work/recipe.err")"
q board get "$board"
expect_rc 0
expect ".content.document.pages[0].objects[-1] | .kind.embed.item == \"$spare\" and .kind.label.text[0].text == \"Untitled\""
etag="$(val .etag)"
jq -c --arg s "$spare" '.content | .document.pages[0].objects |= map(select(.kind.embed | type == "object" and .item == $s | not))' \
  "$OUT_FILE" >"$work/without-spare.inkboard" || _fail "jq could not write without-spare.inkboard"
q item trash "$spare" --discard-empty
expect_rc 8
expect_err '.error.code == "not_empty"'
q item get "$spare"
expect_rc 0
expect '.trashed_at == null'
ok "item trash --discard-empty of a new, empty document that a card shows exits 8 (not_empty) and keeps it"
q board put "$board" - --if-match "$etag" <"$work/without-spare.inkboard"
expect_rc 0
q item trash "$spare" --discard-empty
expect_rc 0
q item get "$spare"
expect_rc 7
ok "after a board put without that card, item trash --discard-empty exits 0 and item get exits 7"
