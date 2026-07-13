# Brainstorm Overview

Last updated: 2026-07-13

## Sessions

| # | Date | Topic | Status | Spec | Issue |
|---|------|-------|--------|------|-------|
| 01 | 2026-07-09 | warm-pool-feasibility | active | - | - |
| 02 | 2026-07-09 | cluster-setup | active | - | - |
| 03 | 2026-07-09 | warm-pool-measurements | active | - | - |
| 04 | 2026-07-09 | results-and-recommendations | active | - | - |
| 05 | 2026-07-10 | k8s-watch-crash-fix | active | - | [#2211](https://github.com/NVIDIA/OpenShell/issues/2211) |
| 06 | 2026-07-11 | warm-pool-grpc-poc | active | 002 | - |
| 07 | 2026-07-11 | warm-pool-sandbox-profile | active | - | - |
| 08 | 2026-07-13 | warm-pool-e2e-ssh | active | - | - |

## Structure

01 is the parent document defining the overall feasibility study. 02-04 are execution phases:

- **02** depends on: nothing (first step)
- **03** depends on: 02 (cluster must be running)
- **04** depends on: 03 (measurements must be complete)

05 is a standalone bug fix discovered during the feasibility study.
06 is the Milestone 1 gRPC PoC (gateway-side flow, spec 002).
07 explores the SandboxProfile entity model for warm pool configuration.
08 is the Milestone 2 plan for E2E SSH demo with full bootstrap extraction.

## Open Threads

- Does the Red Hat Agent Sandbox operator tech preview include extension CRDs? (from #01, #02)
- Does env var injection at claim time trigger cold start? (from #01, #03) -- confirmed yes
- Is KEP-753 (native sidecars) available on the target OpenShift version? (from #01, #02)
- How does pool exhaustion behave (cold fallback vs. Pending)? (from #03)
- Should findings be posted to upstream agent-sandbox repo? (from #04)
- Should the defensive skip use `debug!` or `warn!` level? (from #05)
- How much of run_sandbox() parameter setup can be shared vs reconstructed? (from #08)
- Does the SSH relay path work with claimed pod IP or need Sandbox CRD name? (from #08)
- What's the actual latency impact of proxy/nftables setup at activation time? (from #08)

## Parked Ideas

None.
