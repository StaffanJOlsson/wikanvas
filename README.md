# Quire

A self-hosted wiki: a sidebar of collections holding documents and
[Inkboard](https://github.com/VictorLetirand/inkboard) whiteboards, nested to any depth. The server is Rust (axum) on
SQLite; the frontend is React with the TipTap editor; boards run Inkboard compiled to WebAssembly.

**Status:** milestone 1 is being built and nothing is released yet. This README is a stub; the full one (installation
with Docker Compose, HTTPS through the bundled Caddy, configuration, runtime limits, backups, the API and the `quire`
CLI) comes with the end of milestone 1.

## Design

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): the architecture and decisions of milestone 1.
- [`docs/PLAN.md`](docs/PLAN.md): the implementation plan and its workstreams.

## Building

Builds never run on the host. Every cargo, node and pnpm command runs in a resource-capped container through
`scripts/dev.sh` (for example `scripts/dev.sh cargo build --workspace --locked`), and Docker images are built only
through the scripts in `scripts/`. `CLAUDE.md` lists the rules.

## Licence

Quire is licensed under the MIT License ([`LICENSE`](LICENSE)). Third-party software, Inkboard included, is listed in [`THIRD_PARTY.md`](THIRD_PARTY.md).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Quire by you shall be
licensed under the MIT License as above, without any additional terms or conditions.
