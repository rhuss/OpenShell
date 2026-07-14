# Feature Specification: Warm Pool E2E with SSH (Milestone 2)

**Feature Branch**: `6113-warm-pool-grpc-poc`
**Created**: 2026-07-13
**Status**: Draft
**Input**: Brainstorm 08 - Warm Pool E2E with SSH

## User Scenarios & Testing *(mandatory)*

### User Story 1 - SSH into a Warm Pool Sandbox in Under 3 Seconds (Priority: P1)

A developer runs `openshell sandbox create --name demo --from base` on a cluster where a warm pool exists for the base sandbox image. The CLI drops into an interactive SSH shell within 3 seconds, compared to the ~17 seconds of a cold start. The developer experiences the sandbox identically to a cold-started one: shell access, file system, networking all work.

**Why this priority**: This is the acceptance criterion for the PoC. Without a working SSH session, we cannot demonstrate the warm pool value to external customers or validate real-world latency improvements.

**Independent Test**: Run `openshell sandbox create --name warm-test --from base` on a cluster with a deployed warm pool. Verify the CLI drops into a shell and basic commands (`ls`, `whoami`, `cat /etc/hostname`) work. Measure wall-clock time from command invocation to shell prompt.

**Acceptance Scenarios**:

1. **Given** a SandboxWarmPool with readyReplicas > 0 for the base sandbox image, **When** a user runs `openshell sandbox create --name X --from base`, **Then** the CLI drops into an interactive SSH shell within 3 seconds of command invocation.
2. **Given** a warm pool sandbox with an active SSH session, **When** the user runs commands in the shell, **Then** the commands execute identically to a cold-started sandbox (file system, environment variables, user identity all match).
3. **Given** a warm pool sandbox with an active SSH session, **When** the user runs `openshell sandbox create --name X --from base -- echo hello`, **Then** the command output is "hello" and the sandbox exits, within 3 seconds.

---

### User Story 2 - Cold-Start Regression Prevention (Priority: P1)

When no warm pool exists for the requested image, the sandbox creation follows the existing cold-start path with no behavioral changes, no latency regression, and no errors from the warm pool code.

**Why this priority**: Co-equal with Story 1. The warm pool changes must not break the existing experience for users without warm pools configured.

**Independent Test**: Delete all SandboxWarmPools, then run `openshell sandbox create --name cold-test --from base`. Verify the sandbox starts via cold start with identical behavior to the current release.

**Acceptance Scenarios**:

1. **Given** no SandboxWarmPool exists for the requested image, **When** a user creates a sandbox, **Then** the gateway uses the existing cold-start path and the sandbox starts normally with SSH access.
2. **Given** existing unit and integration tests for cold-start sandbox creation, **When** the test suite runs after the warm pool changes, **Then** all existing tests pass without modification.

---

### User Story 3 - Side-by-Side Timing Demonstration (Priority: P2)

An operator can run two sandbox creations back-to-back (one cold, one warm) and observe the latency difference. This is the demo scenario for convincing stakeholders.

**Why this priority**: The demo is the primary deliverable for external communication. It requires both Story 1 (warm path works) and Story 2 (cold path works) to be complete.

**Independent Test**: Run two timed commands in sequence and compare wall-clock durations.

**Acceptance Scenarios**:

1. **Given** a cluster with both cold-start and warm pool capabilities, **When** an operator runs `time openshell sandbox create --name cold --from base -- true` followed by `time openshell sandbox create --name warm --from base -- true`, **Then** the warm creation completes in under 3 seconds while the cold creation takes the usual ~17 seconds.

---

### Edge Cases

- What happens when the activation succeeds but the in-sandbox networking setup fails (proxy port conflict, nftables error)? The supervisor should log the error and the sandbox should be reported as failed, not left in a broken state.
- What happens when the SSH relay cannot reach the activated sandbox? The CLI should timeout with a clear error, not hang indefinitely.
- What happens when `bootstrap_sandbox()` takes longer than expected (slow OPA compilation, slow gateway RPCs)? The 5-second activation timeout in the gateway should trigger cold-start fallback.
- What happens when the warm pool pod's entrypoint process fails to start? The sandbox should report as failed with diagnostics.

## Clarifications

### Session 2026-07-13

