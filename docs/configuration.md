# Configuration

cadastre is configured with environment variables only. There are no hidden
defaults for URLs or paths: a missing required variable is a startup error
that names the variable.

| Variable | Required | Default | Meaning |
|---|---|---|---|
| `CADASTRE_DOCKER_URL` | yes | `http://socket-proxy:2375` in the image | Base URL of the Docker API, or a comma-separated list of `[name=]url` endpoints (see [Several servers](#several-servers)). Plain `http://` only; point it at a read-only socket proxy. `https://`, `unix://` and `ssh://` are rejected. |
| `CADASTRE_OUT_DIR` | yes | `/out` in the image | Folder that receives the Markdown. Must already exist: a missing folder is an error, so an unmounted volume cannot silently fill the container's own disk. |
| `CADASTRE_STATE_DIR` | yes | `/state` in the image | Folder for the snapshot of the previous run. Keep it on a volume, or every restart records an "initial snapshot". |
| `CADASTRE_METRICS_DIR` | no | unset | Folder read by node_exporter's textfile collector. Metrics are off when unset. See [prometheus.md](prometheus.md). |
| `CADASTRE_HEARTBEAT_FILE` | no | `/tmp/cadastre.heartbeat` | File read by `cadastre healthcheck`. Must be writable. |
| `CADASTRE_DEBOUNCE_SECONDS` | no | `30` | Quiet period after the last Docker event before regenerating. `docker compose up` on twenty containers produces one run. |
| `CADASTRE_RECONCILE_SECONDS` | no | `3600` | Interval of the unconditional run (minimum 1). It bounds how stale the documentation can get if the event stream dies unnoticed. |
| `CADASTRE_SHRINK_GUARD_PERCENT` | no | `50` | Refuse to publish if the container count falls below this percentage of the previous snapshot. `0` disables the guard. |
| `CADASTRE_OBSIDIAN` | no | `false` | Add Obsidian front matter and `[[wikilinks]]`. Accepts `1/true/on/yes` and `0/false/off/no`; anything else is an error. |

## Several servers

`CADASTRE_DOCKER_URL` accepts several endpoints, separated by commas, each with
an optional friendly name:

```
CADASTRE_DOCKER_URL=nas=http://nas:2375,vps=http://10.0.0.2:2375
```

Endpoints without a name are named after their host and port; names are unique
and made of letters, digits, `.`, `_` and `-`. With a single, unnamed endpoint
the output is unchanged. With several:

- containers of all endpoints are documented together; stacks with the same
  Compose project name share a page, with a `Server` column and a
  `**Server:**` line per container;
- the index has a `Servers` table with the container count and status of each
  endpoint, and the changelog and warnings name containers as `name@server`;
- an unreachable endpoint does not stop the others: it is logged, marked
  `unreachable (last known data)` in the index, and its containers keep their
  entries from the previous run, so nothing is reported as removed. The run
  fails only when no endpoint answers;
- the event stream of every endpoint triggers a regeneration.

Each remote endpoint should be a read-only socket proxy, as for a single one.

## Commands

```
cadastre run                     # the service
cadastre once [--accept-shrink]  # one run, then exit
cadastre healthcheck             # exit 0 if a running service is healthy
```

`once` does not write metrics or the heartbeat. `--accept-shrink` skips the
drop guard for a deliberate teardown.

## What is written

```
<OUT_DIR>/
  index.md            overview and warnings
  changelog.md        what changed, newest first
  docker/<stack>.md   one page per Compose project
  .tmp/               scratch space for atomic writes
```

Containers that were not started by Compose are grouped in a `_standalone`
page. cadastre only deletes pages under `docker/` that carry its generated
marker; anything you wrote yourself is left alone. An existing `changelog.md`
that does not carry the marker is never overwritten: the run fails instead.

If the folder is synchronised, add `.tmp` to the sync tool's ignore list.
