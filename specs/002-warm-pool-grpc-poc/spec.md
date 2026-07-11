# Feature Specification: Warm Pool gRPC PoC (Milestone 1)

**Feature Branch**: `6113-warm-pool-grpc-poc`
**Created**: 2026-07-11
**Status**: Draft
**Input**: Brainstorm 06 - Warm Pool gRPC PoC

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Claim a Warm Pool Sandbox with Sub-2s Latency (Priority: P1)

An agent operator creates a sandbox via the CLI. When a warm pool exists with ready replicas matching the requested image, the gateway claims a pre-provisioned pod and activates it by pushing identity and policy configuration over gRPC. The sandbox becomes usable without a full cold-start cycle.

**Why this priority**: This is the core value proposition. Reducing sandbox startup from ~16.7s (cold start) to sub-2s (warm pool claim + activation) directly impacts agent developer productivity and platform responsiveness.

**Independent Test**: Can be fully tested by creating a SandboxWarmPool with ready replicas, then running `openshell sandbox create` and measuring time-to-ready. Delivers value by proving the end-to-end claim-time activation flow.

**Acceptance Scenarios**:

1. **Given** a SandboxWarmPool exists with readyReplicas > 0 for the requested image, **When** a user creates a sandbox via the CLI, **Then** the gateway claims a warm pod, calls ActivateSandbox with identity and policy, and returns the sandbox as ready in under 2 seconds (excluding network latency to the gateway).
2. **Given** a warm pool pod is in the unidentified state and listening on gRPC, **When** the gateway sends an ActivateSandbox request with sandbox ID, JWT, name, policy config, and gateway endpoint, **Then** the supervisor stores identity, compiles OPA policies, connects to the gateway, and returns a success response.
3. **Given** a warm pool pod has been activated, **When** the user connects to the sandbox, **Then** the sandbox behaves identically to a cold-started sandbox (SSH access, policy enforcement, inference routing all work).

---

### User Story 2 - Cold-Start Fallback When No Warm Pool Available (Priority: P1)

When no warm pool exists for the requested image, or all warm pool replicas are already claimed, the system falls back to the existing cold-start path transparently. The user experience is unchanged from today's behavior.

**Why this priority**: Co-equal with Story 1 because the warm pool path must not break the existing cold-start path. Both paths must coexist safely.

**Independent Test**: Can be tested by requesting a sandbox for an image with no warm pool configured, and verifying the sandbox starts via the existing cold-start flow with no errors or behavioral changes.

**Acceptance Scenarios**:

1. **Given** no SandboxWarmPool exists for the requested image, **When** a user creates a sandbox, **Then** the gateway uses the existing cold-start path and the sandbox starts normally.
2. **Given** a SandboxWarmPool exists but readyReplicas is 0, **When** a user creates a sandbox, **Then** the gateway falls back to cold start rather than waiting for a warm pod.

---

### User Story 3 - Supervisor Starts in Unidentified Mode in Warm Pods (Priority: P2)

The supervisor process in a warm pool pod starts without any gateway connection, identity, or OPA policies. It listens on a gRPC port, exposes a readiness endpoint, and waits for an ActivateSandbox call to receive its identity and configuration.

**Why this priority**: This is the architectural foundation that enables Story 1, but is not user-facing. It provides the unidentified supervisor state that NVIDIA's feedback explicitly endorsed.

**Independent Test**: Can be tested by deploying a SandboxTemplate with the supervisor in unidentified mode and verifying the pod reaches Ready state, the gRPC port is listening, and no gateway connection or OPA compilation occurs until activation.

**Acceptance Scenarios**:

1. **Given** a SandboxTemplate configured with unidentified supervisor mode, **When** a warm pool pod starts, **Then** the supervisor process starts, listens on the gRPC port, and the pod's readinessProbe returns 200.
2. **Given** a supervisor running in unidentified mode, **When** no ActivateSandbox call has been made, **Then** the supervisor has no gateway connection, no compiled OPA policies, and no sandbox identity.

---

### User Story 4 - Activation Failure Handling (Priority: P2)

