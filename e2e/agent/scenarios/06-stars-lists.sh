#!/usr/bin/env bash
# 06: stars and lists; items are published on create; unpublish makes drafts; publish --with-ancestors (D23).
. "$AGENT_ROOT/e2e/agent/lib.sh"

scratch_collection
q item create --collection "$SCRATCH" --kind doc --title Parent
expect_rc 0
parent="$(val .id)"
q item create --collection "$SCRATCH" --kind doc --title Child --parent "$parent"
expect_rc 0
child="$(val .id)"

q star add "$child"
expect_rc 0
expect ".ok == true and .id == \"$child\""
q list starred
expect_rc 0
expect "any(.items[]; .id == \"$child\")"
ok "star add prints the ok object; list starred has it"

q item get "$child"
expect_rc 0
expect '.effectively_published == true and .item.published == true'
q sidebar
expect_rc 0
drafts="$(val .drafts_count)"
q list drafts
expect_rc 0
expect "all(.items[]; .id != \"$child\")"
ok "a new doc is published on create and is not a draft"

q item unpublish "$child"
expect_rc 0
q item unpublish "$parent"
expect_rc 0
q list drafts
expect_rc 0
expect "any(.items[]; .id == \"$child\") and any(.items[]; .id == \"$parent\")"
q sidebar
expect_rc 0
expect ".drafts_count == ($drafts + 2)"
ok "unpublishing the doc and its parent puts both in list drafts (drafts_count + 2)"

q item publish "$child" --with-ancestors
expect_rc 0
q list drafts
expect_rc 0
expect "all(.items[]; .id != \"$child\" and .id != \"$parent\")"
q sidebar
expect_rc 0
expect ".drafts_count == $drafts"
ok "item publish --with-ancestors empties list drafts again"
