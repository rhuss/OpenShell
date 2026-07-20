# Feature Specification: Warm Pool Integration with Always-On Supervisor (gRPC-Push)

**Feature Branch**: `6111-warm-pool-feasibility`
**Created**: 2026-07-10
**Status**: Draft
**Input**: Alternative RFC for warm pool integration using always-on supervisor with gRPC-push identity binding

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sub-Second Sandbox Provisioning via Warm Pool (Priority: P1)

An agent platform operator configures warm pools for known sandbox images in the gateway configuration. When an agent requests a sandbox, the gateway claims a pre-provisioned pod from the pool. The always-on supervisor in that pod receives the sandbox identity via a direct gRPC push from the gateway. The supervisor compiles only sandbox-specific OPA policies (global policies were pre-compiled at pool time) and activates the session. The agent gets an SSH-ready sandbox in under 1 second from request to ready.

**Why this priority**: This is the core value proposition. The annotation-based alternative delivers 3-4s; the gRPC-push approach targets sub-1s by eliminating supervisor startup and downward API propagation delay.

**Independent Test**: Can be tested by configuring a single warm pool for the base sandbox image, creating a sandbox, and measuring claim-to-ready latency. Expected: < 1s.

**Acceptance Scenarios**:

1. **Given** a warm pool with 5 ready replicas for the base sandbox image, **When** a user runs `openshell sandbox create`, **Then** the sandbox reaches Ready state in under 1 second.
2. **Given** a warm pool with available replicas, **When** a SandboxClaim is created, **Then** the gateway pushes identity to the supervisor via gRPC within 100ms of claim binding.
3. **Given** a supervisor that has pre-compiled global OPA policies at pool time, **When** it receives identity via gRPC push, **Then** it compiles only sandbox-specific policies and reports Ready within 500ms.

---

### User Story 2 - Gateway-Managed Pool Lifecycle (Priority: P1)

A platform operator defines warm pool configuration in the gateway config file, specifying which sandbox images should have pools and how many replicas each pool maintains. The gateway creates, scales, and deletes the corresponding SandboxTemplate and SandboxWarmPool Kubernetes resources. The operator does not need to manage these resources manually.

**Why this priority**: Without gateway-managed pools, operators must manually create and maintain Kubernetes resources for each image. This is error-prone and blocks adoption.

**Independent Test**: Can be tested by adding a warm pool entry to gateway config, restarting the gateway, and verifying that SandboxTemplate and SandboxWarmPool resources appear in the namespace with the correct image and replica count.

**Acceptance Scenarios**:

1. **Given** a gateway config with a warm pool entry for `base:latest` with 5 replicas, **When** the gateway starts, **Then** it creates a SandboxTemplate and SandboxWarmPool with the specified image and replica count.
2. **Given** a running gateway with a warm pool, **When** the operator changes the replica count in the config and restarts the gateway, **Then** the SandboxWarmPool is updated to the new count.
3. **Given** a running gateway with a warm pool, **When** the operator removes the pool entry from config and restarts the gateway, **Then** the SandboxWarmPool and SandboxTemplate are deleted.

---

### User Story 3 - Multi-Image Pool Management (Priority: P2)

A platform operator configures multiple warm pools for different sandbox images (e.g., base, ollama, custom-ml). Each image gets its own pool with independent replica counts. When a user creates a sandbox specifying an image that has a pool, the gateway routes the request to the matching pool. When the image has no pool, the gateway falls back to cold-start provisioning.

**Why this priority**: Real deployments use multiple sandbox images. Without multi-image support, warm pooling only benefits a single image.

**Independent Test**: Can be tested by configuring two pools (base with 5 replicas, ollama with 2 replicas), creating sandboxes with each image, and verifying pool routing. Then creating a sandbox with an unconfigured image and verifying cold-start fallback.

**Acceptance Scenarios**:

