# Prometheus

Set `CADASTRE_METRICS_DIR` to the folder read by node_exporter's textfile
collector (`--collector.textfile.directory`). cadastre rewrites
`cadastre.prom` in it after every run, successful or not, atomically: the file
is written to a scratch folder and renamed, so the collector never reads half
of it.

## Metrics

| Metric | Meaning |
|---|---|
| `cadastre_last_success_timestamp_seconds` | Unix time of the last run that published successfully. |
| `cadastre_last_attempt_timestamp_seconds` | Unix time of the last run, successful or not. |
| `cadastre_last_attempt_success` | `1` if the last run succeeded, `0` if it failed. |
| `cadastre_containers` | Containers in the last successful snapshot. |
| `cadastre_build_info{version}` | Always `1`; carries the running version. |

A series with no value is **omitted**, not reported as `0`. After a restart,
`cadastre_last_success_timestamp_seconds` does not exist until the first run
succeeds. This is deliberate: a made-up zero would look like "succeeded long
ago", while an absent series can be alerted on.

The metrics carry no `instance` label; the scraper adds its own.

## Alert rules

[`examples/prometheus-alerts.yml`](../examples/prometheus-alerts.yml) has four
rules:

- **CadastreStale**: no successful publication for two hours.
- **CadastreLastRunFailed**: the last run failed.
- **CadastreMetricsAbsent**: `absent(cadastre_build_info)`. Without this rule a
  dead service, or a collector pointed at the wrong folder, would simply stop
  producing alerts and look healthy.
- **CadastreNeverSucceeded**: running, but no success since start.

Validate your copy with `promtool check rules` before loading it.

## Test the alert path

An alert that has never fired is an assumption. After installing:

1. Stop the socket proxy: `docker compose stop socket-proxy`.
2. Within about a minute the container turns unhealthy and
   `cadastre_last_attempt_success` becomes `0`.
3. After the rule's `for:` duration, confirm the notification arrived in the
   channel you actually read, not just that the alert shows as firing.
4. Start the proxy again and confirm the alert resolves on its own.
5. Stop cadastre itself and confirm `CadastreMetricsAbsent` fires.

Skip a step and you have a monitoring setup you hope works.
