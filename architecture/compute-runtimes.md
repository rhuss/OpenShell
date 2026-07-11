# Compute Runtimes

Compute runtimes create, stop, delete, and watch sandbox workloads for the
gateway. They do not replace sandbox policy enforcement. Every runtime starts a
workload that runs the `openshell-sandbox` supervisor, and the supervisor
enforces the sandbox contract locally.

## Driver Contract

Each runtime receives a sandbox spec from the gateway and is responsible for:

- Selecting the sandbox image.
- Injecting sandbox identity and gateway callback configuration.
- Supplying TLS or secret material for supervisor callbacks.
- Providing the supervisor binary or image in the workload.
- Reporting lifecycle and platform events back to the gateway.
- Cleaning up runtime-owned resources.

Drivers own runtime-specific platform event interpretation. When an event should
drive client provisioning UI, the driver attaches the shared
`openshell.progress.*` metadata defined in `openshell-core` instead of requiring
clients to parse Kubernetes reasons, VM cache states, or other driver-local
reason strings.

The capability RPC reports driver identity, version, and the default sandbox
image used by the gateway. GPU availability stays driver-local and is validated
when a sandbox create request asks for GPU resources.

## Runtime Summary

| Runtime | Best fit | Sandbox boundary | Notes |
|---|---|---|---|
| Docker | Local development with Docker available. | Container plus nested sandbox namespace. | Uses host networking so loopback gateway endpoints work from the supervisor. |
| Podman | Rootless or single-machine deployments. | Container plus nested sandbox namespace. | Uses the Podman REST API, OCI image volumes, and CDI GPU devices when available. |
| Kubernetes | Cluster deployment through Helm. | Pod plus nested sandbox namespace. | Uses Kubernetes API objects, service accounts, secrets, PVC-backed workspace storage, and GPU resources. |
| VM | Experimental microVM isolation. | Per-sandbox libkrun VM. | Managed endpoint-backed driver. The gateway spawns `openshell-driver-vm`, waits for its Unix socket, and then consumes it through the same remote `compute_driver.proto` path used by unmanaged endpoint drivers. The VM driver boots a cached bootstrap `rootfs.ext4`, prepares requested OCI images inside a bootstrap VM with `umoci`, attaches the prepared image disk read-only, and gives each sandbox a writable `overlay.ext4` for merged-root changes and runtime material. The driver persists each accepted launch request beside the overlay and restarts those VMs on driver startup without recreating the overlay. |
| Extension | Out-of-tree drivers operated alongside the gateway. | Whatever boundary the driver implements. | Selected by a non-reserved custom `compute_drivers = ["<name>"]` entry with `[openshell.drivers.<name>].socket_path`, or at launch time by pairing `--drivers <name>` with `--compute-driver-socket=<path>`. Reserved built-in names such as `vm`, `docker`, `podman`, and `kubernetes` cannot be used as unmanaged socket endpoints. The gateway connects to a UDS the operator already provisioned, runs `GetCapabilities`, logs the advertised `driver_name`, and dispatches all sandbox lifecycle calls through `compute_driver.proto`. The driver process and socket lifecycle are operator-owned; the gateway does not spawn, supervise, or remove unmanaged extension drivers. The trust boundary is the socket's filesystem permissions: the operator must ensure only the gateway uid can read/write it. |

Per-sandbox CPU and memory values currently enter the driver layer through
template resource limits. Docker and Podman apply them as runtime limits.
Kubernetes mirrors each limit into the matching request. VM accepts the fields
but currently ignores them.

Docker and Podman also accept per-sandbox driver-config mounts for existing
runtime-managed named volumes and tmpfs mounts. Podman additionally accepts
image mounts through its image-volume API. User-supplied bind and volume mounts
default to read-only. Direct host bind mounts, and Docker or Podman local-driver
bind-backed named volumes, are available only when explicitly enabled in the
active local driver table of `gateway.toml`. Host bind mounts are an unsafe
operator override because they place gateway-host filesystem state inside the
sandbox and can negate OpenShell workspace isolation and filesystem-policy
controls. Driver-owned supervisor, token, and TLS bind mounts stay reserved.

