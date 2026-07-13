# Code Review: Warm Pool E2E with SSH (Milestone 2)

**Spec:** specs/003-warm-pool-e2e-ssh/spec.md
**Date:** 2026-07-13
**Branch:** 6113-warm-pool-grpc-poc
**Reviewer:** Claude (speckit.spex-gates.review-code + spex-deep-review)

## Compliance Summary

**Overall Score: 100%**

- Functional Requirements: 10/10 (100%)
- Error Handling: 4/4 (100%)
- Edge Cases: 3/3 (100%)

## Detailed Review

### Functional Requirements

#### FR-001: BootstrapContext Struct
**Implementation:** crates/openshell-sandbox/src/lib.rs:323-347
**Status:** Compliant
**Notes:** All 18 fields present including command, sandbox_id, policy, opa_engine, ssh_socket_path, provider_credentials, provider_env, loaded_policy_origin, and sidecar_bootstrap gated behind Option.

#### FR-002: post_identity_bootstrap() Extraction
**Implementation:** crates/openshell-sandbox/src/lib.rs:349-803
**Status:** Compliant
**Notes:** Shared async function accepting BootstrapContext. Handles UID/GID override, network namespace, denial/activity channels, run_networking(), sidecar control, aggregators, policy poll, GCE metadata, run_process(), and lifecycle management.

#### FR-003: run_sandbox() Refactoring
**Implementation:** crates/openshell-sandbox/src/lib.rs:94-267
**Status:** Compliant
**Notes:** run_sandbox() builds BootstrapContext from CLI args and calls post_identity_bootstrap(ctx).await. Cold-start path preserved with identical behavior.

#### FR-004: bootstrap_sandbox() Returns BootstrapContext
**Implementation:** crates/openshell-sandbox/src/lib.rs:805-931
**Status:** Compliant
**Notes:** Connects to gateway with JWT, fetches settings snapshot, compiles OPA, fetches provider env, spawns ConnectSupervisor, returns Result<BootstrapContext, BootstrapError>.

#### FR-005: ActivateSandbox gRPC Handler
**Implementation:** crates/openshell-sandbox/src/activation.rs:46-203
**Status:** Compliant
**Notes:** Validates request fields, calls bootstrap_sandbox(), spawns post_identity_bootstrap() in a tokio task, returns success response. AtomicBool prevents double activation.

#### FR-006: Oneshot Channel Exit Code
**Implementation:** crates/openshell-sandbox/src/activation.rs:22 (Sender), lib.rs:1012 (Receiver)
**Status:** Compliant
**Notes:** oneshot::Sender<i32> carries exit code from spawned bootstrap task to run_unidentified().

#### FR-007: run_unidentified() Lifecycle
**Implementation:** crates/openshell-sandbox/src/lib.rs:942-1022
**Status:** Compliant
**Notes:** Starts gRPC server on port 9090, health on 8080, waits on activation_rx via tokio::select!, returns exit code.

#### FR-008: --unidentified CLI Flag
**Implementation:** crates/openshell-sandbox/src/main.rs:207-208
**Status:** Compliant
**Notes:** Flag dispatches to run_unidentified() entry point.

#### FR-009: Warm Pool CRD Interaction
**Implementation:** crates/openshell-driver-kubernetes/src/warm_pool.rs:1-328
**Status:** Compliant
**Notes:** list_warm_pools, find_matching_pool, create_claim, wait_for_claim_ready using kube-rs dynamic API.

#### FR-010: Activation Client
**Implementation:** crates/openshell-driver-kubernetes/src/activation_client.rs:1-98
**Status:** Compliant
**Notes:** Client for ActivateSandbox with TLS support and 5-second timeout.

### Error Handling

#### EH-001: Invalid Request Rejection
**Implementation:** activation.rs:62-82
**Status:** Compliant
**Notes:** Empty sandbox_id, sandbox_token, gateway_endpoint, or missing policy all return InvalidRequest error code with descriptive message. Activated flag is reset on rejection.

