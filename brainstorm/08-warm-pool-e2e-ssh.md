# Brainstorm: Warm Pool E2E with SSH (Milestone 2)

**Date:** 2026-07-13
**Status:** active

## Problem Framing

Milestone 1 proved the gateway-side warm pool flow works: pool
detection (18ms), SandboxClaim (240ms), ActivateSandbox gRPC (105ms),
ConnectSupervisor (3ms). The sandbox reaches Ready in ~350ms on the
gateway side. But the CLI hangs because the supervisor's
`bootstrap_sandbox()` only does gateway registration, not the
in-sandbox networking stack (SSH, proxy, OPA enforcement, entrypoint).

Without SSH, we can't demonstrate the end-to-end experience to
external customers. The demo that matters is a side-by-side comparison:
cold start (~17s) vs warm pool (~2s), both dropping into an interactive
SSH shell. That's the moment that sells the approach.

### What works today (Milestone 1)

- Supervisor starts in `--unidentified` mode (<1s to readiness)
- Gateway detects warm pool, claims pod, calls ActivateSandbox
- Supervisor receives identity (real gateway-minted JWT) and registers
  with ConnectSupervisor
- Sandbox reaches Ready state on the gateway side
- Cold-start fallback works when no pool or activation fails

### What's missing

1. **SSH listener**: the CLI connects to the sandbox via SSH relay.
   Without an SSH listener, the CLI can't open a shell.
2. **HTTP proxy**: egress traffic routes through the proxy with OPA
   policy enforcement. Required for inference routing and network
   policy.
3. **OPA enforcement**: policies are compiled in `bootstrap_sandbox()`
   but never wired into the proxy/network stack.
4. **Entrypoint process**: the user's shell or command needs to start.
5. **Networking setup**: nftables rules, DNS, proxy configuration.

All of these are handled by `run_sandbox()` (lib.rs, ~650 lines) in
the cold-start path. `bootstrap_sandbox()` needs the same setup.

### Related: gRPC authentication gap

The inbox item `supervisor-grpc-authentication` notes that
ActivateSandbox has no caller authentication. Deferred to a future
milestone (plaintext is acceptable for the PoC demo).

## Approaches Considered

### A: Extract full bootstrap from `run_sandbox()` (chosen)

Extract the networking/SSH/proxy/entrypoint startup steps from
`run_sandbox()` into shared functions that both `run_sandbox()` (cold
start) and `bootstrap_sandbox()` (warm pool) call after identity is
established.

- Pros: full functional parity with cold start, best timing, reusable
  refactoring, the demo shows identical sandbox behavior
- Cons: touching a 650-line monolith risks cold-start regressions,
  4-6 hours of careful work
- Estimate: 4-6 hours

### B: Minimal SSH-only bootstrap

Add only SSH listener + entrypoint to `bootstrap_sandbox()`. Skip
proxy and OPA enforcement.

- Pros: less code, lower regression risk, 2-3 hours
- Cons: no policy enforcement, no inference routing, not true parity

### C: Process restart shim

After activation, restart the supervisor in normal mode with env vars
set. The supervisor runs the exact same cold-start path.

- Pros: zero extraction, guaranteed parity, very little new code
- Cons: adds ~1.5s restart latency (total ~1.85s), negates some warm
  pool benefit

## Decision

**Approach A: full bootstrap extraction.** The demo needs to be
convincing for external customers, which means full parity. The
refactoring is valuable beyond the PoC since the `run_sandbox()`
monolith needs decomposition regardless. Lessons learned will feed
into a targeted upstream PR.

## Key Requirements

1. `openshell sandbox create --name X --from base` drops into an SSH
   shell when a warm pool exists, in under 3 seconds
2. Side-by-side demo: cold start (~17s) vs warm pool (~2-3s), both
   with interactive SSH
3. Full functional parity: SSH, proxy, OPA, entrypoint
4. Cold-start path unchanged (no regressions)
5. Plaintext gRPC for activation (mTLS deferred)
6. Capture lessons for future targeted PR

## Open Questions

- How much of `run_sandbox()`'s parameter setup can be shared vs
  reconstructed from the activation request?
- Does the proxy need the full gateway config, or can it bootstrap
  from `GetSandboxConfig` alone?
- Will the SSH relay path work with the claimed pod's IP, or does
  the gateway need the Sandbox CRD's name for routing?
- What's the actual latency impact of proxy/nftables setup at
  activation time?