When the ActivateSandbox call fails (supervisor crash, OPA compilation error, gateway connectivity issue), the gateway handles the failure gracefully by falling back to cold start rather than returning an error to the user.

**Why this priority**: Important for reliability but secondary to the happy-path flow. Users should never see a warm pool internal failure.

**Independent Test**: Can be tested by intentionally causing an activation failure (e.g., invalid policy config) and verifying the gateway falls back to cold start.

**Acceptance Scenarios**:

1. **Given** a warm pool pod is claimed and the gateway calls ActivateSandbox, **When** the activation fails (timeout, OPA error, connection refused), **Then** the gateway falls back to creating a sandbox via cold start.
2. **Given** an activation failure occurred, **When** the fallback cold start completes, **Then** the user receives a working sandbox with no indication that a warm pool attempt was made.

---

### Edge Cases

- What happens when a warm pool pod is claimed but the supervisor crashes between claim and activation? The gateway should detect the failure via the ActivateSandbox error response and fall back to cold start.
- What happens when two gateways attempt to activate the same warm pool pod simultaneously? The SandboxClaim operator ensures exclusive binding, so only one gateway will receive the pod IP. The second claim will either get a different pod or fall back to cold start.
- What happens when the gateway cannot reach the supervisor's gRPC port (network policy, pod not ready)? The ActivateSandbox call will timeout, triggering the cold-start fallback.
- What happens when the supervisor receives an ActivateSandbox call with an invalid or expired JWT? The supervisor should return an error in the ActivateSandbox response, and the gateway should fall back to cold start.

## Clarifications

### Session 2026-07-11

- Q: What timeout should the gateway use for the ActivateSandbox gRPC call before falling back to cold start? → A: 5 seconds (generous for OPA compilation + gateway registration, well under cold-start time of ~16.7s)
- Q: Should warm pool claim/activation events be logged via OCSF structured logging? → A: Yes, activation success and failure are observable sandbox behavior and should use OCSF events (AppLifecycleBuilder for activation, DetectionFindingBuilder for failures)
- Q: How does the gateway match a sandbox create request to a SandboxWarmPool? → A: Exact container image match for this PoC. Label-based or selector-based matching deferred to Milestone 2.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Supervisor MUST support an unidentified startup mode where it starts without gateway connection, identity, or OPA policies.
- **FR-002**: Supervisor in unidentified mode MUST listen on a gRPC port and expose a `/readyz` HTTP endpoint (reusing the existing `--health-check` infrastructure) that returns 200 when the supervisor is ready to receive activation.
- **FR-003**: Supervisor MUST implement an ActivateSandbox gRPC endpoint that accepts sandbox ID, name, JWT, policy configuration, and gateway endpoint.
- **FR-004**: Upon receiving ActivateSandbox, the supervisor MUST store the identity, compile OPA policies from the provided config, use the gateway-minted JWT directly (skipping IssueSandboxToken, since the gateway already minted the token and passes it in the request), call GetSandboxConfig against the gateway, and call ConnectSupervisor to register the session.
- **FR-005**: The ActivateSandbox endpoint MUST return a success or failure response with error details to the caller.
- **FR-006**: The gateway's Kubernetes driver MUST detect when a SandboxWarmPool with ready replicas exists for the requested image and use the warm pool claim path instead of cold start.
- **FR-007**: After a SandboxClaim reports Ready with a pod IP, the gateway MUST read the pod IP from the claim status and call ActivateSandbox on the supervisor.
- **FR-008**: The gateway MUST use existing namespace mTLS certificates for the ActivateSandbox channel.
- **FR-009**: When no warm pool exists for the requested image, or readyReplicas is 0, the gateway MUST fall back to the existing cold-start path.
- **FR-010**: When ActivateSandbox fails or does not respond within 5 seconds, the gateway MUST fall back to cold start rather than returning an error to the user.
- **FR-011**: A new `Supervisor` gRPC service MUST be defined (in a new `supervisor.proto` or in `sandbox.proto`) with an `ActivateSandbox` RPC. The request MUST carry sandbox ID, name, JWT, policy config, and gateway endpoint. The response MUST carry success/failure and error details. This is a new service because the supervisor acts as gRPC server here (the reverse of the existing `OpenShell` service where the supervisor is a client).
- **FR-012**: The unidentified supervisor mode MUST be selectable via both a CLI flag (`--unidentified`) and an environment variable (`OPENSHELL_UNIDENTIFIED`) on the supervisor binary.
- **FR-013**: The gateway MUST log warm pool activation events (claim, activation success, activation failure, fallback to cold start) using OCSF structured logging.
- **FR-014**: The gateway MUST match sandbox create requests to SandboxWarmPools by exact container image match.

