#!/usr/bin/env bash
# 09: export: a tar with manifest.json and wikanvas.db for admin; exit 6 for the write token.
. "$AGENT_ROOT/e2e/agent/lib.sh"

q_raw export.tar --as admin export
expect_rc 0
listing="$(tar -tf "$AGENT_RESULTS/export.tar" 2>/dev/null)" || _fail "export is not a tar"
grep -qx 'manifest.json' <<<"$listing" || _fail "the tar has no manifest.json"
grep -qx 'wikanvas.db' <<<"$listing" || _fail "the tar has no wikanvas.db"
ok "export is a tar with manifest.json and wikanvas.db"

q --as write export --out /tmp/agent-export.tar
expect_rc 6
expect_err '.error.code == "insufficient_scope"'
ok "export with the write token exits 6"
