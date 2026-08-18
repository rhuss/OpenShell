# Implementation Plan: Supervisor OTLP Relay

**Branch**: `001-supervisor-otlp-relay` | **Date**: 2026-08-13, evolved 2026-08-17 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/001-supervisor-otlp-relay/spec.md`, brainstorm 03-cross-driver-otlp-relay.md

## Summary

Add a telemetry relay to the sandbox supervisor that receives OTLP traces from agent processes over HTTP on `127.0.0.1:4318`, enriches them with sandbox resource attributes, buffers them in a bounded channel, and forwards them to the gateway over the existing session protocol. The gateway re-exports relayed spans through its configured OTLP exporter. OCSF events share the same transport. The relay is non-blocking to sandbox control operations and supports all four compute drivers (Docker, Podman, Kubernetes, VM). The relay is opt-in, activated only when the gateway has OTLP configured and confirms the `telemetry_relay` capability.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace uses `opentelemetry 0.32`, `tonic 0.14`, `tokio`)

**Primary Dependencies**:
- `opentelemetry 0.32` / `opentelemetry-otlp 0.32` / `tracing-opentelemetry 0.33` (existing workspace deps)
- `opentelemetry-proto` (NEW, for `ExportTraceServiceRequest` types)
- `hyper` (existing dep via tonic, for OTLP HTTP receiver)
- `prost` / `tonic-prost` (existing, for proto codegen)
- `serde_json` (existing, for JSON OTLP decoding)

**Storage**: In-memory bounded `tokio::sync::mpsc` channel (4096 spans default). No persistent storage.

**Testing**: `cargo test` (unit), `mise run e2e` (end-to-end with Docker/Podman driver)

**Target Platform**: Linux (sandbox supervisor runs inside sandbox host)

**Project Type**: Library crates integrated into the sandbox supervisor binary

**Performance Goals**: Handle 100 spans/sec sustained per sandbox without affecting control plane latency (2x baseline tolerance)

**Constraints**: <20 MB memory for buffer (4096 slots, shared between trace batches and OCSF events), non-blocking control plane, no agent-side configuration required, env vars only injected when relay active

**Scale/Scope**: Per-sandbox relay instance, one supervisor process per sandbox

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

No project constitution is configured (template placeholder). Gate passes by default.

**Post-Phase 1 Re-check**: The design follows existing codebase patterns (tokio tasks, mpsc channels, proto message extension, env var injection). No violations.

## Project Structure

### Documentation (this feature)

```text
specs/001-supervisor-otlp-relay/
├── plan.md              # This file
├── research.md          # Phase 0: research decisions
├── data-model.md        # Phase 1: entities, relationships, state transitions
├── quickstart.md        # Phase 1: validation scenarios
├── contracts/
│   ├── session-protocol.md  # Phase 1: TelemetryData proto contract
│   └── otlp-receiver.md     # Phase 1: OTLP HTTP receiver contract
└── tasks.md             # Phase 2 output (/speckit-tasks command)
```

### Source Code (repository root)

```text
proto/
└── openshell.proto              # TelemetryData message, SupervisorMessage variant,
                                 # capabilities fields on SupervisorHello/SessionAccepted

crates/
├── openshell-sandbox/
│   └── src/lib.rs               # Wire TelemetryRelay for all topologies:
│                                #   netns: bind_tcp_in_netns(127.0.0.1:4318)
│                                #   no-netns: direct bind 127.0.0.1:4318
│                                # Gate on network_enabled (not process_enabled)
│
├── openshell-supervisor-network/
│   └── src/
│       ├── otlp/                # OTLP receiver + enrichment + buffer
│       │   ├── mod.rs           # TelemetryRelay, RelayConfig, RateLimitedOcsfSink
│       │   ├── receiver.rs      # hyper HTTP server: accept TcpListener OR SocketAddr
│       │   ├── enrichment.rs    # Span resource attribute injection
│       │   └── buffer.rs        # Bounded buffer with TelemetryItem enum (Trace/Ocsf)
│       └── proxy.rs             # traceparent inject-if-missing on forward HTTP
│
├── openshell-supervisor-process/
│   └── src/
│       ├── child_env.rs         # otel_env_vars() for agent child processes
│       ├── process.rs           # Wire otel_env_vars() into entrypoint spawn
│       ├── ssh.rs               # Wire otel_env_vars() into SSH sessions
│       └── supervisor_session.rs # Capability negotiation + TelemetryData forwarding
│
├── openshell-ocsf/
│   └── src/tracing_layers/
│       └── relay_layer.rs       # OcsfRelayLayer: capture OCSF events for relay
│
├── openshell-server/
│   └── src/
│       ├── supervisor_session.rs # Handle TelemetryData: extract + re-export,
│       │                        # capability negotiation (confirm_capabilities)
│       └── telemetry_relay.rs   # Dedicated SpanExporter for relayed traces
│
├── openshell-driver-docker/
│   └── src/lib.rs               # REMOVE OTEL env vars from container env
│
├── openshell-driver-podman/
│   └── src/container.rs         # REMOVE OTEL env vars from container env
│
├── openshell-driver-kubernetes/
│   └── src/driver.rs            # ADD OTEL env vars to agent container spec
│                                # (K8s only: agent is separate container)
│
├── openshell-otel/
│   └── src/propagation.rs       # inject_traceparent_if_missing(), HeaderMapInjector
│
└── openshell-core/
    └── src/sandbox_env.rs       # OTEL_EXPORTER_OTLP_ENDPOINT constant
```

**Structure Decision**: The telemetry relay lives in `openshell-supervisor-network` alongside the proxy, since it shares the localhost bind lifecycle. The receiver always binds to `127.0.0.1:4318`. For Docker/Podman with a network namespace, the binding happens inside the namespace via `bind_tcp_in_netns()`. For Kubernetes and VM (shared namespace), the receiver binds directly. In the K8s sidecar topology, the relay runs in the network sidecar container (`network_enabled = true`), which owns the gateway session. The process supervisor in the agent container only sets OTEL env vars for agent child processes.

## Complexity Tracking

No constitution violations to justify.