Kubernetes deployments may set an AppArmor profile on sandbox agent containers
through the driver configuration. The Helm chart defaults sandbox agents to
`Unconfined` so runtime/default AppArmor profiles do not block supervisor
network namespace setup on AppArmor-enabled nodes.

Resource requirements enter the driver layer through `SandboxSpec.resource_requirements`. This includes a set of GPU requirements, where a user
can request a specific number of GPUs or the driver-specific default behaviour.
For all in-tree drivers, this is equivalent to selecting a single GPU.

VM runtime state paths are derived only from driver-validated sandbox IDs
matching `[A-Za-z0-9._-]{1,128}`. The gateway-owned VM driver socket uses a
private `run/` directory plus Unix peer UID/PID checks. Standalone
unauthenticated TCP mode is disabled unless explicitly enabled for local
development.

Runtime-specific implementation notes belong in the driver crate README:

- `crates/openshell-driver-docker/README.md`
- `crates/openshell-driver-podman/README.md`
- `crates/openshell-driver-kubernetes/README.md`
- `crates/openshell-driver-vm/README.md`

## Supervisor Delivery

The supervisor must be available inside each sandbox workload:

| Runtime | Delivery model |
|---|---|
| Docker | Bind-mounted local supervisor binary, or a binary extracted from the configured supervisor image. |
| Podman | Read-only OCI image volume containing the supervisor binary. |
| Kubernetes | Supervisor image side-loaded into the sandbox pod by image volume or init container. |
| VM | Embedded in the guest rootfs bundle. |
| Extension | Defined by the out-of-tree driver. |

Driver-controlled environment variables must override sandbox image or template
values for sandbox ID, sandbox name, gateway endpoint, relay socket path, TLS
paths, and command metadata.

Kubernetes can run the supervisor in the default combined topology or in a
sidecar topology. Combined mode keeps network and process supervision in the
agent container. Sidecar mode runs network enforcement, the proxy, and gateway
session in a dedicated sidecar, while the agent container runs only the
process-supervision leaf and launches the user workload after the sidecar
serves bootstrap state over a local control socket. The network sidecar owns
gateway credentials and sends policy plus workload-facing provider environment
state to the process leaf over that socket. It also streams provider
environment updates after settings polls so future process sessions see
updated provider env without giving the process leaf gateway access. The
pre-workload process supervisor is the only accepted control client: the
network sidecar verifies its UID, GID, and PID with peer credentials, removes
the listener after accepting it, and ignores workload-supplied relay targets.
SSH relays use a Linux abstract socket and verify its peer PID against that
authenticated process-supervisor connection, so workload filesystem access
cannot replace the relay endpoint. Either supervisor exits when this control
connection closes. This couples their restart lifecycle and prevents a workload
that survives an isolated network-sidecar restart from becoming the next
authoritative control client. In sidecar mode, an init container performs the
privileged pod-network nftables setup with
`NET_ADMIN`. The default binary-aware network sidecar runs as UID 0 without
`NET_ADMIN` and adds `SYS_PTRACE` plus `DAC_READ_SEARCH` so it can resolve
cross-UID workload process/binary identity through shared `/proc`. Operators
can set the sidecar `process_binary_aware_network_policy` flag false to run the
sidecar as the configured non-root proxy UID, omit both inspection capabilities,
and downgrade network policy to endpoint/L7 matching without `policy.binaries`.
The init path applies nftables as individual commands so optional conntrack and
log expressions can fail without rolling back the required table, chain, and
reject rules.
The agent container runs as the resolved sandbox UID/GID with no added Linux
capabilities. Sidecar mode preserves gateway session and SSH behavior, but
treats the process leaf as network-only: Landlock filesystem policy and child
seccomp still apply where supported, while process privilege dropping and
supervisor identity mount isolation do not run because the agent container is
already unprivileged. Sidecar pods use a shared process namespace so the
network sidecar can resolve workload process and binary identity through
`/proc/<entrypoint-pid>`.