1. **Given** pools for `base:latest` (5 replicas) and `ollama:latest` (2 replicas), **When** a user creates a sandbox with `--from base`, **Then** the gateway claims from the base pool.
2. **Given** pools for `base:latest` and `ollama:latest`, **When** a user creates a sandbox with `--from ollama`, **Then** the gateway claims from the ollama pool.
3. **Given** no pool for `custom-ml:latest`, **When** a user creates a sandbox with that image, **Then** the gateway provisions via cold-start.

---

### User Story 4 - Cold-Start Fallback on Pool Exhaustion (Priority: P2)

When all replicas in a warm pool are claimed and the pool is temporarily empty, the gateway falls back to cold-start provisioning. The user still gets a sandbox, just with higher latency. When the pool replenishes, subsequent requests use warm pods again.

**Why this priority**: Pool exhaustion is inevitable under burst load. Graceful fallback prevents failures.

**Independent Test**: Can be tested by claiming all 5 replicas from a pool, then immediately requesting a 6th sandbox. The 6th should succeed via cold-start. After the pool replenishes, the next request should use a warm pod.

**Acceptance Scenarios**:

1. **Given** a warm pool with 0 ready replicas (all claimed), **When** a user creates a sandbox, **Then** the gateway provisions via cold-start and the sandbox reaches Ready.
2. **Given** a pool that was exhausted, **When** the pool controller replenishes replicas, **Then** the next sandbox request claims from the pool.

---

### User Story 5 - Two-Tier OPA Policy Compilation (Priority: P2)

The supervisor in a warm pool pod pre-compiles global OPA policies (network rules, filesystem constraints applicable to all sandboxes) at pool provisioning time. At claim time, only sandbox-specific policies (per-sandbox rules, provider-specific constraints) are compiled and merged with the pre-compiled global policies. This reduces claim-time policy overhead from ~1.4s to the delta compilation time.

**Why this priority**: OPA compilation is the largest contributor to supervisor startup latency (1.4s of the 1.5s total). Splitting into pre-compiled globals and claim-time specifics is what makes sub-1s possible.

**Independent Test**: Can be tested by measuring policy compilation time at pool provisioning (global) and at claim time (sandbox-specific delta). The claim-time compilation should be significantly faster than full compilation.

**Acceptance Scenarios**:

1. **Given** a warm pool pod with global policies pre-compiled, **When** it receives identity via gRPC push, **Then** it compiles only sandbox-specific policies.
2. **Given** a supervisor with pre-compiled global policies and a sandbox-specific network deny rule, **When** sandbox-specific policy compilation completes, **Then** the combined policy set blocks requests matching the sandbox-specific deny rule AND requests matching global deny rules.

---

### Edge Cases

- What happens when the gRPC push to the supervisor fails (pod crashed between claim and push)? The gateway retries once (2-second timeout per attempt), then falls back to cold-start provisioning.
- What happens when the gateway config defines duplicate pool entries for the same image? The gateway rejects the config at startup with a validation error.
- What happens when a warm pool pod's supervisor binary is outdated (image updated but pod not replaced)? The pool's `OnReplenish` update strategy gradually replaces pods as they are claimed and replenished.
- What happens when the gateway loses connectivity to the Kubernetes API during pool reconciliation? The gateway uses its last-known pool state and retries reconciliation on a backoff interval.
- What happens when mTLS certificates are missing or expired on the gRPC push channel? The gateway treats the push as failed (TLS handshake error), applies the same retry/fallback policy as FR-012 (2-second timeout, 1 retry, then cold-start fallback).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST expose an `ActivateSandbox` gRPC endpoint on the supervisor that accepts sandbox identity (ID, name, sandbox-specific policy configuration).
- **FR-002**: System MUST start the supervisor process at pool provisioning time and pre-compile global OPA policies before the pod is marked as ready in the warm pool.
- **FR-003**: System MUST allow the gateway to push sandbox identity to the supervisor via direct gRPC connection to the pod IP after claim binding.
- **FR-004**: System MUST compile only sandbox-specific OPA policies at claim time, merging them with pre-compiled global policies.
- **FR-005**: System MUST allow the gateway to create, update, and delete SandboxTemplate and SandboxWarmPool Kubernetes resources based on gateway configuration.
- **FR-006**: System MUST support multiple warm pools, one per configured sandbox image, with independent replica counts.
- **FR-007**: System MUST fall back to cold-start provisioning when no warm pool exists for the requested image or when the matching pool has no available replicas.
- **FR-008**: System MUST read the adopted pod's IP from the SandboxClaim's `.status.sandbox.podIPs` field for gRPC push targeting.
- **FR-009**: System MUST NOT include `spec.env` fields on SandboxClaims, as this bypasses warm pool adoption in the Agent Sandbox operator v0.9.0.
- **FR-010**: System MUST validate gateway warm pool configuration at startup, rejecting duplicate image entries and invalid replica counts.
- **FR-011**: System MUST secure the gRPC push channel between gateway and supervisor using the existing namespace mTLS certificates (TLS client secret volume already mounted in the warm pod template).
- **FR-012**: System MUST apply a 2-second timeout with 1 retry for the gRPC identity push. If both attempts fail, the system MUST fall back to cold-start provisioning.
- **FR-013**: Supervisor MUST return 200 on `/readyz` in both idle (waiting for claim) and activated (serving session) states so the pod remains Ready in the warm pool.

