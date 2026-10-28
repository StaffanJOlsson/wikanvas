#!/usr/bin/env bash
# 05: search: a prefix, an excluded word, titles, and --all over several pages.
. "$AGENT_ROOT/e2e/agent/lib.sh"

md="$(fixture quokka.md)" || exit 1
scratch_collection
q item create --collection "$SCRATCH" --kind doc --title "Island notes" --md - <"$md"
expect_rc 0
id="$(val .item.id)"
ok "a document containing quokka"

q search quok
expect_rc 0
expect ".items[0].item.id == \"$id\""
ok "search quok lists that doc first"

q search "quok -zebra"
expect_rc 0
expect "any(.items[]; .item.id == \"$id\")"
ok "search \"quok -zebra\" still finds it"

q search --titles Island
expect_rc 0
expect "any(.[]; .item.id == \"$id\")"
ok "search --titles finds it"

for i in 1 2 3 4; do
  q item create --collection "$SCRATCH" --kind doc --title "quokka $i"
  expect_rc 0
done
q search quokka --all --limit 2
expect_rc 0
expect '(.items | length) >= 5 and .next_cursor == null'
ok "--all merges pages"