## Warm Pool (Kubernetes)

The Kubernetes driver supports an optional warm pool fast path that claims
a pre-provisioned pod instead of creating a new Sandbox CRD. When a warm
pool is available, sandbox creation skips image pull and pod scheduling
entirely.

On every `create_sandbox` call the driver runs `try_warm_pool()` before
the cold-start path. The sequence is:

1. **Pool discovery.** List `SandboxWarmPool` CRDs
   (`agents.x-k8s.io/v1alpha1`) in the driver namespace. Match by exact
   container image and `status.readyReplicas > 0`.
2. **Claim.** Create a `SandboxClaim` CRD referencing the matched pool
   name and sandbox ID. Poll the claim until `status.phase` reaches
   `Ready` (10-second timeout, 200 ms poll interval). Extract the
   assigned pod IP from `status.sandbox.podIP`.
3. **Activation.** Connect to the supervisor gRPC endpoint at
   `pod_ip:9090` and call `ActivateSandbox`
   (`openshell.supervisor.v1.Supervisor`). The request carries the
   sandbox ID, name, gateway-minted token, gateway endpoint, and sandbox
   policy. The supervisor compiles OPA rules, connects back to the
   gateway, and starts the entrypoint. A 5-second timeout wraps the
   activation RPC.
4. **Result.** If the activation response has `success = true`, the
   driver returns immediately and the cold-start path is skipped. Any
   failure at any step (no matching pool, claim timeout, activation
   error, non-success response) returns `None` from `try_warm_pool()`
   and the driver falls through to the normal Sandbox CRD creation.

The warm pool path is implemented in two modules inside
`crates/openshell-driver-kubernetes/`:

- `warm_pool.rs` handles SandboxWarmPool listing, image matching, claim
  creation, and claim polling.
- `activation_client.rs` provides the gRPC client for the supervisor
  `ActivateSandbox` RPC.

The supervisor-side `ActivateSandbox` service is defined in
`proto/supervisor.proto`. The supervisor hosts this service (port 9090)
while waiting in an unactivated warm pool state. Once activated, the
supervisor transitions to its normal gateway-connected lifecycle.

Fallback is silent: warm pool failures produce `debug!` or `warn!` log
lines but do not propagate errors to the caller. The combined claim and
activation timeouts (10 s + 5 s) bound the worst-case delay before
cold-start fallback begins.

## Images

The gateway image and Helm chart are built from this repository. Sandbox images
are maintained separately in the OpenShell Community repository or supplied by
users.

Custom sandbox images must include the agent runtime and any system
dependencies, but they should not need to include the gateway. GPU-capable
images must include the user-space libraries required by the workload. The
runtime still owns GPU device injection. GPU requests are explicit, and can be
refined with a driver-native device identifier or requested count; the gateway
validates the request shape and each runtime enforces the GPU allocation modes it
supports.

## Deployment Shape

Kubernetes deployments use the Helm chart under `deploy/helm/openshell`. The
chart deploys the gateway and sandbox runtime integration. The default gateway
workload is a StatefulSet for SQLite-backed single-replica installs. External
database-backed installs can render a Deployment with `workload.kind=deployment`;
HA deployments must point `server.externalDbSecret` at an operator-managed
PostgreSQL database.
Standalone local deployments start the gateway with a selected runtime such as
Docker, Podman, or VM. The CLI can register multiple gateways and switch between
them without changing the sandbox architecture.

When runtime infrastructure changes, validate the relevant sandbox e2e path and
update the matching driver README if a maintainer-facing constraint changes.
