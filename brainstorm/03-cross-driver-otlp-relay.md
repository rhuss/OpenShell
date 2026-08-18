# Cross-Driver OTLP Relay Design

**Date**: 2026-08-16
**Status**: spec-created (evolving spec 001-supervisor-otlp-relay)
**Context**: During implementation of the supervisor OTLP relay (branch `6115-supervisor-otlp-relay`), the guided demo revealed design issues that need resolution before the feature ships across all compute drivers.

## Problem Statement

The initial OTLP relay implementation scoped to Docker and Podman only. Three issues surfaced:

1. **Address mismatch**: Docker/Podman drivers inject `OTEL_EXPORTER_OTLP_ENDPOINT=http://host.openshell.internal:4318` at container creation time. This resolves to the Docker host IP (outside the container), but the OTLP receiver binds to `10.200.0.1:4318` (the veth host-side IP inside the container). The agent would try to reach the wrong address.

2. **Wrong injection point**: OTEL env vars are set by the compute drivers for the supervisor container. They should be set by the supervisor for the agent process (like proxy env vars are via `child_env.rs`). The supervisor knows its own bind address and whether the relay is active.

3. **Missing driver support**: VM and Kubernetes drivers have no relay support, but the architecture naturally supports them.

## Hard Requirement

The OTLP receiver MUST always bind to `127.0.0.1:4318` (localhost). The agent always exports via the supervisor relay, never directly to the gateway. This is uniform across all drivers.

## Driver Architecture Analysis

### Docker Driver (`crates/openshell-driver-docker/src/lib.rs`)

- **Supervisor deployment**: Host-side bind-mount of a Linux ELF binary into the container at `/opt/openshell/bin/openshell-sandbox`. Extracted from a supervisor Docker image and cached on the host keyed by digest. On macOS, the bind-mount fails due to virtiofs translating files as directories.
- **Network**: Bridge network with `host.openshell.internal` extra_hosts entry resolving to the Docker host IP. Supervisor creates a veth-based network namespace inside the container (10.200.0.1 host side, 10.200.0.2 sandbox side).
- **OTLP relay path**: Receiver binds inside the sandbox netns on localhost:4318 using `bind_tcp_in_netns()`. Agent at 10.200.0.2 reaches `127.0.0.1:4318` via the netns loopback.

### Podman Driver (`crates/openshell-driver-podman/src/container.rs`)

- **Supervisor deployment**: OCI image volume (`type=image`) mounts the supervisor image directly. No host-side bind-mount, so it works on macOS.
- **Network**: Same as Docker (bridge network, host alias, supervisor creates netns internally).
- **OTLP relay path**: Identical to Docker. Same netns architecture inside the container.

### Kubernetes Driver (`crates/openshell-driver-kubernetes/src/driver.rs`)

Two topologies:

- **Combined**: Supervisor side-loaded into the agent container (via ImageVolume on K8s >= 1.33 or init container copy). Supervisor and agent share the pod network namespace. OTLP receiver binds to `127.0.0.1:4318`, directly reachable from agent processes.

- **Sidecar**: Network sidecar container runs the supervisor with `--mode=network` (gateway credentials, proxy, enforcement). Agent container runs `--mode=process` (entrypoint lifecycle, SSH). Both share the pod network via `shareProcessNamespace: true`. OTLP receiver lives in the network sidecar (which owns gateway session connectivity). Agent container reaches it at `127.0.0.1:4318`.

- **Key difference**: In K8s, the agent runs in a separate container (not a supervisor child process), so the K8s driver must set `OTEL_EXPORTER_OTLP_ENDPOINT` in the agent container's env vars. This is the one case where the driver, not the supervisor, sets the OTEL env vars.

### VM Driver (`crates/openshell-driver-vm/`)

- **Supervisor deployment**: Binary embedded at compile time via `include_bytes!()`, zstd-compressed, decompressed into the ext4 rootfs at `/opt/openshell/bin/openshell-sandbox`.
- **Network**: libkrun uses gvproxy (gvisor-tap-vsock) for guest networking. QEMU uses TAP devices. `host.openshell.internal` resolves via gvproxy DNS or `/etc/hosts`.
- **OTLP relay path**: Supervisor runs as init inside the guest. Agent processes are children of the supervisor. All share the guest kernel. OTLP receiver binds to `127.0.0.1:4318`, directly reachable from agent processes. No special handling needed.

## Proposed Design: Always Localhost

### Bind Address (uniform)

