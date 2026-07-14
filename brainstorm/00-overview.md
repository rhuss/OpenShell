# Brainstorm Overview

Last updated: 2026-07-11

## Sessions

| # | Date | Topic | Status | Spec | Issue |
|---|------|-------|--------|------|-------|
| 01 | 2026-07-09 | warm-pool-feasibility | active | - | - |
| 02 | 2026-07-09 | cluster-setup | active | - | - |
| 03 | 2026-07-09 | warm-pool-measurements | active | - | - |
| 04 | 2026-07-09 | results-and-recommendations | active | - | - |
| 05 | 2026-07-10 | warm-pool-grpc-push | active | - | - |
| 06 | 2026-07-11 | warm-pool-grpc-poc | active | - | - |
| 07 | 2026-07-11 | warm-pool-sandbox-profile | active | - | - |

## Structure

01 is the parent document defining the overall feasibility study. 02-04 are execution phases:

- **02** depends on: nothing (first step)
- **03** depends on: 02 (cluster must be running)
- **04** depends on: 03 (measurements must be complete)

05 is the RFC brainstorm for always-on supervisor with gRPC-push identity binding.

06-07 are prototype milestones incorporating NVIDIA feedback (2026-07-10):

- **06** (Milestone 1): ActivateSandbox gRPC PoC with unidentified supervisor. Proves the claim-time flow end-to-end with manual pool setup.
- **07** (Milestone 2): SandboxProfile entity + workspace-scoped pool lifecycle. Builds on 06, makes pools mergeable upstream. Depends on Derek's workspace PR.

## Open Threads

- Does the Red Hat Agent Sandbox operator tech preview include extension CRDs? (from #01, #02)
- Does env var injection at claim time trigger cold start? (from #01, #03)
- Is KEP-753 (native sidecars) available on the target OpenShift version? (from #01, #02)
- How does pool exhaustion behave (cold fallback vs. Pending)? (from #03)
- Should findings be posted to upstream agent-sandbox repo? (from #04)
- What is the exact proto definition for ActivateSandbox? (from #05, #06)
- How does the supervisor discover the gateway endpoint at activation time? (from #06)
- Should the supervisor support an activation timeout to prevent resource waste? (from #06)
- Coordinate with Craig's work on issue #1955 (legacy RPC cleanup) (from #06)
- What is the relationship between SandboxProfile and SandboxTemplate? (from #07)
- Does Derek's workspace PR define a workspace proto that references SandboxProfiles? (from #07)
- How to handle pool config for workspaces spanning multiple namespaces? (from #07)
- Pool utilization metrics: SandboxProfile status field vs. metric endpoint? (from #07)

## Parked Ideas

None.
