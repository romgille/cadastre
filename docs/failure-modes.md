# Failure modes

The design rule is that a documentation tool must not fail quietly. A folder
of notes that stopped updating months ago looks exactly like a folder that is
up to date. For each way things can go wrong, this page says what you will
see.

| What goes wrong | What happens | Loud or silent |
|---|---|---|
| Docker or the socket proxy is unreachable, or answers 5xx | The run fails. The old pages stay. The heartbeat says `failing`, so the container becomes **unhealthy**; `cadastre_last_attempt_success` is `0`; the error is logged. Retried every 60 s. | Loud |
| The proxy refuses with 403 | Same, and the error says which proxy sections to enable (`CONTAINERS=1`, `EVENTS=1`). | Loud |
| The proxy answers with an empty or much shorter list and no error | The shrink guard refuses to publish (below 50% of the previous snapshot by default). Nothing is deleted. Same signals as above. | Loud |
| The output folder is missing (volume not mounted) | The run fails; it never creates the folder. The snapshot is not updated. | Loud |
| `changelog.md` exists and was not written by cadastre | The run fails before writing anything. | Loud |
| The snapshot is corrupt or from an incompatible version | The run fails before writing anything, rather than resetting the history. | Loud |
| The process is stuck | The heartbeat goes stale; the health check fails after 300 s. | Loud |
| The process is dead | The container exits and restarts. If metrics are enabled, the file stops updating and the alert rules fire (see [prometheus.md](prometheus.md)). | Loud, if you alert on it |
| The event stream dies or hangs | A warning is logged and reconnection is attempted every 5 s. Documentation still updates at every reconcile (default hourly). The container stays healthy. | **Quiet degradation** |
| The output folder is mounted, but it is not the folder your sync tool watches | cadastre cannot know. | **Silent** |
| The sync tool stops syncing | Outside cadastre's view. | **Silent** |
| A crash between writing the changelog and saving the snapshot | The next run repeats one changelog entry. Duplicate rather than lost. | Harmless |

## The two quiet cases

The event stream is the only part whose failure does not turn the container
unhealthy: the periodic reconcile deliberately covers for it, so the
documentation is at worst one reconcile interval stale. Lower
`CADASTRE_RECONCILE_SECONDS` if that is too long.

The mount mismatch is a deployment error. Check once, after installing, that a
change to a container shows up in the place you read the notes.

## Prove the alerts work

A notification path is not finished until it has been broken on purpose. After
setting up the alerts, stop the socket proxy and confirm that the failure
arrives through the channel you actually read. See
[prometheus.md](prometheus.md#test-the-alert-path).
