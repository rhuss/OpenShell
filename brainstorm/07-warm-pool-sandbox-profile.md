# Brainstorm: SandboxProfile + Workspace-Scoped Pool Lifecycle (Milestone 2)

**Date:** 2026-07-11
**Status:** active

## Problem Framing

Milestone 1 (brainstorm 06) proves the ActivateSandbox gRPC claim-time
flow with manually created pools. This milestone addresses the entity
model and lifecycle management that makes warm pools mergeable upstream.

NVIDIA feedback (2026-07-10 Slack thread) established clear direction:

- **Derek Carr**: Pools must be workspace-scoped, not global gateway
  config. Workspaces map to namespaces. Different workspaces need
  different pools (openclaw tool execution vs opencode on-demand).
  "Prototyping with global config is fine for now, I just don't think
  we will want to merge that."

- **Andrew Newberry**: Use `SandboxProfile` as the entity, consistent
  with Provider/ProviderProfile naming. Attach K8s-specific warm pool
  config via the `driver_config` passthrough (RFC 0006, already
  implemented). This avoids leaking K8s details to non-K8s drivers.

- **Derek Carr**: "Maybe we expose certain entity model resources if
  and only if OpenShell is using a particular driver." Also mentioned
  that for non-K8s drivers, pools could signal image pre-pulls.

## Approaches Considered

### A: SandboxProfile with driver_config for pools (chosen)

Introduce a `SandboxProfile` entity. The K8s driver's `driver_config`
block carries warm pool settings (image, replicas, readiness config).
The gateway reads SandboxProfile and reconciles SandboxWarmPool
resources in the workspace's namespace.

```
SandboxProfile
  name: "fast-coding"
  sandbox_template: "base"
  driver_config:
    kubernetes:
      warm_pool:
        replicas: 5
        readiness_timeout: 300s
```

Workspace references one or more SandboxProfiles. When a workspace is
created in a namespace, the gateway creates corresponding
SandboxWarmPool resources in that namespace.

- Pros: Consistent with NVIDIA entity naming (Provider/ProviderProfile).
  Uses existing RFC 0006 driver_config passthrough (no new API pattern).
  Naturally workspace-scoped. Non-K8s drivers can ignore the warm_pool
  block or use it for pre-pulls.
- Cons: Introduces a new entity (SandboxProfile). Requires workspace
  entity to exist first (Derek's PR).

### B: Workspace.spec.pools direct config

Pool config lives directly in the workspace spec, no SandboxProfile
indirection:

```
Workspace
  spec:
    pools:
    - image: "base:latest"
      replicas: 5
```

- Pros: Simpler (no new entity). Pool config is right where you see it.
- Cons: Workspace becomes K8s-aware (pool replicas are a K8s concept).
  Can't reuse pool configs across workspaces. Doesn't follow the
  ProviderProfile pattern. Andrew explicitly recommended SandboxProfile.

### C: Gateway TOML with namespace mapping

Keep pool config in gateway TOML but add namespace targeting:

```toml
[[compute.warm_pools]]
image = "base:latest"
replicas = 5
namespace = "workspace-alpha"
```

- Pros: No new entities. Works without workspace entity.
- Cons: Derek explicitly rejected global config ("we will not want to
  merge that"). Doesn't scale to dynamic workspace creation. Static
  config doesn't know about future namespaces.

## Decision

**Approach A: SandboxProfile with driver_config for pools.**

This is the consensus from both Derek (workspace-scoped) and Andrew
(SandboxProfile entity, driver_config passthrough). The gateway TOML
approach (Approach C) is acceptable for the PoC in milestone 1 but
must not be the merge target.

## Key Requirements

### Entity Model

1. **SandboxProfile proto definition**: New message in the OpenShell
   proto. Contains a name, a reference to a SandboxTemplate (or inline
   template spec), and a `driver_config` JSON block following RFC 0006.

2. **K8s driver_config schema for warm pools**:
   ```json
   {
     "warm_pool": {
       "replicas": 5,
       "readiness_timeout": "300s",
       "max_surge": 1
     }
   }
   ```
   The K8s driver owns validation of this block. Other drivers ignore it
   or interpret `warm_pool` as a pre-pull hint.

3. **Workspace references SandboxProfile**: When a workspace is
   configured, it references one or more SandboxProfiles by name. Each
   profile maps to pool resources in the workspace's namespace.

### Gateway Pool Lifecycle

4. **Pool reconciliation per workspace**: When the gateway learns about
   a workspace (via watch or startup scan), it reads the associated
   SandboxProfiles, extracts the K8s driver_config, and creates/updates
   SandboxTemplate + SandboxWarmPool resources in the workspace's
   namespace.

5. **Pool teardown**: When a workspace is deleted or a SandboxProfile
   is removed from it, the gateway deletes the corresponding pool
   resources.

6. **Pool update**: When a SandboxProfile's driver_config changes
   (replica count, image), the gateway updates the SandboxWarmPool.
   Changes to image require draining the old pool and creating a new
   one.

### Non-K8s Driver Behavior

7. **Docker/Podman**: The `warm_pool` driver_config block signals
   which images to pre-pull. No container pooling (containers are
   cheap to create locally).

8. **VM driver**: Future consideration. Snapshot-based pooling is
   possible but out of scope.

### Integration with Milestone 1

9. **ActivateSandbox flow unchanged**: The claim-time gRPC flow from
   milestone 1 is not affected by how pools are created. SandboxProfile
   controls pool lifecycle; ActivateSandbox controls claim-time
   activation. Clean separation.

10. **Migration path**: PoC can start with gateway TOML config (M1),
    then replace the pool creation source with SandboxProfile reads
    (M2) without changing the claim-time path.

### RBAC and Security

11. **Workspace admin can configure pools**: SandboxProfile creation
    and workspace association should be available to workspace admins,
    not just cluster admins.

12. **Pool resource limits**: Consider per-workspace quotas on total
    warm pool replicas to prevent resource exhaustion.

## Open Questions

- What is the relationship between SandboxProfile and existing
  SandboxTemplate? Is SandboxProfile a wrapper around SandboxTemplate
  plus driver_config, or a separate entity that references a template?
- Does Derek's workspace PR define a workspace proto/CRD? If so, how
  does it reference SandboxProfiles? By name, by label selector, or
  inline?
- Should SandboxProfile be a K8s CRD or a gateway-internal entity
  stored in the gateway's state? CRD is more natural for K8s but adds
  RBAC complexity.
- How to handle pool config for images that don't yet exist in a
  SandboxProfile? Fall back to cold start, or auto-create a profile?
- Pool utilization metrics: Derek raised the question of whether to
  expose utilization. Should this be a SandboxProfile status field,
  a separate metric endpoint, or both?
- What happens when a workspace spans multiple namespaces (Derek
  mentioned this for operator workloads)? One pool per namespace,
  or a pool that somehow spans namespaces?
