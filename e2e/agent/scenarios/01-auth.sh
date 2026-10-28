#!/usr/bin/env bash
# 01: authentication and scopes: whoami per token, insufficient_scope, an unknown, a malformed and a revoked token.
. "$AGENT_ROOT/e2e/agent/lib.sh"

for scope in read write admin; do
  case "$scope" in
    read) want='["read"]' ;;
    write) want='["read","write"]' ;;
    admin) want='["read","write","admin"]' ;;
  esac
  q --as "$scope" auth whoami
  expect_rc 0
  expect ".auth.method == \"token\" and .auth.token.scopes == $want and .user.email == \"agent@quire.invalid\""
  ok "auth whoami as $scope shows its scopes"
done

q --as read collection create --name "agent 01 read"
expect_rc 6
expect_err '.error.code == "insufficient_scope" and .exit_code == 6 and .status == 403'
ok "a read token creating a collection exits 6 with insufficient_scope"

q --token "$(unknown_token)" auth whoami
expect_rc 5
expect_err '.error.code == "invalid_token" and .status == 401'
ok "a well-formed unknown token exits 5"

q --token qpat_notAToken auth whoami
expect_rc 3
expect_err '.error.code == "cli_config" and .request_id == null'
ok "a malformed token exits 3 without a request"

q --as admin token create --name agent-01 --scope read --expires 1d
expect_rc 0
t="$(val .token)"
id="$(val .api_token.id)"
q --token "$t" auth whoami
expect_rc 0
q --as admin token revoke "$id"
expect_rc 0
expect ".ok == true and .id == \"$id\""
q --token "$t" auth whoami
expect_rc 5
ok "a token revoked with token revoke exits 5"
