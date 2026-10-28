#!/usr/bin/env bash
# 02: collections and the tree: A, B under A, C under B; move B and back; a cycle exits 8; duplicate C.
. "$AGENT_ROOT/e2e/agent/lib.sh"

scratch_collection
q item create --collection "$SCRATCH" --kind doc --title A
expect_rc 0
a="$(val .id)"
q item create --collection "$SCRATCH" --kind doc --title B --parent "$a"
expect_rc 0
b="$(val .id)"
q item create --collection "$SCRATCH" --kind doc --title C --parent "$b"
expect_rc 0
c="$(val .id)"
ok "created A, B under A and C under B"

q tree "$SCRATCH"
expect_rc 0
expect "(.items | length) == 3
  and ([.items[] | select(.id == \"$a\")][0].parent_id == null)
  and ([.items[] | select(.id == \"$b\")][0].parent_id == \"$a\")
  and ([.items[] | select(.id == \"$c\")][0].parent_id == \"$b\")"
ok "tree --json has three items with the right parents"

q item move "$b" --top
expect_rc 0
expect '.parent_id == null'
q item move "$b" --parent "$a"
expect_rc 0
expect ".parent_id == \"$a\""
ok "move B to the top level and back"

q item move "$a" --parent "$c"
expect_rc 8
expect_err '.error.code == "cycle" and .exit_code == 8'
ok "moving A under C exits 8 with cycle"

q item duplicate "$c"
expect_rc 0
expect ".id != \"$c\" and .kind == \"doc\""
ok "duplicate C"
