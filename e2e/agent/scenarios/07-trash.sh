#!/usr/bin/env bash
# 07: trash, restore and purge of the scenario's own items (never --all).
. "$AGENT_ROOT/e2e/agent/lib.sh"

scratch_collection
q item create --collection "$SCRATCH" --kind doc --title B
expect_rc 0
b="$(val .id)"
q item create --collection "$SCRATCH" --kind doc --title C --parent "$b"
expect_rc 0
c="$(val .id)"

q item trash "$b"
expect_rc 0
q tree "$SCRATCH"
expect_rc 0
expect "all(.items[]; .id != \"$b\" and .id != \"$c\")"
q trash list
expect_rc 0
expect "any(.items[]; .id == \"$b\" and .descendant_count == 1)"
ok "trashing B hides B and C; trash list shows B with one descendant"

q item restore "$b"
expect_rc 0
q tree "$SCRATCH"
expect_rc 0
expect "any(.items[]; .id == \"$b\") and any(.items[]; .id == \"$c\")"
ok "item restore brings both back"

q item trash "$b"
expect_rc 0
q --as admin trash purge "$b"
expect_rc 2
expect_err '.error.code == "cli_usage"'
q --as write trash purge "$b" --yes
expect_rc 6
q --as admin trash purge "$b" --yes
expect_rc 0
expect ".ok == true and .id == \"$b\""
q item get "$b"
expect_rc 7
ok "trash purge needs --yes (2) and admin (6); with both it is gone"
