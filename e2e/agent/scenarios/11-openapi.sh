#!/usr/bin/env bash
# 11: the OpenAPI document through `wikanvas api`, a missing route, and the path guard.
. "$AGENT_ROOT/e2e/agent/lib.sh"

q --as read api GET /api/v1/openapi.json
expect_rc 0
expect '.openapi == "3.1.0" and (.paths | keys | all(startswith("/api/v1/"))) and .components.securitySchemes.bearer != null'
ok "api GET /api/v1/openapi.json is OpenAPI 3.1.0 under /api/v1/ with the bearer scheme"

q --as read api GET /api/v1/nope
expect_rc 7
expect_err '.error.code == "not_found" and .status == 404'
ok "api GET /api/v1/nope exits 7"

q --as read api GET //example.invalid/x
expect_rc 2
expect_err '.error.code == "cli_usage"'
ok "api GET //example.invalid/x exits 2"
