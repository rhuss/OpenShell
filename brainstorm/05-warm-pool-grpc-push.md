# Brainstorm: Warm Pool with Always-On Supervisor (gRPC-Push Identity)

**Date:** 2026-07-10
**Status:** active

## Problem Framing

The existing warm pool RFC (`rfc/NNNN-warm-pool-feasibility/`) proposes annotation-based identity binding: the supervisor starts at claim time, detects its identity via the Kubernetes downward API, and bootstraps normally. This approach delivers ~3-4s claim latency (2.3s claim + 1.5s supervisor startup).

For workloads that need sub-1s sandbox provisioning, the supervisor startup at claim time is the bottleneck. The 1.5s breaks down as ~80ms for 8 gRPC calls and ~1.4s for process startup plus OPA policy compilation.

An alternative architecture keeps the supervisor always running in the warm pod. Global OPA policies (applicable to every sandbox) are pre-compiled at pool time. At claim time, the gateway pushes the sandbox identity directly to the supervisor via gRPC, the supervisor compiles only sandbox-specific policies, and the session activates. This eliminates process startup entirely and reduces OPA compilation to the per-sandbox delta.

This brainstorm scopes the alternative RFC, its relationship to the annotation RFC, and how gateway-managed pool lifecycle works across multiple sandbox images.

## Approaches Considered

### A: Two standalone RFCs with cross-references

Create a new RFC (`NNNN-warm-pool-grpc-push`) that stands independently alongside the existing annotation RFC. Both get an "Alternative Approaches" section with a comparison table. No ordering or primary/secondary framing.

- Pros: Each RFC is self-contained, reviewable independently. Maintainers can accept one, both, or neither.
- Cons: Some duplication in shared context (motivation, measurements).

### B: Single RFC with two design variants

Merge both approaches into one RFC with variant sections.

- Pros: No duplication, single document.
- Cons: Very long. Harder to discuss one variant without the other.

### C: Split RFC by concern (3 documents)

Shared pool infrastructure RFC plus two identity binding RFCs.

- Pros: Cleanest separation.
- Cons: Three documents is heavy to manage.

## Decision

**Approach A: Two standalone RFCs with cross-references.** The shared context is short enough to duplicate. The pool management section (gateway-managed pools, multi-image config) lives in the new gRPC-push RFC since it's the one proposing gateway-managed pools. The annotation RFC can reference it.

## Key Requirements

### gRPC-Push RFC content

1. **Always-on supervisor in warm pods.** Supervisor starts at pool provisioning time. Pre-compiles global OPA policies. Exposes a new gRPC endpoint (`ActivateSandbox` or similar) for identity push.

2. **Two-tier OPA compilation.** Global policies (applicable to all sandboxes) compile at pool time. Sandbox-specific policies (per-sandbox rules, provider-specific constraints) compile at claim time after identity push. The split must be explicit in the RFC.

3. **Identity binding via gRPC push.** After the operator binds the claim, the gateway reads the pod IP from `.status.sandbox.podIPs`, connects directly to the supervisor's gRPC endpoint, and pushes sandbox identity (ID, name, policy config). No downward API, no annotation propagation delay.

4. **Gateway-managed pool lifecycle.** The gateway owns SandboxTemplate + SandboxWarmPool resource creation, scaling, and cleanup. Gateway config defines which images get pools and their sizes. Gateway reconciles pool state against desired config.

5. **Pool-per-image for known images.** Gateway config lists images with pool sizes. Example:
   ```yaml
   warmPools:
     - image: ghcr.io/nvidia/openshell-community/sandboxes/base:latest
       replicas: 5
     - image: ghcr.io/nvidia/openshell-community/sandboxes/ollama:latest
       replicas: 2
   ```

6. **Unknown images always cold-start.** Custom Dockerfiles, arbitrary image refs, first-time images go through the existing cold-start path. No auto-promotion in the initial design.

7. **Dynamic pool promotion as documented extension.** The RFC includes a "Future Extensions" section describing auto-promotion: gateway creates a pool for an image after seeing it N times. Not part of the initial design.

### Cross-referencing and comparison

8. **Both RFCs get an "Alternative Approaches" section.** Each links to the other with a comparison table covering:
   - Claim-to-ready latency
   - Supervisor complexity (idle-start vs always-on)
   - Gateway complexity (claim-only vs pool reconciler + gRPC client)
   - OPA compilation (full at claim vs pre-compiled global + delta)
   - Security surface (downward API vs new gRPC endpoint on supervisor)
   - Alignment with upstream (craig-kindo's validated approach vs novel)
   - Resource cost of idle pools (sleeping process vs running supervisor)

9. **Equal framing.** Neither RFC is "primary." Both are valid alternatives for different latency targets and complexity appetites.

### Upstream references (shared)

- [OpenShell#2157](https://github.com/NVIDIA/OpenShell/issues/2157): Feature issue for warm-pool provisioning
- [OpenShell#1447](https://github.com/NVIDIA/OpenShell/issues/1447): craig-kindo's annotation-based implementation (validates the other RFC)
- [agent-sandbox#1118](https://github.com/kubernetes-sigs/agent-sandbox/pull/1118): Operator adoption finalization improvement
- [agent-sandbox#384](https://github.com/kubernetes-sigs/agent-sandbox/issues/384): Upstream file-based env injection tracking

## Open Questions

- What is the new gRPC endpoint name and proto definition for the supervisor identity push?
- How does the gateway discover pool pod IPs before the supervisor has registered? (likely from `.status.sandbox.podIPs` on the adopted Sandbox resource)
- What happens if the gRPC push fails (supervisor crashed, network partition)? Fall back to cold start or retry?
- How does gateway config validation prevent conflicting pool definitions (same image, different pool sizes)?
- Should the gateway pool reconciler run as a background loop or be event-driven (watch SandboxWarmPool status)?
- How does the always-on supervisor handle image updates? The supervisor process is running an older binary while the pool template references a newer image.