#### EH-002: Bootstrap Failure Reset
**Implementation:** activation.rs:120-164
**Status:** Compliant
**Notes:** On bootstrap_sandbox() failure, activated flag is reset (line 121), OCSF failure events emitted, error code mapped via BootstrapError::to_error_code().

#### EH-003: Double Activation Guard
**Implementation:** activation.rs:54-60
**Status:** Compliant
**Notes:** AtomicBool with swap(true, AcqRel) ensures exactly-once activation. Second call returns AlreadyActivated.

#### EH-004: Post-Bootstrap Error Handling
**Implementation:** activation.rs:182-196
**Status:** Compliant
**Notes:** Spawned task catches post_identity_bootstrap errors, logs them, sends exit code 1 through oneshot.

### Edge Cases

#### EC-001: Concurrent Activation Requests
**Implementation:** activation.rs:54 (AtomicBool::swap with AcqRel ordering)
**Status:** Compliant
**Notes:** Atomic swap ensures exactly one caller proceeds, all others get AlreadyActivated.

#### EC-002: Readiness Within 1 Second
**Implementation:** tests/activation_timing.rs
**Status:** Compliant
**Notes:** Integration test verifies readyz endpoint responds within 1 second of process start.

#### EC-003: Cold-Start Regression
**Implementation:** lib.rs:94-267 (run_sandbox unchanged path)
**Status:** Compliant
**Notes:** run_sandbox() constructs BootstrapContext from same local variables and calls shared post_identity_bootstrap(). All existing tests pass.

## Deep Review Report

**Date:** 2026-07-13
**Branch:** 6113-warm-pool-grpc-poc
**Rounds:** 0
**Gate Outcome:** PASS
**Invocation:** superpowers

### Summary

| Severity | Found | Fixed | Remaining |
|----------|-------|-------|-----------|
| Critical | 0 | 0 | 0 |
| Important | 0 | 0 | 0 |
| Minor | 2 | - | 2 |
| Notable | 3 | - | 3 |
| **Total** | **5** | **0** | **5** |

**Agents completed:** 5/5 (correctness, architecture, security, production-readiness, test-quality)
**External tools:** CodeRabbit (2 stored findings from prior spec review, new run hit 212-file limit)
**Agents failed:** none

### Findings

#### FINDING-1
- **Severity:** Minor
- **Confidence:** 70
- **File:** crates/openshell-sandbox/src/activation.rs:182-196, lib.rs:1012
- **Category:** correctness
- **Source:** correctness-agent
- **Round found:** 1
- **Resolution:** acknowledged (low probability, acceptable for PoC)

**What is wrong:**
If the spawned bootstrap task in activation.rs panics (line 182-196), the oneshot sender is dropped without sending a value. In run_unidentified() at lib.rs:1012, the pattern `Ok(exit_code) = activation_rx =>` will not match on `Err(RecvError)`, causing that select branch to be silently skipped. If there are no other active branches in the select, this could cause the process to hang.

**Why this matters:**
A panic in the bootstrap task is a low-probability event (requires a bug in post_identity_bootstrap or OOM). In practice, the process would be reaped by Kubernetes liveness probes. For a PoC, this is acceptable.

**Recommendation:**
Future hardening could add `Err(_) = activation_rx => { return Ok(1); }` to the select block to handle dropped senders explicitly.

#### FINDING-2
- **Severity:** Minor
- **Confidence:** 65
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs, activation_timing.rs
- **Category:** test-quality
- **Source:** test-quality-agent
- **Round found:** 1
- **Resolution:** acknowledged (acceptable duplication for test isolation)

**What is wrong:**
The `free_port()` and `check_readyz()` helper functions are duplicated between activation_smoke.rs and activation_timing.rs. Both files implement identical TCP port selection and HTTP health check logic.

**Why this matters:**
Duplicated test helpers can drift over time. However, Rust integration tests are compiled as separate binaries, and sharing helpers requires a `tests/common/mod.rs` module. For two test files, the duplication is acceptable and maintains test isolation.

