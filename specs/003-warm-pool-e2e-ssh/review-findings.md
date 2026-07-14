# Deep Review Findings

**Date:** 2026-07-13
**Branch:** 6113-warm-pool-grpc-poc
**Rounds:** 0
**Gate Outcome:** PASS
**Invocation:** superpowers

## Summary

| Severity | Found | Fixed | Remaining |
|----------|-------|-------|-----------|
| Critical | 0 | 0 | 0 |
| Important | 0 | 0 | 0 |
| Minor | 2 | - | 2 |
| Notable | 3 | - | 3 |
| **Total** | **5** | **0** | **5** |

**Agents completed:** 5/5 (+ 1 external tool)
**Agents failed:** none

## Findings

### FINDING-1
- **Severity:** Minor
- **Confidence:** 70
- **File:** crates/openshell-sandbox/src/activation.rs:182-196
- **Category:** correctness
- **Source:** correctness-agent
- **Round found:** 1
- **Resolution:** acknowledged

**What is wrong:**
If the spawned bootstrap task panics, the oneshot sender drops without sending. The `Ok(exit_code) = activation_rx =>` pattern in run_unidentified() (lib.rs:1012) silently skips Err(RecvError), potentially causing the process to hang.

**Why this matters:**
Low probability (requires panic in post_identity_bootstrap). Kubernetes liveness probes would reap the process. Acceptable for PoC scope.

**How to resolve:**
Add `Err(_) = activation_rx => { return Ok(1); }` to the select block.

### FINDING-2
- **Severity:** Minor
- **Confidence:** 65
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs:1-159
- **Category:** test-quality
- **Source:** test-quality-agent
- **Round found:** 1
- **Resolution:** acknowledged

**What is wrong:**
`free_port()` and `check_readyz()` helpers are duplicated between activation_smoke.rs and activation_timing.rs.

**Why this matters:**
Acceptable for two test files. Rust integration tests compile as separate binaries; sharing requires tests/common/mod.rs.

**How to resolve:**
Extract to tests/common/mod.rs if more integration tests are added.

### FINDING-3
- **Severity:** Notable
- **Confidence:** 90
- **File:** crates/openshell-sandbox/src/activation.rs:198-203
- **Category:** architecture
- **Source:** architecture-agent
- **Round found:** 1
- **Resolution:** by design

**What is wrong:**
gRPC response (success:true) sent before post_identity_bootstrap() completes.

**Why this matters:**
Intentional design for low latency. Driver uses warm pool claim status for readiness, not the gRPC response.

### FINDING-4
- **Severity:** Notable
- **Confidence:** 85
- **File:** crates/openshell-sandbox/src/activation.rs:46-203
- **Category:** security
- **Source:** security-agent
- **Round found:** 1
- **Resolution:** out of scope per spec

**What is wrong:**
No authentication on ActivateSandbox gRPC endpoint.

**Why this matters:**
Explicitly deferred per spec ("Plaintext gRPC acceptable for PoC"). Endpoint only reachable within pod network. sandbox_token validated by gateway during bootstrap.

### FINDING-5
- **Severity:** Notable
- **Confidence:** 80
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs:1-159
- **Category:** test-quality
- **Source:** test-quality-agent
- **Round found:** 1
- **Resolution:** acknowledged

**What is wrong:**
No test for successful full bootstrap (requires real gateway). Only failure paths tested in CI.

**Why this matters:**
Infrastructure constraint. Smoke test proves real code path exercised (GATEWAY_UNREACHABLE). Full E2E covered by cluster tasks T012-T015.
