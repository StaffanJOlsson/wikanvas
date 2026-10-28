#!/usr/bin/env bash
# 10: tokens: the secret once (masked in the report), --token-out, list, revoke, admin limits, revocation of children.
. "$AGENT_ROOT/e2e/agent/lib.sh"

q --as admin token create --name agent-10 --scope read --expires 1d
expect_rc 0
expect '(.token | test("^qpat_[0-9A-Za-z]{46}$")) and .api_token.scopes == ["read"]'
t="$(val .token)"
tid="$(val .api_token.id)"
ok "token create --scope read --expires 1d prints qpat_… once"
grep -qF "$t" "$AGENT_STEPS" && _fail "the report holds the secret unmasked"
ok "the report shows the token masked"

q --as admin token create --name agent-10-file --scope read --expires 1d --token-out /tmp/agent-t.token
expect_rc 0
expect '.token_file == "/tmp/agent-t.token" and .token == null and (.api_token.prefix | startswith("qpat_"))'
grep -Eq 'qpat_[0-9A-Za-z]{46}' <<<"$OUT$ERR" && _fail "--token-out printed a secret"
ok "--token-out prints no secret"

q --as admin token list
expect_rc 0
expect "any(.[]; .id == \"$tid\" and .prefix == \"${t:0:12}\")"
grep -Eq 'qpat_[0-9A-Za-z]{46}' <<<"$OUT" && _fail "token list printed a secret"
ok "token list shows the prefix and never the secret"

q --as admin token revoke "$tid"
expect_rc 0
q --token "$t" auth whoami
expect_rc 5
ok "token revoke, then reuse exits 5"

q --as admin token create --name agent-10-admin --scope admin --expires 1d
expect_rc 6
expect_err '.error.code == "session_required"'
ok "the admin token minting an admin token exits 6"

q_admin create-token --name agent-10-parent --scope admin --expires 2d
expect_rc 0
parent="$(val .token)"
parent_id="$(val .api_token.id)"
q --token "$parent" token create --name agent-10-child --scope read --expires 1d
expect_rc 0
child="$(val .token)"
q --token "$child" auth whoami
expect_rc 0
q --as admin token revoke "$parent_id"
expect_rc 0
q --token "$child" auth whoami
expect_rc 5
ok "a child of a second admin token exits 5 once that parent is revoked"
