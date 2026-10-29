# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue: use **Report a vulnerability** on the repository's
[Security tab](https://github.com/StaffanJOlsson/wikanvas/security/advisories/new) (GitHub's private vulnerability
reporting). Include the version (`wikanvas-server --version`, or Settings → About), how Wikanvas is deployed (the
bundled compose project, behind another proxy, LAN or public), the steps to reproduce, and what an attacker gains.

You should get a first answer within a week. Fixes are made in private, released, and then the advisory is published
with credit to the reporter, unless you prefer otherwise.

## Supported versions

Wikanvas is an early alpha. Only the latest release and `main` get security fixes; there are no backports.

| Version | Supported |
|---|---|
| latest `0.1.x` alpha | yes |
| older | no |

## Scope

In scope: the server (`wikanvas-server`), the web client, the `wikanvas` CLI, the container image and the compose
setup as shipped. Out of scope: problems that need an already compromised host or Docker daemon, a self-chosen weak
configuration (for example exposing the app's port 8080 without Caddy), and issues in third-party software that are not
reachable through Wikanvas (report those upstream; [`THIRD_PARTY.md`](THIRD_PARTY.md) lists what Wikanvas uses).
