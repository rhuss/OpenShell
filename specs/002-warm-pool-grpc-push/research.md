# Research: Warm Pool gRPC-Push RFC

## Existing Annotation-Based RFC Structure

**Decision**: Follow the same RFC template structure as `rfc/NNNN-warm-pool-feasibility/README.md` (Summary, Motivation, Design, Implementation Plan, Risks, Alternatives, Prior Art, Open Questions).

**Rationale**: Consistency with the existing RFC makes cross-referencing easier and reduces reviewer cognitive load.

**Alternatives considered**: Combined RFC with variant sections (rejected per brainstorm decision for independent reviewability).

## Identity Binding Mechanism

**Decision**: gRPC push from gateway to supervisor pod IP. The gateway reads `.status.sandbox.podIPs` from the adopted Sandbox resource after claim binding.

**Rationale**: Eliminates the 1-2s downward API propagation delay of the annotation approach. Direct network call is the lowest-latency option. The existing mTLS certificates already mounted in warm pods secure the channel.

**Alternatives considered**: Annotation-based (covered by the other RFC), ConfigMap-based (60-90s propagation default), Kubernetes watch from supervisor (requires RBAC in the sandbox pod).

## Two-Tier OPA Compilation

**Decision**: Global policies compile at pool provisioning time. Sandbox-specific policies compile at claim time after identity push.

**Rationale**: The 1.4s OPA compilation cost dominates supervisor startup. Global policies (network rules, filesystem constraints) are identical across sandboxes. Only per-sandbox rules (provider-specific constraints, custom policies) need claim-time compilation.

**Alternatives considered**: Full compilation at pool time (impossible, sandbox-specific policies unknown until claim), full compilation at claim time (same as annotation RFC, no improvement).

## Gateway Pool Management

**Decision**: Gateway owns SandboxTemplate and SandboxWarmPool lifecycle. Config-driven, reconciled at startup.

**Rationale**: Centralizes pool management in the gateway where image routing decisions are already made. Avoids requiring operators to manually manage Kubernetes extension resources.

**Alternatives considered**: Operator-managed pools (simpler gateway, but pool management becomes a separate ops concern), Helm-managed pools (middle ground, but no runtime scaling).

## Supervisor gRPC Endpoint

**Decision**: New `ActivateSandbox` RPC on the supervisor's existing gRPC server. Accepts sandbox ID, name, and sandbox-specific policy configuration.

**Rationale**: The supervisor already runs a gRPC server for health checks and internal communication. Adding an endpoint is simpler than introducing a new protocol.

**Alternatives considered**: HTTP endpoint (different protocol from existing supervisor communication), Unix socket (not routable from gateway pod).

## Upstream References

**Decision**: Cross-reference OpenShell#2157, #1447, agent-sandbox#1118, #384 in both RFCs.

**Rationale**: These issues track the upstream feature work and implementation proposals. Craig-kindo's GKE validation (#1447) confirms the annotation approach independently. PR #1118 improves operator adoption speed, benefiting both approaches.
