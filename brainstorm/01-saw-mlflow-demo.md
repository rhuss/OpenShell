# Brainstorm: SAW + MLflow Demo on ROSA

**Date:** 2026-08-08
**Status:** active

## Problem Framing

We need a repeatable, one-command way to deploy the Secure Agent Workspace (SAW) validated pattern with MLflow tracing on a fresh ROSA HCP cluster. The goal is to demonstrate OpenShell's OTLP integration with MLflow, showing agent interaction traces in a real multi-tenant environment.

The deployment must be documented as a reproducible recipe so anyone on the team can spin up the full demo stack. It should also support a fast development loop for iterating on OpenShell's agent-level instrumentation, since that work is expected to require significant iteration with code changes deployed quickly into the SAW environment.

### Context from prior work

- SAW integration plan exists at `architecture/plans/saw-otlp-mlflow-integration.md`
- Derek Carr's architecture call established per-workspace OTLP endpoints as the production design
- Saurabh Agarwal's fork (`fix/nemoclaw-dual-gateway` branch, PR #18) is the working deployment
- The ROSA plugin (`cc-rosa`) already has instills for `rhoai`, `mlflow`, and `openshell`
- MLflow 3.x has built-in bidirectional OTLP support (`/v1/traces` endpoint)

## Approaches Considered

### A: Single monolithic instill

One `saw` instill handles everything: cluster creation, RHOAI, MLflow, SAW clone+install, OTLP config, dev loop.

- Pros: One command, no coordination between parts
- Cons: Violates composable instill model. Can't reuse existing `rhoai`/`mlflow` instills. Hard to skip steps on a partially-set-up cluster. The install.md would be enormous.

### B: Composable instills + orchestrating recipe

A focused `saw` instill handles only SAW-specific concerns. Cluster creation and RHOAI/MLflow use existing instills. A recipe YAML chains them for the one-shot experience.

- Pros: Each instill stays focused and reusable. Existing `rhoai` and `mlflow` instills work as-is. Can run `rosa:install saw` independently on any cluster with prerequisites. Recipe documents the full sequence.
- Cons: Needs the recipe mechanism (script exists but no recipes directory yet). Two new artifacts: the instill + the recipe.

### C: Instill with dependency auto-chaining

The `saw` instill declares `requires: [rhoai, mlflow]` and the dispatcher auto-installs missing deps.

- Pros: Simple UX, automatic dependency resolution
- Cons: Can't handle cluster creation (that's `/rosa:create`, not an instill). GPU machinepool addition is also separate. Still needs manual steps for the cluster lifecycle.

## Decision

**Approach B: Composable instills + orchestrating recipe.**

Each piece stays independently useful. The recipe gives the one-shot experience for fresh clusters. The `saw` instill can be run standalone on any cluster that already has RHOAI + MLflow.

## Key Requirements

### SAW instill (`~/.config/cc-rosa/instills/saw/`)

- **Location**: User-level instill, available across all projects
- **Wrapper pattern**: Clones SAW repo, configures it, calls `make install`. Does not reimplement SAW's ArgoCD-based deployment.
- **SAW source**: Saurabh's fork (`sauagarwa/secure-agent-workspace`), branch `fix/nemoclaw-dual-gateway`
- **Inference provider**: Configurable at install time (NVIDIA Build API for no-GPU clusters, or on-cluster vLLM via existing `model` instill)
- **OTLP integration**: Gateway-level traces to MLflow via `[openshell.gateway.otlp]` in `cloudinit-sandbox.yaml`
- **Dev loop (SSH+binary)**: Build OpenShell locally, scp gateway/supervisor binaries into the running VM, restart services. Fast iteration default.
- **Dev loop (container image)**: Build a container image from local source, push to cluster registry or quay.io, update VM config. Triggered via `--build-image` flag. More robust, slower cycle.

### Orchestrating recipe

- Creates a fresh ROSA HCP cluster (latest supported OCP version, `aaet` profile)
- Adds GPU machinepool if on-cluster inference is selected
- Installs RHOAI, MLflow, SAW in dependency order
- Lives at `~/.config/cc-rosa/recipes/saw-demo.yaml` (or equivalent location)

### Install steps (SAW instill)

1. Clone SAW repo (Saurabh's fork, `fix/nemoclaw-dual-gateway` branch)
2. Configure `values-secret.yaml` (SSH keys, inference provider, API keys)
3. Run `make copy-images && make install`
4. Wait for SAW deployment (VM running, gateway accessible)
5. Configure `[openshell.gateway.otlp]` pointing at MLflow cluster service
6. Create OpenShift Route for MLflow UI
7. Verify traces appear in MLflow

## Open Questions

- Should the recipe support `demo.redhat.com` clusters as an alternative to ROSA HCP? The SAW plan documents this path but it requires a different provisioning flow.
- When Saurabh's PR #18 merges to upstream, the instill should switch to the upstream repo. How to handle that transition (config flag? auto-detect?).
- The Red Hat-built OpenShell gateway/supervisor doesn't work with NemoClaw yet (upstream NVIDIA builds required). When that's resolved, the dev loop becomes more relevant.
- NemoClaw requires Docker CE (not Podman). This is baked into the SAW VM image. If we switch to custom images for the dev loop, we need to preserve that constraint.
- Martin Jackson write access to the SAW repo is pending. Needed for pushing changes upstream.