| Topology | How `TcpListener` is Created | Agent Env |
|----------|------------------------------|-----------|
| Docker/Podman (netns) | `bind_tcp_in_netns(127.0.0.1:4318)` | `http://127.0.0.1:4318` |
| K8s combined | `TcpListener::bind(127.0.0.1:4318)` | `http://127.0.0.1:4318` |
| K8s sidecar | `TcpListener::bind(127.0.0.1:4318)` | `http://127.0.0.1:4318` |
| VM (libkrun/QEMU) | `TcpListener::bind(127.0.0.1:4318)` | `http://127.0.0.1:4318` |

For Docker/Podman with netns: use the existing `bind_tcp_in_netns()` helper (`crates/openshell-supervisor-process/src/netns/mod.rs:330`). This spawns a dedicated OS thread that enters the namespace via `setns()`, binds the `TcpListener`, and returns it. The supervisor's tokio runtime then handles I/O on the returned listener from outside the netns. Same proven pattern.

For K8s and VM: no special binding needed. `127.0.0.1` is shared between supervisor and agent.

### Required Changes

#### 1. Remove driver-level OTEL env var injection

Remove `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` from `build_environment_for_oci_user()` (Docker) and `build_env()` (Podman). These set container-level env for the supervisor, not agent-level env.

#### 2. Modify OTLP receiver to accept pre-bound listener

Change `spawn_receiver()` in `otlp/receiver.rs` to accept either a pre-bound `TcpListener` (for netns) or a `SocketAddr` (for direct bind). When a netns exists, the caller provides a listener bound inside the namespace.

#### 3. Enable relay for all topologies

In `run_sandbox()`, start the relay on all Linux topologies:
- If netns exists: use `bind_tcp_in_netns()` for the listener
- If no netns (sidecar, VM, combined K8s): bind `127.0.0.1:4318` directly
- On non-Linux: skip

#### 4. Wire `otel_env_vars()` into agent process spawn

In `run_process()`, add `otel_env_vars("http://127.0.0.1:4318", "http/protobuf")` alongside existing `proxy_env_vars()` and `tls_env_vars()`, conditional on the relay being active.

#### 5. K8s driver: set OTEL env for agent container

Add `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318` and `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` to the agent container env in both combined and sidecar topologies. This is the one driver that must set these vars (agent is a separate container, not a supervisor child process).

#### 6. K8s sidecar: relay in network sidecar

The OTLP relay belongs in the network sidecar (which owns the gateway session). Gate relay startup on `network_enabled`.

#### 7. VM driver: no changes needed

The supervisor handles everything inside the guest.

### Architecture Diagram

```
ALL TOPOLOGIES:
  [supervisor] -> starts OTLP receiver @ 127.0.0.1:4318
              -> spawns agent with env: OTEL_ENDPOINT=http://127.0.0.1:4318
              -> agent exports to localhost -> receiver enriches -> buffer -> session -> gateway

Docker/Podman (netns):
  Receiver bound INSIDE sandbox netns via bind_tcp_in_netns()
  Agent process runs inside same netns, reaches localhost:4318

K8s (combined or sidecar):
  Receiver bound on pod-shared localhost
  Agent container shares pod network, reaches localhost:4318

VM:
  Receiver bound on guest localhost
  Agent process is supervisor child, same guest kernel
```

Session forwarding (supervisor -> gateway -> external collector) is identical across all drivers.

## Key Files

| File | Change |
|------|--------|
| `crates/openshell-driver-docker/src/lib.rs` | Remove OTEL env vars from `build_environment_for_oci_user()` |
| `crates/openshell-driver-podman/src/container.rs` | Remove OTEL env vars from `build_env()` |
| `crates/openshell-supervisor-network/src/otlp/receiver.rs` | Accept pre-bound `TcpListener` or `SocketAddr` |
| `crates/openshell-sandbox/src/lib.rs` | Start relay for all topologies, use `bind_tcp_in_netns` when netns present |
| `crates/openshell-supervisor-process/src/run.rs` | Wire `otel_env_vars()` into agent child process env |
| `crates/openshell-driver-kubernetes/src/driver.rs` | Add OTEL env vars to agent container (both topologies) |
| `crates/openshell-supervisor-process/src/netns/mod.rs` | Existing `bind_tcp_in_netns()` reused as-is |

## Open Questions

1. Should the relay be opt-in (requires gateway OTLP config) or always-on (starts optimistically, forwarder gates on capability negotiation)?
2. Port 4318 is hardcoded. Should it be configurable? The proxy port (3128) is configurable via policy.
3. For the K8s sidecar, should the network sidecar expose port 4318 explicitly in the pod spec, or is pod-local localhost sufficient?
