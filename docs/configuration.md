# Configuration

cadastre is configured with environment variables only. There are no hidden
defaults for URLs or paths: a missing required variable is a startup error
that names the variable.

| Variable | Required | Default | Meaning |
|---|---|---|---|
| `CADASTRE_DOCKER_URL` | yes | `http://socket-proxy:2375` in the image | Base URL of the Docker API. Plain `http://` only; point it at a read-only socket proxy. `https://` and `unix://` are rejected. |
| `CADASTRE_OUT_DIR` | yes | `/out` in the image | Folder that receives the Markdown. Must already exist: a missing folder is an error, so an unmounted volume cannot silently fill the container's own disk. |
| `CADASTRE_STATE_DIR` | yes | `/state` in the image | Folder for the snapshot of the previous run. Keep it on a volume, or every restart records an "initial snapshot". |
| `CADASTRE_METRICS_DIR` | no | unset | Folder read by node_exporter's textfile collector. Metrics are off when unset. See [prometheus.md](prometheus.md). |
| `CADASTRE_HEARTBEAT_FILE` | no | `/tmp/cadastre.heartbeat` | File read by `cadastre healthcheck`. Must be writable. |
| `CADASTRE_DEBOUNCE_SECONDS` | no | `30` | Quiet period after the last Docker event before regenerating. `docker compose up` on twenty containers produces one run. |
| `CADASTRE_RECONCILE_SECONDS` | no | `3600` | Interval of the unconditional run (minimum 1). It bounds how stale the documentation can get if the event stream dies unnoticed. |
| `CADASTRE_SHRINK_GUARD_PERCENT` | no | `50` | Refuse to publish if the container count falls below this percentage of the previous snapshot. `0` disables the guard. |
| `CADASTRE_OBSIDIAN` | no | `false` | Add Obsidian front matter and `[[wikilinks]]`. Accepts `1/true/on/yes` and `0/false/off/no`; anything else is an error. |

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
