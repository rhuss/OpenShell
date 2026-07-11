# Data Model: Warm Pool gRPC PoC

**Branch**: `6113-warm-pool-grpc-poc` | **Date**: 2026-07-11

## Entities

### ActivateSandboxRequest (gRPC message)

Sent by the gateway to the supervisor at claim time.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| sandbox_id | string | yes | UUID of the sandbox being activated |
| sandbox_name | string | yes | Human-readable sandbox name |
| sandbox_token | string | yes | Gateway-minted JWT for the sandbox |
| gateway_endpoint | string | yes | Gateway gRPC endpoint (host:port) for supervisor to connect back |
| policy | SandboxPolicy | yes | Full policy config (reused from sandbox.proto) |

### ActivateSandboxResponse (gRPC message)

Returned by the supervisor to the gateway after activation attempt.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| success | bool | yes | Whether activation completed successfully |
| error_message | string | no | Human-readable error description (when success=false) |
| error_code | ErrorCode | no | Machine-readable error category (when success=false) |

### ErrorCode (enum)

| Value | Description |
|-------|-------------|
| UNSPECIFIED | Default/unknown error |
| INVALID_REQUEST | Missing or malformed request fields |
| POLICY_COMPILATION_FAILED | OPA policy compilation error |
| GATEWAY_UNREACHABLE | Cannot connect to gateway endpoint |
| TOKEN_INVALID | JWT validation or exchange failure |
| ALREADY_ACTIVATED | Supervisor already received a prior activation |
| INTERNAL | Unexpected internal error |

## External CRDs (read-only, managed by warm pool operator)

### SandboxWarmPool

| Status Field | Type | Description |
|-------------|------|-------------|
| readyReplicas | int | Count of pods ready to be claimed |
| spec.image | string | Container image the pool provisions |

### SandboxClaim

| Spec Field | Type | Description |
|-----------|------|-------------|
| warmPoolRef | string | Reference to the SandboxWarmPool |
| sandboxId | string | UUID of the sandbox claiming the pod |

| Status Field | Type | Description |
|-------------|------|-------------|
| phase | string | Pending, Bound, Ready, Failed |
| sandbox.podIP | string | IP address of the claimed pod |

## State Transitions

### Supervisor Lifecycle

```
Unidentified ──[ActivateSandbox]──> Activating ──[bootstrap complete]──> Running
                                        │
                                        └──[error]──> Failed (returns error response,
                                                       stays listening for retry or pod kill)
```

### Gateway Sandbox Creation (warm pool path)

```
Request ──[check warm pools]──> WarmPoolFound ──[create SandboxClaim]──> ClaimPending
    │                                                                        │
    └──[no pool]──> ColdStart                         [claim bound + pod IP]─┘
                                                             │
                                                    ActivateSandbox call
                                                             │
                                              ┌──────────────┴──────────────┐
                                          [success]                     [failure/timeout]
                                              │                              │
                                          SandboxReady                   ColdStartFallback
```