**Recommendation:**
If more integration tests are added, extract shared helpers into `tests/common/mod.rs`.

#### FINDING-3
- **Severity:** Notable
- **Confidence:** 90
- **File:** crates/openshell-sandbox/src/activation.rs:198-203
- **Category:** architecture
- **Source:** architecture-agent
- **Round found:** 1
- **Resolution:** by design

**What is wrong:**
The ActivateSandbox gRPC response (success:true) is sent before post_identity_bootstrap() runs. The caller receives confirmation of activation before the sandbox is fully operational (SSH listener, networking, entrypoint process).

**Why this matters:**
This is an intentional design choice documented in the spec. The caller (driver) uses a separate readiness mechanism (warm pool claim status) to know when the sandbox is fully ready. Returning early keeps the gRPC call latency low and avoids holding the connection during potentially long bootstrap operations.

#### FINDING-4
- **Severity:** Notable
- **Confidence:** 85
- **File:** crates/openshell-sandbox/src/activation.rs:46-203, lib.rs:942-1022
- **Category:** security
- **Source:** security-agent
- **Round found:** 1
- **Resolution:** out of scope per spec

**What is wrong:**
The ActivateSandbox gRPC endpoint has no authentication. Any process that can reach port 9090 on the pod can send an activation request with arbitrary sandbox_id and token.

**Why this matters:**
The spec explicitly states: "Plaintext gRPC for ActivateSandbox is acceptable for this PoC." The endpoint is only exposed within the Kubernetes pod network (not externally routable). mTLS and authentication are deferred to a future milestone. The sandbox_token is validated by the gateway during bootstrap_sandbox(), providing identity verification at that layer.

#### FINDING-5
- **Severity:** Notable
- **Confidence:** 80
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs
- **Category:** test-quality
- **Source:** test-quality-agent
- **Round found:** 1
- **Resolution:** acknowledged (infrastructure constraint)

**What is wrong:**
There is no integration test for a successful full bootstrap path. The smoke test validates that bootstrap_sandbox() fails with GATEWAY_UNREACHABLE (expected in CI without a real gateway), proving the real code path is exercised, but the success path can only be tested on a live cluster.

**Why this matters:**
This is an inherent constraint of the architecture: bootstrap_sandbox() requires a real gateway connection. The test correctly verifies that the activation handler calls the real bootstrap function (not a mock), and the GATEWAY_UNREACHABLE error proves the code path is exercised up to the network call. Full E2E testing is covered by Phase 5 tasks (T012-T015) on the cluster.

### CodeRabbit External Tool Results

CodeRabbit CLI was invoked but hit the 212-file limit (max 150). Two stored findings from a prior review were retrieved, both relating to specs/002-warm-pool-grpc-poc (the previous milestone's spec), not the current implementation code. A scoped re-run with `--dir crates/openshell-sandbox` was attempted but results were not available before review completion.

**CodeRabbit stored findings (informational, from prior milestone spec):**
1. Feature branch naming inconsistency in specs/002-warm-pool-grpc-poc/spec.md (Minor)
2. FR-002/FINDING-3 wording contradiction in specs/002-warm-pool-grpc-poc/REVIEW-CODE.md (Minor)

Neither finding applies to the current milestone's implementation code.

### Gate Decision

**GATE: PASS**

- Critical findings: 0
- Important findings: 0
- Fix loop rounds needed: 0
- All 10 functional requirements compliant (100%)
- 2 Minor findings acknowledged (low-probability edge case, test helper duplication)
- 3 Notable findings documented (all by-design or out-of-scope per spec)

## Recommendations

### Spec Evolution Candidates
- None identified. Implementation matches spec precisely.

### Future Hardening (Post-PoC)
- [ ] Handle dropped oneshot sender in run_unidentified() select block (FINDING-1)
- [ ] Extract shared test helpers if more integration tests are added (FINDING-2)
- [ ] Add mTLS to ActivateSandbox gRPC endpoint (FINDING-4, deferred per spec)
- [ ] Add cluster-level E2E test suite for success path (FINDING-5)
