# Contributing to Wikanvas

Thanks for helping. Wikanvas is an early alpha with one maintainer, so please open an issue to discuss anything larger
than a small fix before you write it; that saves us both a rewrite.

## Before you start

- **Bugs and ideas:** open an issue with the templates. For security problems, use private reporting instead
  ([`SECURITY.md`](SECURITY.md)).
- **Design:** the architecture and every decision are in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (the D-numbered
  decisions in §2.1); proposals in progress are under `docs/` (for example `docs/multi-user.md`).

## Building and testing

Builds never run on the host: every cargo, node and pnpm command runs in a resource-capped container through
`scripts/dev.sh`. The README's [Building and the build caps](README.md#building-and-the-build-caps) section explains the
scripts, and [`CLAUDE.md`](CLAUDE.md) lists the rules every contributor and coding agent follows (no builds inside
`vendor/inkboard`, formatting only with `scripts/fmt.sh`, and so on). The checks CI runs, which a pull request has to
pass:

```sh
scripts/fmt.sh --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo test --workspace --locked
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test
scripts/image.sh && scripts/e2e.sh smoke          # the browser tests against the production image
```

`.github/workflows/ci.yml` has the full list (the image smoke, the agent scenarios and the board frame tests too), and
docs/ARCHITECTURE.md §14 describes every test layer.

## Pull requests

- Branch from `main` and keep each pull request to one change. Write commit messages that say what changed and why.
- Add or update tests with the change, and update the docs it affects (the README, `docs/ARCHITECTURE.md`,
  `docs/AGENTS.md` for the CLI, `docs/api/openapi.json` through its test).
- `main` is protected: a commit reaches it only after all three CI jobs have passed on it, history stays linear, and
  force pushes are refused.
- The API is versioned (`/api/v1`): changes to it are additive. The database migrations are append-only.

## Licence and third-party code

Wikanvas is MIT. By contributing you agree that your contribution is licensed under the MIT License, as the README's
[Licence](README.md#licence) section says. Do not copy code from projects under the AGPL, the BSL or another copyleft
or source-available licence; reading them for ideas is fine. A change to `Cargo.lock` or `web/pnpm-lock.yaml` also
updates [`THIRD_PARTY.md`](THIRD_PARTY.md), and every new dependency's licence must be permissive (its last section
says how to check).