### Key Entities

- **WarmPoolConfig**: Gateway configuration entry defining an image and its pool size. Maps to a SandboxTemplate + SandboxWarmPool pair.
- **ActivateSandboxRequest**: gRPC message carrying sandbox identity (ID, name, policy config) from gateway to supervisor.
- **GlobalPolicyBundle**: Pre-compiled OPA policy set applicable to all sandboxes, compiled at pool provisioning time.
- **SandboxPolicyDelta**: Sandbox-specific OPA rules compiled at claim time and merged with the global bundle.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Sandbox provisioning from warm pool reaches Ready state in under 1 second (p50) and under 2 seconds (p99), measured from `CreateSandbox` request to SSH-ready.
- **SC-002**: Gateway creates correct SandboxTemplate and SandboxWarmPool resources within 5 seconds of startup when warm pool config is present.
- **SC-003**: Sandbox-specific OPA policy compilation at claim time completes in under 200ms (compared to ~1,400ms for full compilation).
- **SC-004**: 100% of sandbox requests succeed when a warm pool is exhausted, via cold-start fallback.
- **SC-005**: Gateway correctly routes sandbox requests to the matching pool based on image, with zero misroutes across 100 consecutive requests.

## Clarifications

### Session 2026-07-10

- Q: How is the gRPC push channel between gateway and supervisor secured? → A: Use existing namespace mTLS certificates already mounted in the warm pod template (TLS client secret volume). No new certificate infrastructure needed.
- Q: What is the timeout and retry policy for gRPC push before cold-start fallback? → A: 2-second timeout with 1 retry. If both attempts fail, fall back to cold-start provisioning.
- Q: How does the supervisor's readiness probe distinguish between idle (ready for claim) and activated (serving a session)? → A: `/readyz` returns 200 in both states so the pod stays Ready in the pool. Activation state is tracked internally by the supervisor. The pool controller uses pool membership, not readiness, to determine claimability.

## Assumptions

- The Agent Sandbox operator v0.9.0 (or later) is installed with extension CRDs (SandboxTemplate, SandboxWarmPool, SandboxClaim).
- Env var injection on SandboxClaim bypasses warm pool adoption. This is a known operator constraint documented in the feasibility study.
- The gateway has Kubernetes RBAC permissions to create, update, and delete SandboxTemplate, SandboxWarmPool, and SandboxClaim resources.
- Pod IPs are routable from the gateway pod (same namespace, standard Kubernetes networking).
- Global OPA policies are stable across sandbox sessions. Policy updates require pool replenishment (new pods get new policies).
- This spec is an equal alternative to the annotation-based approach described in the feasibility study. Neither is primary.
- Dynamic pool promotion for unknown images is a future extension, not part of this initial design.
