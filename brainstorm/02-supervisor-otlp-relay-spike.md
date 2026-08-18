# Brainstorm: Supervisor OTLP Relay Spike

**Date:** 2026-08-13 (updated 2026-08-15)
**Status:** active

## Problem Framing

Agents running inside network-isolated OpenShell sandboxes cannot export OTel traces to external collectors. The sandbox's egress policy blocks direct OTLP export, and per-sandbox policy exceptions for collector endpoints are operationally impractical. Agent developers lose visibility into tool calls, LLM invocations, and reasoning steps when running inside sandboxes.

The current workaround (NemoClaw's `diagnostics-otel` plugin + socat forwarder + network policy exception) proves the value of agent traces in observability backends like MLflow, but it's fragile and NemoClaw-specific. The proper solution is for the OpenShell supervisor to relay agent traces natively.

This spike produces a working prototype of the supervisor-to-gateway OTLP relay on the Docker/Podman compute driver.

**Upstream issues:**
- [#2641](https://github.com/NVIDIA/OpenShell/issues/2641): Agent-level trace collection via supervisor OTLP relay
- [#2642](https://github.com/NVIDIA/OpenShell/issues/2642): Shared telemetry relay infrastructure
- [#2698](https://github.com/NVIDIA/OpenShell/issues/2698): Noisy infrastructure traces (54K spans/day idle, 664:1 ratio)

## Approaches Considered

### A: Reuse existing PushSandboxLogs

Add an OTLP HTTP receiver in the supervisor, serialize accepted spans, and forward via the existing `PushSandboxLogs` session protocol message. The gateway deserializes and re-exports through its own OTLP exporter.

- Pros: Smallest change, reuses existing transport, minimal protocol changes
- Cons: `PushSandboxLogs` is designed for log lines, not structured trace data. Retrofitting it for traces conflates two different data models. Harder to evolve independently

### B: New PushTelemetry session protocol message (Chosen)

Add a dedicated `PushTelemetry` message type to the session protocol. The supervisor serializes OTLP spans into this message. The gateway has a handler that deserializes and feeds them to its OTLP exporter.

- Pros: Clean separation from logs. Can carry traces, metrics, and OCSF events in the future. The message type documents its intent. Better foundation for production (#2642)
- Cons: Requires protobuf updates and new session protocol handling. More work for a prototype

### C: gRPC OTLP passthrough

Tunnel raw OTLP gRPC from the sandbox through the supervisor to the gateway. The gateway runs an OTLP receiver.

- Pros: No serialization/deserialization in the supervisor. Standard OTLP end-to-end
- Cons: Supervisor becomes a dumb tunnel, losing enrichment capability. Harder to add backpressure. Requires a new gRPC stream or separate channel

## Decision

Approach B: New `PushTelemetry` session protocol message. Clean protocol design is worth the extra effort because this message type will eventually carry supervisor traces (#2508), agent traces (#2641), and OCSF events. Starting with the right abstraction avoids a retrofit later.

## Key Requirements

1. **OTLP HTTP receiver in supervisor**: Listen on port 4318, bound to `netns.host_ip()` (reachable from the sandbox's workload network namespace). Accept spans from any OTel-instrumented agent framework. Set `TCP_NODELAY` on accepted connections via `openshell_core::net::set_tcp_nodelay_best_effort` to avoid delayed-ACK stalls on small OTLP batches
2. **Automatic environment setup**: Set `OTEL_EXPORTER_OTLP_ENDPOINT` in the agent process environment, pointing at the supervisor's OTLP receiver. Zero-config for agents
3. **Span enrichment**: Add resource attributes to forwarded spans: `openshell.sandbox.id`, `openshell.workspace.id`, `openshell.sandbox.policy_hash`, and `openshell.telemetry.source` (set to `"agent"` for relayed spans, distinguishing them from `"gateway"` infrastructure spans, see #2698 interaction below)
4. **Session protocol capability negotiation**: Add a `capabilities` field to `SupervisorHello` and a corresponding field to `SessionAccepted` so the supervisor advertises telemetry support and the gateway confirms it understands. The supervisor must not send `TelemetryData` messages unless the gateway acknowledged the capability
5. **New session protocol message**: `TelemetryData` variant in `SupervisorMessage.oneof payload` carrying serialized OTLP spans from supervisor to gateway
6. **Gateway forwarding**: Gateway receives `TelemetryData`, deserializes spans, feeds them to a **dedicated `SpanExporter`** (not the gateway's own `SdkTracerProvider`, which would overwrite the supervisor-enriched resource attributes with gateway identity)
7. **Docker/Podman driver target**: Prototype on the local compute driver where the supervisor creates a workload netns via `setns()`

## Scope

**In scope for prototype:**
- OTLP HTTP receiver in supervisor (port 4318, TCP_NODELAY on accepted connections)
- `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` environment variable injection (new `otel_env_vars()` in `child_env.rs` + Docker/Podman driver env builders)
- Session protocol capability negotiation (`capabilities` field in `SupervisorHello` / `SessionAccepted`)
- `TelemetryData` variant in `SupervisorMessage.oneof payload` (protobuf definition + handler)
- Span enrichment with sandbox resource attributes, including `openshell.telemetry.source: "agent"`
- Gateway forwarding via a dedicated `SpanExporter` (not the gateway's own `SdkTracerProvider`)
- Basic in-memory buffering (bounded `mpsc` channel, drop-oldest on overflow)
- Docker/Podman driver only

**Out of scope for prototype (future work from #2642):**
- Backpressure signaling from gateway to supervisor
- Head sampling under load
- OCSF log relay (separate data model, shares transport)
- Per-sandbox rate limits with drop counters
- gRPC OTLP receiver (port 4317, HTTP-only for prototype)
- Kubernetes sidecar driver support
- VM driver support
- Gateway-side `openshell.telemetry.source: "gateway"` attribute for infrastructure spans (tracked by #2698)
- W3C traceparent injection on forward-proxy HTTP requests (FR-009, narrow scope: only applies to non-CONNECT forward HTTP, not tunneled HTTPS)

## Open Questions (Resolved 2026-08-15)

All original open questions have been answered through codebase research:

- **Session protocol format?** Protobuf. `SupervisorMessage` in `proto/openshell.proto:2039-2046` has a `oneof payload` with four variants today: `SupervisorHello`, `SupervisorHeartbeat`, `RelayOpenResult`, `RelayClose`. The bidi-streaming `ConnectSupervisor` RPC carries these. Adding a `TelemetryData` variant follows the established extension pattern.

- **How does the supervisor bind to `netns.host_ip()`?** Via `proxy_bind_ip: Option<IpAddr>` passed into `run_networking()` at `crates/openshell-supervisor-network/src/run.rs:287-295`. The orchestrator provides the host-side veth IP. The proxy binds to port 3128 by default. The OTLP receiver should follow the same pattern, binding to the same IP on port 4318.

- **Does the gateway OTLP exporter support external span injection?** No. The `SdkTracerProvider` (built in `crates/openshell-otel/src/lib.rs:151`) only exports spans it creates internally. Feeding relayed spans through it would overwrite resource attributes. A **dedicated `SpanExporter`** is needed that reuses the same OTLP endpoint config (`[openshell.gateway.otlp]`).

- **Raw bytes or structured representation?** Raw OTLP protobuf bytes (opaque `bytes` field in `TelemetryData`). The supervisor serializes the enriched `ExportTraceServiceRequest` after enrichment; the gateway deserializes and feeds to the dedicated exporter. This keeps the session protocol agnostic to OTLP schema changes.

- **Buffering strategy?** Bounded `tokio::sync::mpsc` channel with capacity 4096, drop-oldest semantics via a custom wrapper that tracks drops. Matches existing patterns in the supervisor (denial and activity aggregation use mpsc channels). A separate forwarding task drains and batches into `TelemetryData` messages.

## Remaining Open Questions

- Should the `TelemetryData` message carry a batch sequence number for observability (detecting gaps at the gateway)?
- What drain timeout is appropriate during sandbox teardown? Too short loses final spans; too long delays container cleanup

## Findings from Codebase Research (2026-08-15)

### Backward Compatibility: Gateway Catch-All Logs Warnings

The gateway's `handle_supervisor_message()` at `supervisor_session.rs:883-930` has a `_ =>` catch-all arm that **logs a warning per unknown message**, not a silent discard. The spec's assumption that "protobuf unknown-field behavior ensures older gateways silently discard" is incorrect at the application layer. A newer supervisor sending `TelemetryData` to an older gateway would spam warnings in the gateway logs.

**Mitigation**: Add capability negotiation to `SupervisorHello` / `SessionAccepted`. The supervisor only sends telemetry if the gateway acknowledged the capability. This also future-proofs the protocol for additional extensions.

### No Version Negotiation in Session Protocol

`SupervisorHello` contains only `sandbox_id` (string) and `instance_id` (string). `SessionAccepted` returns only `session_id` and `heartbeat_interval_secs`. There are no version fields, capability bits, or feature flags. This is the first extension to the protocol, so the extensibility story must be established now.

### Config Placement: `deny_unknown_fields` Constraint

`OtlpConfig` at `crates/openshell-server/src/config_file.rs:197` uses `#[serde(deny_unknown_fields)]`. Relay-specific configuration (buffer capacity, enrichment toggle, OCSF rate limit) cannot be added to the existing `[openshell.gateway.otlp]` section. A new config section is needed, likely `[openshell.supervisor.telemetry]` in the supervisor's own config, since the relay is a supervisor-side component.

### Environment Variable Injection: Fixed-Size Arrays

`child_env.rs` in `openshell-supervisor-process` returns fixed-size arrays:
- `proxy_env_vars()` returns `[(&str, String); 9]`
- `tls_env_vars()` returns `[(&str, &str); 6]`

Adding OTEL vars requires either bumping the array sizes (and updating all call sites) or refactoring to `Vec`. The Docker driver uses `HashMap<String, String>` and Podman uses `BTreeMap<String, String>`, so the container-side injection is straightforward. The spike should add a new `otel_env_vars()` function returning a small array rather than modifying the existing ones.

### Noisy Infrastructure Traces (#2698 Interaction)

Issue #2698 ("allow filtering gateway OTLP traces to API-initiated requests only") is `state:accepted` but unimplemented. The gateway uses `AlwaysOn` sampling, producing 54K infrastructure spans/day while idle (664:1 noise ratio). Agent spans relayed through the same pipeline will be difficult to find without collector-side filtering. The relay should add `openshell.telemetry.source` as an enrichment attribute (value: `"agent"`) so operators can filter at the collector level. Gateway infrastructure spans should correspondingly carry `openshell.telemetry.source: "gateway"` (separate from this spike, tracked by #2698).

## Codebase Entry Points (Verified 2026-08-15)

- `proto/openshell.proto:2039-2046` - `SupervisorMessage` oneof definition (add `TelemetryData` variant here)
- `crates/openshell-server/src/supervisor_session.rs:883-930` - Gateway `handle_supervisor_message()` dispatch (add `TelemetryData` handler, fix `_ =>` catch-all to not warn on unknown variants)
- `crates/openshell-server/src/supervisor_session.rs:693` - `SupervisorHello` processing (add capability negotiation)
- `crates/openshell-server/src/config_file.rs:197` - `OtlpConfig` struct with `deny_unknown_fields` (relay config goes in a separate section)
- `crates/openshell-otel/src/lib.rs:151` - `build_provider()` for gateway's `SdkTracerProvider` (relay needs a separate `SpanExporter`)
- `crates/openshell-supervisor-network/src/run.rs:287-295` - Proxy bind pattern via `proxy_bind_ip` (OTLP receiver follows same pattern)
- `crates/openshell-supervisor-process/src/child_env.rs:8,24` - `proxy_env_vars()` [9 items] and `tls_env_vars()` [6 items] fixed-size arrays (add new `otel_env_vars()`)
- `crates/openshell-driver-docker/src/lib.rs:2380` - `build_environment_for_oci_user()` HashMap-based env (add OTEL vars)
- `crates/openshell-driver-podman/src/container.rs:396` - `build_env()` BTreeMap-based env (add OTEL vars)
- `crates/openshell-core/src/net.rs:293,303` - `set_tcp_nodelay_best_effort` / `connect_tcp_nodelay_best_effort` (use on OTLP receiver)
