# cadastre

Keeps a folder of Markdown up to date with what your Docker containers are
running. One page per Compose stack, an overview with warnings, and a
changelog of what changed. Built for people who keep their notes in Obsidian
(or any folder of Markdown) and are tired of documentation that was true last
spring.

- **Always current.** It reacts to Docker events, and reconciles on a timer in
  case it missed one.
- **Never contains your secrets.** Environment variables are listed by name
  only, labels are allowlisted, and a canary test fails the build if a secret
  ever reaches the output. [Threat model](docs/threat-model.md).
- **Read-only.** It talks to a [socket proxy](compose.example.yaml) that cannot
  create, stop or exec anything.
- **Fails loudly.** A dead Docker connection, an empty answer or an unmounted
  folder makes the container unhealthy and the metrics say so, instead of
  leaving stale notes that look fine. [Failure modes](docs/failure-modes.md).
- **Small.** One Rust binary, a 64 MB memory limit, no database, no
  LLM, no network access beyond the proxy.

## What it writes

An overview page, with the things worth a second look:

```markdown
# Containers overview

4 containers in 2 stacks.

| Stack | Containers |
|---|---|
| [`shop`](stacks/shop.md) | 2 |
| [`standalone`](stacks/_standalone.md) | 2 |

## Warnings

- `node-agent` uses host networking: every port it listens on is exposed on the host.
- `uptime-probe` publishes `9115/tcp` on all interfaces (`0.0.0.0:9115`).
```

and one page per stack:

```markdown
## `shop-web-1`

- **Image:** `ghcr.io/example/web:2.4.1` (id `aaaaaaaaaaaa`)
- **Restart policy:** `unless-stopped`
- **Networks:** `proxy`, `shop_default`
- **Environment variables (names only):** `APP_ENV`, `DATABASE_URL`, `SECRET_KEY_BASE`

| Container port | Published on |
|---|---|
| `8080/tcp` | `127.0.0.1:8080` |
```

plus `changelog.md`:

```markdown
## 2026-03-02T09:14:07Z

- Added `shop-web-1` (`ghcr.io/example/web:2.4.1`)
- Image of `shop-db-1` changed: `postgres:17.1 (17.1)` → `postgres:17.2 (17.2)`
```

## Quick start

1. Copy [`compose.example.yaml`](compose.example.yaml). Set the output folder
   to a folder inside your vault, and `user:` to the uid:gid that owns it.
2. `docker compose up -d`
3. Open the folder. The first run happens at startup.
4. Start or stop any container and watch the page change about 30 seconds
   later.

With Syncthing, Obsidian flavour and metrics, see
[Obsidian and Syncthing](docs/obsidian-syncthing.md),
[configuration](docs/configuration.md) and [Prometheus](docs/prometheus.md).

Without Docker Compose: `cadastre once` documents the containers once and
exits; `cadastre run` is the service.

## How it works

```
Docker events ──▶ debounce (30 s) ──┐
hourly timer ───────────────────────┼──▶ list + inspect ──▶ sanitize ──▶ render
retry after a failed run ───────────┘                          │            │
                                                     secrets stop here      ▼
                                                                  atomic, write-if-changed
```

The sanitizer is the only code that sees a raw `docker inspect` document. It
produces a small typed model; renderers cannot reach anything the model does
not hold.

## Status

Version 0.1. It documents containers, stacks, images, ports, mounts, networks
and Traefik routing. Not included yet: summaries written by a language model,
user-defined templates, Podman. See the open issues.

This is maintained on a best-effort basis. Issues and pull requests are
welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

[AGPL-3.0-or-later](LICENSE).
