#!/usr/bin/env bash
# 08: attachments: upload from stdin, reference from Markdown, download with the same sha256.
. "$AGENT_ROOT/e2e/agent/lib.sh"

png="$(fixture pixel.png)" || exit 1
q attachment upload - --filename pixel.png <"$png"
expect_rc 0
expect '(.id | length) == 36 and .mime == "image/png" and .width == 1 and .height == 1'
att="$(val .id)"
sha="$(val .sha256)"
ok "attachment upload - of pixel.png"

scratch_collection
q item create --collection "$SCRATCH" --kind doc --title Pixel
expect_rc 0
id="$(val .id)"
q doc put "$id" --md - --force <<<"![pixel](/api/v1/attachments/$att)"
expect_rc 0
q doc get "$id"
expect_rc 0
expect ".content | contains(\"/api/v1/attachments/$att\")"
ok "a Markdown PUT that references the attachment round-trips"

q_raw pixel.out attachment get "$att"
expect_rc 0
[ "$(sha256sum "$AGENT_RESULTS/pixel.out" | cut -d' ' -f1)" = "$sha" ] || _fail "attachment get has another sha256"
[ "$sha" = "$(sha256sum "$png" | cut -d' ' -f1)" ] || _fail "the stored file differs from the fixture"
ok "attachment get has the same sha256"