### Key Entities

- **SandboxWarmPool**: Kubernetes custom resource that defines a pool of pre-provisioned sandbox pods for a specific image. Key attributes: target image, desired replicas, ready replica count.
- **SandboxClaim**: Kubernetes custom resource that represents a request to bind a warm pool pod to a specific sandbox. Reports pod IP when bound.
- **SandboxTemplate**: Kubernetes custom resource defining the pod specification for warm pool pods, including the supervisor container configured for unidentified mode.
- **ActivateSandbox Request**: gRPC message carrying sandbox identity (ID, name, JWT), policy configuration, and gateway endpoint to the supervisor.
- **ActivateSandbox Response**: gRPC message carrying success/failure status and error details back to the gateway.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Sandbox creation using a warm pool completes in under 2 seconds end-to-end (from CLI request to sandbox ready), compared to ~16.7 seconds for cold start.
- **SC-002**: A supervisor in unidentified mode reaches pod readiness (gRPC port listening, `/readyz` returning 200) within 1 second of container start.
- **SC-003**: Cold-start sandbox creation continues to work identically when no warm pool is available, with no latency regression or behavioral changes.
- **SC-004**: Activation failures (OPA errors, timeouts, network issues) result in successful cold-start fallback 100% of the time, with no user-visible errors from the warm pool attempt.
- **SC-005**: An activated warm pool sandbox is functionally identical to a cold-started sandbox (SSH access, policy enforcement, inference routing).

## Assumptions

- The SandboxWarmPool, SandboxClaim, and SandboxTemplate CRDs already exist in the cluster (created by the warm pool operator from the feasibility study).
- Warm pool creation and lifecycle management are manual for this PoC (kubectl-based). Automated pool management is deferred to Milestone 2.
- The existing namespace mTLS certificates are available and sufficient for the gateway-to-supervisor gRPC channel. Warm pool pods run in the same namespace where these certificates are provisioned.
- The supervisor binary already supports gRPC serving infrastructure that can be extended with the new ActivateSandbox endpoint.
- OPA policy compilation at claim time adds approximately 100-200ms, which is acceptable within the sub-2s target.
- The gateway already has access to SandboxClaim status fields including pod IP.
- Issue #1955 (legacy RPC cleanup) will not conflict with adding the new ActivateSandbox RPC, though coordination with that work is recommended.
- The supervisor will not implement an activation timeout for this PoC (resource cleanup of unclaimed pods is deferred to Milestone 2).
- The ActivateSandbox RPC requires a new `Supervisor` gRPC service definition because the supervisor acts as the gRPC server (the reverse of the `OpenShell` service where the supervisor is a client calling `ConnectSupervisor`, `IssueSandboxToken`, etc.).
- The gateway endpoint is passed in the ActivateSandbox request (not via env var at pool provisioning time), since warm pods are unidentified and should not have gateway-specific configuration baked in.

## Out of Scope

- Automated warm pool lifecycle management (auto-scaling, pod replacement after claim). Deferred to Milestone 2.
- Label-based or selector-based image matching for SandboxWarmPool. This PoC uses exact container image match only (FR-014).
- Two-tier OPA pre-compilation (global policies at pool time, sandbox-specific at claim time). Deferred pending latency analysis.
- Activation timeout and resource cleanup of unclaimed pods. Deferred to Milestone 2.
- Multi-gateway warm pool coordination beyond SandboxClaim's exclusive binding mechanism.
- Warm pool observability dashboards or metrics beyond OCSF event logging (FR-013).
