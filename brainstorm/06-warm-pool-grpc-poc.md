# Brainstorm: Warm Pool gRPC PoC (Milestone 1)

**Date:** 2026-07-11
**Status:** active

## Problem Framing

The feasibility study proved that claiming a pre-provisioned pod from a
SandboxWarmPool takes ~1.4s (operator reconciliation), compared to ~16.7s
for a full OpenShell cold start. The main blocker is identity binding:
`spec.env` on SandboxClaim bypasses the warm pool entirely.

Two RFCs proposed solutions (annotation-based at ~3s, gRPC-push at sub-2s).
NVIDIA feedback from the Slack thread (2026-07-10) confirmed:

- **+1 on gRPC-push** (Andrew Newberry): "This is the direction we want to
  move the supervisor to."
- **Unidentified supervisor state** (Andrew): Warm pods should run with an
  unidentified supervisor, identity arrives at claim time. No pre-compiled
  global OPA (policies "may become out of date by the time the sandbox
  comes online").
- **Validation from internal POC** (Dhiraj Bokde): They've used pooled
  pods for months, <1s claim latency confirmed, "pod has to be re-IDed
  once claimed."

This PoC proves the claim-time gRPC flow end-to-end before wiring the
entity model (see brainstorm 07).

## Approaches Considered

### A: ActivateSandbox with Unidentified Supervisor (chosen)

Supervisor starts in an unidentified/unbound state in warm pool pods.
No gateway connection, no OPA compilation, no identity. At claim time,
the gateway discovers the pod IP from the SandboxClaim status, calls
`ActivateSandbox` with sandbox ID, JWT, name, and full policy config.
The supervisor bootstraps from there (OPA compile, gateway connect).

- Pros: Aligns with NVIDIA direction (Andrew's unidentified state
  proposal). Avoids stale global policy problem. Simpler supervisor
  lifecycle (idle until activated). Clean separation between pool-time
  and claim-time.
- Cons: All OPA compilation happens at claim time (~100-200ms estimated
  for sandbox-specific policies without global pre-compilation). Slightly
  higher claim latency than two-tier OPA, but simpler.

### B: Two-Tier OPA (original RFC proposal)

Supervisor starts at pool time, pre-compiles global OPA policies, waits
for ActivateSandbox with only the sandbox-specific delta.

- Pros: Lowest possible claim-time latency (global policies already
  compiled).
- Cons: Andrew explicitly pushed back: "policies may become out of date."
  More complex supervisor lifecycle (must handle policy refresh, partial
  state). Requires the supervisor to know what "global" means at pool
  time without gateway context.

### C: Annotation-based (companion RFC)

Supervisor starts at claim time, detects identity via downward API
annotations. No new gRPC endpoint.

- Pros: Simplest implementation (~18 story points). No new proto surface.
- Cons: ~3-4s latency (1.5s supervisor startup at claim time). NVIDIA
  prefers gRPC-push direction. Downward API propagation adds 1-2s.

## Decision

**Approach A: ActivateSandbox with Unidentified Supervisor.**

This aligns with NVIDIA's stated direction and Andrew's explicit
recommendation. The two-tier OPA optimization (Approach B) can be
re-evaluated later as a latency optimization if claim-time OPA
compilation becomes a bottleneck, but the unidentified state is the
right starting point.

## Key Requirements

### Supervisor Changes

1. **Unidentified startup mode**: Supervisor process starts without
   gateway connection, identity, or OPA policies. Listens on a gRPC
   port and exposes `/readyz` once ready to receive activation.

2. **ActivateSandbox gRPC endpoint**: New RPC on the supervisor that
   receives sandbox identity (ID, name, JWT) and policy configuration.
   On receipt, the supervisor:
   - Stores identity
   - Compiles OPA policies from the provided config
   - Calls `IssueSandboxToken`, `GetSandboxConfig`, etc. against the
     gateway
   - Calls `ConnectSupervisor` to register the session
   - Returns success/failure to the caller

3. **Readiness signaling**: Pod readinessProbe checks `/readyz` which
   returns 200 when the supervisor is listening for `ActivateSandbox`
   (not when the sandbox is fully bootstrapped).

### Gateway Changes (K8s Driver)

4. **Claim-time activation flow**: After SandboxClaim binds and reports
   Ready with a pod IP, the gateway:
   - Reads pod IP from claim status
   - Calls `ActivateSandbox` on the supervisor
   - Waits for success response
   - Returns sandbox as ready to the CLI

5. **Pod IP discovery**: Read from `.status.sandbox.podIPs` on the
   SandboxClaim (or `.status.sandbox.podIP`).

6. **mTLS for ActivateSandbox**: Use the existing namespace mTLS
   certificates for the gateway-to-supervisor channel.

### Proto Changes

7. **New ActivateSandbox RPC**: Added to the supervisor service proto.
   Request carries sandbox ID, name, JWT, policy config, gateway
   endpoint. Response carries success/failure and error details.

### Pool Setup (Manual for PoC)

8. **Manual pool creation**: For the PoC, pools are created manually
   via kubectl (SandboxTemplate + SandboxWarmPool). No automated
   lifecycle management yet (that's milestone 2).

9. **Supervisor image in warm pods**: The warm pool SandboxTemplate
   must include the supervisor container configured to start in
   unidentified mode (new CLI flag or env var).

### Cold-Start Fallback

10. **Fallback path**: If no warm pool exists for the requested image,
    or readyReplicas is 0, fall back to the existing cold-start path.
    The two paths must coexist.

## Open Questions

- What is the exact proto definition for `ActivateSandbox`? Should it
  be a new service or added to the existing supervisor service?
- How does the supervisor discover the gateway endpoint at activation
  time? Passed in the ActivateSandbox request, or via env var at pool
  provisioning time?
- Should the supervisor support a timeout for activation (e.g., kill
  the pod if not activated within N minutes to prevent resource waste)?
- Related upstream work: issue #1955 (legacy RPC cleanup) may affect
  where the new RPC is added. Coordinate with Craig's work.
- How to handle the case where ActivateSandbox fails (supervisor
  crashes, OPA compilation error)? Should the gateway retry or fall
  back to cold start?
