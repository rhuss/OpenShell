# Research: Warm Pool gRPC PoC

**Branch**: `6113-warm-pool-grpc-poc` | **Date**: 2026-07-11

## R-001: Supervisor gRPC Server Infrastructure

**Decision**: Add a new tonic gRPC server to the supervisor binary.

**Rationale**: The supervisor is currently a pure gRPC client with zero server infrastructure. All existing RPCs (IssueSandboxToken, GetSandboxConfig, ConnectSupervisor) are hosted on the gateway's `OpenShell` service and called *by* the supervisor. The ActivateSandbox RPC reverses this direction (gateway calls supervisor), requiring a new server.

**Alternatives considered**:
- Reuse the sidecar control Unix socket protocol: rejected because the gateway needs a network-accessible endpoint, not a local socket.
- HTTP endpoint instead of gRPC: rejected because the project already uses tonic/prost extensively and gRPC provides typed contracts.

**Key files**:
- `crates/openshell-sandbox/src/main.rs:428` - entry point
- `crates/openshell-sandbox/src/lib.rs:92-736` - `run_sandbox()` monolithic startup

## R-002: Supervisor Health Check Status

**Decision**: Wire up the existing `--health-check` / `--health-port` CLI args to serve a real HTTP health endpoint.

**Rationale**: The args exist (main.rs:170-176) and are parsed by clap, but `run_sandbox()` receives them as `_health_check: bool` and `_health_port: u16` (leading underscores, completely ignored). In unidentified mode, the health endpoint signals readiness to receive ActivateSandbox. The existing plumbing (port 8080 default) is ready to use.

**Alternatives considered**:
- gRPC health check service (grpc.health.v1): would work but adds complexity. A simple HTTP `/readyz` is sufficient for K8s readinessProbe and aligns with the spec.
- New CLI flag: rejected because `--health-check` already exists and is the right name.

## R-003: Proto Service Placement

**Decision**: Create a new `Supervisor` service in a new `supervisor.proto` file (package `openshell.supervisor.v1`).

**Rationale**: The existing services are: `OpenShell` (gateway-hosted, openshell.proto), `ComputeDriver` (driver-hosted, compute_driver.proto), `Inference` (inference, inference.proto). None are supervisor-hosted. A new service cleanly separates the supervisor's server-side API from the gateway's client-side RPCs.

**Alternatives considered**:
- Add to `openshell.proto`: rejected because that file defines the `OpenShell` service which is gateway-hosted. Adding a supervisor-hosted RPC there would be confusing.
- Add to `sandbox.proto`: rejected because that file contains message definitions, not service definitions.

## R-004: Warm Pool CRD Integration

**Decision**: The K8s driver will interact with SandboxWarmPool and SandboxClaim CRDs from the external warm pool operator (feasibility study). No CRD definitions are added to this repo.

**Rationale**: The warm pool operator manages the CRDs. The K8s driver only needs to: (1) list SandboxWarmPools to check for image matches, (2) create SandboxClaim objects to request a pod, (3) watch SandboxClaim status for pod IP. All via kube-rs dynamic API.

**Key finding**: The existing K8s driver uses `DynamicObject` for Sandbox CRD interactions (`driver.rs:776-901`). The same pattern applies to warm pool CRDs.

**CRD details from feasibility study**:
- `SandboxWarmPool`: group `agents.x-k8s.io`, lists available pools with `readyReplicas`
- `SandboxClaim`: group `agents.x-k8s.io`, binds a warm pod to a sandbox, reports pod IP in status
- `SandboxTemplate`: group `agents.x-k8s.io`, defines the pod spec for warm pods

## R-005: mTLS for Gateway-to-Supervisor Channel

**Decision**: Reuse the existing namespace mTLS certificates for the ActivateSandbox channel.

**Rationale**: The K8s driver already configures mTLS via `client_tls_secret_name` in `KubernetesComputeConfig`. The TLS material (ca.crt, tls.crt, tls.key) is mounted at `/etc/openshell-tls/client/` in the combined topology. The gateway holds the same CA and can establish a TLS connection to the supervisor's gRPC port.

**Key files**:
- `crates/openshell-driver-kubernetes/src/config.rs:258` - `client_tls_secret_name` config
- `crates/openshell-sandbox/src/grpc_client.rs` - TLS channel setup (client side, can be mirrored for server)

## R-006: Supervisor Startup Phasing

**Decision**: Split `run_sandbox()` into two phases: pre-activation (unidentified mode) and post-activation (normal startup).

**Rationale**: The monolithic `run_sandbox()` (lib.rs:92-736, ~16 params) does everything sequentially: OCSF context, policy load, provider fetch, networking, process start. In unidentified mode, the supervisor should only: (1) initialize OCSF context with minimal info, (2) start the gRPC server, (3) start the health endpoint. Everything else happens after ActivateSandbox provides identity and policy.

**Post-activation sequence** (triggered by ActivateSandbox):
1. Store identity (sandbox_id, name, JWT)
2. Compile OPA policies from provided config
3. Call IssueSandboxToken (using provided JWT for initial auth)
4. Call GetSandboxConfig for full settings
5. Fetch provider environment
6. Start networking (proxy, OPA enforcement)
7. Start ConnectSupervisor session
8. Start entrypoint process
9. Return success to gateway

## R-007: Token Acquisition in Warm Pool Path

**Decision**: The ActivateSandbox request carries the gateway-minted JWT directly. No K8s SA token exchange needed.

**Rationale**: In the cold-start path, the supervisor reads a projected K8s SA token and exchanges it for a gateway JWT via `IssueSandboxToken` (grpc_client.rs:288-316). In the warm pool path, the gateway already has the JWT (it minted it during sandbox create) and passes it directly in the ActivateSandbox request. The supervisor uses this JWT for all subsequent gateway RPCs.

**Alternatives considered**:
- K8s SA token in warm pod + exchange at activation: rejected because the warm pod's SA token is not bound to a specific sandbox. The gateway-minted JWT is the correct identity credential.
