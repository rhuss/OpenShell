# Tasks: Warm Pool gRPC PoC (Milestone 1)

**Input**: Design documents from `specs/002-warm-pool-grpc-poc/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Proto Definition)

**Purpose**: Define the ActivateSandbox proto contract and wire up codegen.

- [x] T001 Create proto/supervisor.proto with Supervisor service, ActivateSandbox RPC, request/response messages, and ErrorCode enum per contracts/supervisor.proto
- [x] T002 Add proto/supervisor.proto to crates/openshell-sandbox/build.rs tonic server codegen (compile supervisor.proto with tonic::configure().build_server(true))
- [x] T003 Add proto/supervisor.proto to crates/openshell-driver-kubernetes/build.rs tonic client codegen (compile supervisor.proto with tonic::configure().build_client(true))

---

## Phase 2: Foundational (Supervisor gRPC Server + Health Endpoint)

**Purpose**: Core infrastructure that MUST be complete before user stories: the supervisor can start in unidentified mode, serve gRPC, and report readiness.

- [x] T004 Add --unidentified / OPENSHELL_UNIDENTIFIED CLI flag to Args struct in crates/openshell-sandbox/src/main.rs (clap derive, bool flag)
- [x] T005 Create crates/openshell-sandbox/src/health.rs implementing an HTTP /readyz endpoint using hyper on the existing --health-port (default 8080). Returns 200 when ready, 503 before ready. Use a shared AtomicBool for readiness state.
- [x] T006 Create crates/openshell-sandbox/src/activation.rs with a stub ActivateSandbox gRPC handler. Implement the generated tonic Supervisor trait. Accept the request, log receipt, return success=false with error_code INTERNAL (placeholder). Include a tokio::sync::oneshot::Sender to signal activation completion.
- [x] T007 Add run_unidentified() function to crates/openshell-sandbox/src/lib.rs that: (1) initializes minimal OCSF context, (2) starts health HTTP server from T005, (3) starts tonic gRPC server with the activation service from T006 on port 9090, (4) marks /readyz as ready, (5) awaits the activation oneshot signal
- [x] T008 Wire --unidentified flag in crates/openshell-sandbox/src/main.rs: when set, call run_unidentified() instead of run_sandbox(). Pass health-check and health-port args through.

**Checkpoint**: Supervisor binary can start with `--unidentified --health-check`, listen on gRPC port 9090, serve /readyz returning 200, and accept (but not yet handle) ActivateSandbox calls.

---

## Phase 3: User Story 1 - Claim a Warm Pool Sandbox with Sub-2s Latency (Priority: P1)

**Goal**: End-to-end warm pool claim flow: gateway detects warm pool, claims a pod, calls ActivateSandbox, supervisor bootstraps fully.

**Independent Test**: Create a SandboxWarmPool with ready replicas, run `openshell sandbox create`, verify sandbox is ready in under 2 seconds.

### Implementation for User Story 1

- [x] T009 [US1] Implement full ActivateSandbox handler in crates/openshell-sandbox/src/activation.rs: validate request fields (empty sandbox_id/sandbox_token/gateway_endpoint → ERROR_CODE_INVALID_REQUEST, missing policy → ERROR_CODE_INVALID_REQUEST), store identity, update OCSF context with sandbox_id
- [x] T010 [US1] Extract post-identity bootstrap logic from run_sandbox() in crates/openshell-sandbox/src/lib.rs into a reusable async fn bootstrap_sandbox(sandbox_id: &str, sandbox_name: &str, token: String, gateway_endpoint: &str, policy: SandboxPolicy) -> Result<(), BootstrapError>. This function performs: OPA compilation, gRPC client channel setup (using token directly, skipping IssueSandboxToken per Global Constraints), GetSandboxConfig call, provider env fetch, networking start, ConnectSupervisor session spawn, entrypoint process start. BootstrapError is a new enum wrapping the individual failure modes (PolicyCompilation, GatewayUnreachable, TokenInvalid, Internal).
- [x] T011 [US1] Modify crates/openshell-sandbox/src/grpc_client.rs to support a direct-JWT path: add a constructor or method that accepts a pre-minted JWT string and skips the K8s SA token exchange (IssueSandboxToken). The existing SA token path remains unchanged for cold start.
- [x] T012 [US1] Wire bootstrap_sandbox() into the ActivateSandbox handler in crates/openshell-sandbox/src/activation.rs: after validation, call bootstrap_sandbox(), map errors to ErrorCode variants (POLICY_COMPILATION_FAILED, GATEWAY_UNREACHABLE, TOKEN_INVALID, INTERNAL), fire the oneshot signal on success, return ActivateSandboxResponse
- [x] T013 [US1] Add OCSF logging for activation events in crates/openshell-sandbox/src/activation.rs: AppLifecycleBuilder for activation start/success, DetectionFindingBuilder for activation failures. Follow severity guidelines from AGENTS.md.
- [x] T014 [P] [US1] Create crates/openshell-driver-kubernetes/src/warm_pool.rs with functions: (1) async fn list_warm_pools(client: &Client, namespace: &str) -> Result<Vec<DynamicObject>, kube::Error> - lists SandboxWarmPool CRDs via kube-rs dynamic API (group: agents.x-k8s.io), (2) fn find_matching_pool(pools: &[DynamicObject], image: &str) -> Option<&DynamicObject> - filters by exact image match on spec.template.containers[0].image and status.readyReplicas > 0, (3) async fn create_claim(client: &Client, namespace: &str, pool_name: &str, sandbox_id: &str) -> Result<String, kube::Error> - creates a SandboxClaim CRD, returns claim name, (4) async fn wait_for_claim_ready(client: &Client, namespace: &str, claim_name: &str, timeout: Duration) -> Result<String, WarmPoolError> - watches claim status until phase=Ready, returns pod IP as String. WarmPoolError covers Timeout, ClaimFailed, MissingPodIp variants.
- [x] T015 [P] [US1] Create crates/openshell-driver-kubernetes/src/activation_client.rs with async fn activate_sandbox(endpoint: &str, tls_config: &ClientTlsConfig, request: ActivateSandboxRequest) -> Result<ActivateSandboxResponse, ActivationError>. Builds a tonic channel with mTLS to the supervisor at {endpoint}:9090, wraps the ActivateSandbox call in tokio::time::timeout(Duration::from_secs(5)). ActivationError covers Timeout, ConnectionFailed, and RpcError variants.
- [x] T016 [US1] Modify create_sandbox() in crates/openshell-driver-kubernetes/src/driver.rs to add warm pool detection before the cold-start path: call list_warm_pools() and find_matching_pool(). If a match is found, call create_claim(), wait_for_claim_ready(), then activate_sandbox(). If activation succeeds, build a DriverSandbox from the claim status and return it. If any step fails, log the failure and fall through to the existing cold-start path.

**Checkpoint**: Full warm pool claim flow works end-to-end. Gateway detects warm pool, claims pod, activates supervisor, supervisor bootstraps and connects back to gateway.

---

## Phase 4: User Story 2 - Cold-Start Fallback (Priority: P1)

**Goal**: Cold-start path continues to work identically when no warm pool is available or when activation fails.

**Independent Test**: Request a sandbox for an image with no warm pool configured, verify cold start works unchanged.

### Implementation for User Story 2

- [x] T017 [US2] Add fallback logic in crates/openshell-driver-kubernetes/src/driver.rs create_sandbox(): when warm pool detection finds no match (no SandboxWarmPool for image, or readyReplicas=0), proceed to existing cold-start code path without modification. Ensure the warm pool check is a no-op when no CRDs exist.
- [x] T018 [US2] Add fallback on activation failure in crates/openshell-driver-kubernetes/src/driver.rs: when create_claim() fails, wait_for_claim_ready() times out, or activate_sandbox() returns success=false, log a warning with the error details and fall through to cold-start. The user must not see any warm-pool-related error.
- [x] T019 [US2] Add OCSF logging for warm pool events in crates/openshell-driver-kubernetes/src/driver.rs: log warm pool detection (found/not found), claim creation, activation attempt, activation result, and fallback-to-cold-start events. Use ConfigStateChangeBuilder for pool detection, AppLifecycleBuilder for activation outcomes.

**Checkpoint**: Cold-start path is unaffected by warm pool code. Activation failures transparently fall back.

---

## Phase 5: User Story 3 - Unidentified Supervisor Mode (Priority: P2)

**Goal**: Supervisor starts cleanly in unidentified mode with no gateway connection, no identity, no OPA.

**Independent Test**: Deploy a SandboxTemplate with --unidentified, verify pod reaches Ready, gRPC port listening, no gateway connection.

### Implementation for User Story 3

- [x] T020 [US3] Add OCSF AppLifecycleBuilder event in crates/openshell-sandbox/src/lib.rs run_unidentified(): emit "supervisor started in unidentified mode" at startup with severity Informational
- [x] T021 [US3] Add idempotency guard in crates/openshell-sandbox/src/activation.rs: track activation state with an AtomicBool. If ActivateSandbox is called when already activated, return success=false with error_code ALREADY_ACTIVATED.
- [x] T022 [US3] Add unit tests for the activation handler in crates/openshell-sandbox/src/activation.rs: test validation (missing sandbox_id returns INVALID_REQUEST), test idempotency (second call returns ALREADY_ACTIVATED), test successful activation signal fires the oneshot

**Checkpoint**: Supervisor unidentified mode is robust with proper logging, idempotency, and test coverage.

---

## Phase 6: User Story 4 - Activation Failure Handling (Priority: P2)

**Goal**: Activation failures are handled gracefully with proper error propagation and fallback.

**Independent Test**: Cause an activation failure (invalid policy), verify gateway falls back to cold start.

### Implementation for User Story 4

- [x] T023 [US4] Add timeout handling in crates/openshell-driver-kubernetes/src/activation_client.rs: wrap the ActivateSandbox call in tokio::time::timeout(Duration::from_secs(5)). On timeout, return a synthetic error response.
- [x] T024 [US4] Add retry-or-fallback decision in crates/openshell-driver-kubernetes/src/driver.rs: on activation failure, do NOT retry (PoC simplicity), immediately fall back to cold start. Log the error details at warn level.
- [x] T025 [US4] Add unit tests for warm pool functions in crates/openshell-driver-kubernetes/src/warm_pool.rs: test find_matching_pool() with exact image match, no match, readyReplicas=0. Test create_claim() builds correct CRD JSON.
- [x] T026 [US4] Add unit tests for activation client in crates/openshell-driver-kubernetes/src/activation_client.rs: test timeout handling, test error response mapping.

**Checkpoint**: All failure paths are covered with tests. Activation failures never surface to users.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, cleanup, and pre-commit validation.

- [x] T027 [P] Update architecture/sandbox.md with unidentified supervisor mode and activation flow documentation
- [x] T028 [P] Update architecture/kubernetes-driver.md with warm pool claim path documentation
- [x] T029 Run mise run pre-commit to validate formatting, linting, and license headers across all changed files
- [x] T030 Run mise run test to verify all unit tests pass (existing + new)
- [x] T031 Add integration test in crates/openshell-sandbox/tests/activation_timing.rs: start supervisor in unidentified mode, assert /readyz returns 200 within 1 second of process start (validates SC-002). Record elapsed time for the ActivateSandbox call in test output for SC-001 validation during manual e2e testing.
- [x] T032 Add smoke test verification that an activated supervisor produces identical behavior to a cold-started one (SC-005): in the ActivateSandbox integration test, verify that after activation the supervisor has a live ConnectSupervisor stream, compiled OPA policies, and a running entrypoint (same observable state as cold start)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies, start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 (proto codegen must exist)
- **US1 (Phase 3)**: Depends on Phase 2 (gRPC server + health endpoint must work)
- **US2 (Phase 4)**: Depends on Phase 3 T016 (warm pool path must exist to test fallback)
- **US3 (Phase 5)**: Depends on Phase 2 only (unidentified mode is foundational)
- **US4 (Phase 6)**: Depends on Phase 3 T015 (activation client must exist)
- **Polish (Phase 7)**: Depends on all prior phases

### User Story Dependencies

- **US1 (P1)**: Depends on Foundational. Core delivery.
- **US2 (P1)**: Depends on US1 T016 (warm pool path in driver.rs). Validates fallback paths.
- **US3 (P2)**: Depends on Foundational only. Can run in parallel with US1.
- **US4 (P2)**: Depends on US1 T015 (activation client). Can run after US1 activation client.

### Within Each User Story

- Proto codegen before handler implementation
- Handler stub before full handler
- gRPC server before client
- Supervisor changes before driver changes (US1)
- Core implementation before logging/observability

### Parallel Opportunities

- T002 and T003 can run in parallel (different build.rs files)
- T005 and T006 can run in parallel (different new files)
- T014 and T015 can run in parallel (different new files in driver crate)
- T027 and T028 can run in parallel (different doc files)
- US3 can run in parallel with US1 (after Phase 2)

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Proto definition (T001-T003)
2. Complete Phase 2: Supervisor foundation (T004-T008)
3. Complete Phase 3: End-to-end warm pool claim (T009-T016)
4. **STOP and VALIDATE**: Test warm pool claim flow
5. Deploy/demo if ready

### Incremental Delivery

1. Setup + Foundational -> Supervisor starts in unidentified mode
2. Add US1 -> Full warm pool claim works -> Demo
3. Add US2 -> Cold-start fallback validated -> Confidence
4. Add US3 -> Unidentified mode hardened -> Robustness
5. Add US4 -> Failure handling complete -> Production-ready PoC
6. Polish -> Docs and cleanup -> Ready for review

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- All warm pool CRD interaction uses kube-rs dynamic API (no generated CRD types)
- The supervisor gRPC server port (9090) is separate from the health port (8080)
- mTLS for activation channel reuses existing client_tls_secret_name infrastructure
- Commit after each task or logical group
