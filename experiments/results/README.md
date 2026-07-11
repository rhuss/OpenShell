# Experiment Results

Each subdirectory contains one complete test run, named by date and cluster
configuration.

## Directory naming convention

```
YYYY-MM-DD-<platform>-<ocp-version>
```

Example: `2026-07-09-rosa-hcp-422` is a run on ROSA HCP with OpenShift 4.22.

## CSV files per run

| File | Experiment | Description |
|------|-----------|-------------|
| `cold-start-prepulled.csv` | Phase 3 | OpenShell cold start, images pre-pulled |
| `cold-start-noprepull.csv` | Phase 3 | OpenShell cold start, no image pre-pull |
| `cold-start-vanilla.csv` | Phase 3 | Vanilla Agent Sandbox (pause image, K8s baseline) |
| `warm-pool-default.csv` | Phase 4 | Warm pool claim, default probes |
| `warm-pool-burst.csv` | Phase 4 | Warm pool burst (5 simultaneous claims) |
| `readiness-gates.csv` | Phase 5 | Readiness gate condition flip latency |
| `env-injection-allowed.csv` | Phase 6 | Env var injection with Allowed policy |
| `env-injection-disallowed.csv` | Phase 6 | Env var injection with Disallowed policy |

## CSV columns

```
run,experiment,config,create_ts,ready_ts,delta_ms,pod,status,scheduled_ms,pulled_ms,created_ms,started_ms,phase5_ms
```

- `delta_ms`: End-to-end latency from create to ready (milliseconds)
- `status`: `ok`, `timeout`, `create-failed`, `pool-exhausted`, `bind-timeout`
- Phase columns: Per-phase deltas from pod events (when available)

## Reproducing a run

See the [setup instructions](../RESULTS.md#how-to-reproduce) in the main
results document.
