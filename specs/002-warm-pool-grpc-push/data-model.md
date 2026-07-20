# Data Model: Warm Pool gRPC-Push RFC

## Entities

### WarmPoolConfig (Gateway Configuration)

Gateway TOML configuration entry defining a warm pool for a specific sandbox image.

| Field | Type | Description |
|-------|------|-------------|
| image | string | Container image reference (e.g., `ghcr.io/nvidia/openshell-community/sandboxes/base:latest`) |
| replicas | integer | Number of pre-provisioned pods to maintain in the pool |

### ActivateSandboxRequest (gRPC Message)

Message sent from gateway to supervisor after claim binding to inject sandbox identity.

| Field | Type | Description |
|-------|------|-------------|
| sandbox_id | string (UUID) | Unique identifier for this sandbox session |
| sandbox_name | string | Human-readable sandbox name |
| policy_config | PolicyConfig | Sandbox-specific policy rules to compile at claim time |
| provider_environment | map<string, string> | Provider credentials and environment variables |

### ActivateSandboxResponse (gRPC Message)

Response from supervisor confirming activation.

| Field | Type | Description |
|-------|------|-------------|
| status | enum (OK, FAILED) | Whether activation succeeded |
| error_message | string | Error details if status is FAILED |
| ready_at | timestamp | When the supervisor became ready |

### GlobalPolicyBundle

Pre-compiled OPA policy set applicable to all sandboxes, compiled at pool provisioning time.

| Field | Type | Description |
|-------|------|-------------|
| compiled_modules | bytes | Pre-compiled OPA modules (network rules, filesystem constraints) |
| version | string | Policy version hash for staleness detection |

### SandboxPolicyDelta

Sandbox-specific OPA rules compiled at claim time.

| Field | Type | Description |
|-------|------|-------------|
| sandbox_rules | list<PolicyRule> | Per-sandbox rules (provider constraints, custom policies) |
| merged_with_global | string | Version of the global bundle this delta was merged with |

## Relationships

```
WarmPoolConfig --[1:1]--> SandboxTemplate (K8s resource, gateway-managed)
WarmPoolConfig --[1:1]--> SandboxWarmPool (K8s resource, gateway-managed)
SandboxWarmPool --[1:N]--> Warm Pods (pre-provisioned, supervisor running)
SandboxClaim --[1:1]--> Warm Pod (adopted at claim time)
ActivateSandboxRequest --[1:1]--> Warm Pod (gRPC push after claim binding)
GlobalPolicyBundle --[1:1]--> Warm Pod (compiled at pool time)
SandboxPolicyDelta --[1:1]--> Activated Pod (compiled at claim time)
```

## State Transitions

### Supervisor State Machine

```
INITIALIZING --> IDLE --> ACTIVATING --> ACTIVE --> TERMINATED
```

| State | Description | Health endpoint |
|-------|-------------|----------------|
| INITIALIZING | Supervisor starting, global OPA compiling | `/readyz` 503 |
| IDLE | Global policies compiled, waiting for identity push | `/readyz` 200 |
| ACTIVATING | Received identity, compiling sandbox-specific policies | `/readyz` 200 |
| ACTIVE | Fully activated, SSH ready, serving session | `/readyz` 200 |
| TERMINATED | Session ended, pod will be deleted by operator | `/readyz` 503 |

### Pool Pod Lifecycle

```
Pool creates pod --> Supervisor reaches IDLE --> Pod marked Ready in pool
--> Claim adopts pod --> Gateway pushes identity --> Supervisor ACTIVE
--> Session ends --> Pod deleted --> Pool replenishes with fresh pod
```
