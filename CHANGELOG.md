# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.1.0]

First release.

### Added

- Per-stack Markdown pages, an overview with warnings (host networking, ports
  published on all interfaces) and a changelog of what changed.
- A sanitizer that builds a small typed model from `docker inspect`:
  environment variables by name only, allowlisted labels, canary tests that
  fail if a secret reaches any output.
- Optional Obsidian flavour (front matter and wikilinks).
- Event-driven updates with a debounce, a periodic reconcile and retry after
  failures.
- Idempotent, atomic output: unchanged pages are not rewritten; only pages
  carrying the generated marker are ever deleted.
- A shrink guard that refuses to publish when the container count collapses.
- Prometheus metrics for the textfile collector, a heartbeat-based health
  check, and example alert rules with `absent()` guards.
- Hardened input handling: Traefik rules kept only with known-safe matchers,
  credential-bearing URLs and invisible Unicode characters dropped, and a
  Docker client that follows no redirects, ignores proxy environment variables
  and caps what the daemon can send.
- Pages grouped in a folder per source (`docker/`), so that future sources
  cannot delete each other's pages.
- A container image (distroless, non-root) and an example deployment with a
  read-only socket proxy.

[0.1.0]: https://github.com/romgille/cadastre/releases/tag/v0.1.0