- Q: What is the target latency for the SSH demo? -> A: Under 3 seconds from CLI invocation to shell prompt (includes claim ~1.4s, activation ~0.5s, SSH setup ~1s).
- Q: Should the warm pool sandbox support the full sandbox command syntax (-- cmd args)? -> A: Yes, both interactive SSH (no command) and command execution (-- echo hello) must work.
- Q: Is mTLS required for the ActivateSandbox channel in this milestone? -> A: No, plaintext gRPC is acceptable for the PoC. mTLS is deferred to a future milestone.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `bootstrap_sandbox()` MUST start the in-sandbox networking stack (HTTP proxy, nftables rules, DNS configuration) after activation, matching the cold-start path's networking setup.
- **FR-002**: `bootstrap_sandbox()` MUST start the SSH listener after networking is configured, so the CLI can connect via the SSH relay.
- **FR-003**: `bootstrap_sandbox()` MUST start the entrypoint process (user shell or command) after SSH is ready.
- **FR-004**: `bootstrap_sandbox()` MUST wire OPA policy enforcement into the HTTP proxy, so network policies are enforced identically to cold-started sandboxes.
- **FR-005**: The supervisor MUST remain running after activation (block on signals), keeping all spawned tasks (ConnectSupervisor, SSH listener, proxy, entrypoint) alive until the pod is terminated.
- **FR-006**: The cold-start path (`run_sandbox()`) MUST continue to work identically after the refactoring. All existing tests MUST pass.
- **FR-007**: The networking/SSH/proxy/entrypoint startup code MUST be shared between `run_sandbox()` (cold start) and `bootstrap_sandbox()` (warm pool), avoiding code duplication.
- **FR-008**: The gateway MUST pass a real gateway-minted JWT in the ActivateSandbox request (already working from Milestone 1, validated on the cluster).
- **FR-009**: After activation and bootstrap, the supervisor MUST call `ConnectSupervisor` with the gateway-minted JWT (already working from Milestone 1).
- **FR-010**: The warm pool sandbox MUST support both interactive SSH sessions (no command argument) and single-command execution (-- command args).

### Key Entities

- **SandboxNetworkingConfig**: The set of parameters needed to start the in-sandbox networking stack (proxy port, OPA engine, nftables rules, DNS policy). Currently embedded in `run_sandbox()`'s local variables, needs to be extractable.
- **SandboxBootstrapContext**: The identity and configuration needed to bootstrap a sandbox after activation (sandbox_id, JWT, gateway endpoint, OPA policies, provider environment, SSH socket path).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A warm pool sandbox creation drops into an interactive SSH shell within 3 seconds of CLI invocation, compared to ~17 seconds for cold start.
- **SC-002**: A warm pool sandbox supports command execution (`-- echo hello`) that completes within 3 seconds.
- **SC-003**: All existing cold-start tests pass without modification after the refactoring.
- **SC-004**: The side-by-side demo shows at least a 5x latency improvement (warm vs cold).
- **SC-005**: An activated warm pool sandbox is functionally identical to a cold-started sandbox for SSH access, command execution, and network policy enforcement.

## Assumptions

- Milestone 1 implementation (branch `6113-warm-pool-grpc-poc`, spec 002) is complete and working. The gateway detects warm pools, claims pods, and calls ActivateSandbox successfully.
- The gateway already mints a real JWT for the sandbox and passes it in the ActivateSandbox request (validated on the cluster).
- The supervisor's `run_sandbox()` function (~650 lines) can be decomposed into phases: pre-identity setup, identity acquisition, post-identity bootstrap (networking, SSH, proxy, entrypoint). The post-identity phase is what needs to be shared.
- The existing mTLS certificates in the namespace are available to the warm pool pods for supervisor-to-gateway communication (validated in Milestone 1).
- Plaintext gRPC for the ActivateSandbox channel is acceptable for this PoC milestone.
- The ROSA HCP 4.22.3 test cluster (`warm-pool-rerun`) with the Agent Sandbox operator v0.9.0 remains available for testing.
- Custom supervisor and gateway images can be pushed to `quay.io/rhuss/` for testing.

## Out of Scope

- mTLS on the ActivateSandbox gRPC endpoint (tracked in idea inbox, deferred)
- Automated warm pool lifecycle management (auto-scaling, pod replacement)
- Workspace-scoped pool configuration (waiting on Derek's workspace model PR)
- Label/selector-based pool matching (exact image match only)
- Inference routing validation in the warm pool sandbox (proxy starts but inference testing is deferred)
- Updates to the smoke test script or SMOKE-TEST.md (will be updated after implementation)
