# Research: Supervisor OTLP Relay

**Date**: 2026-08-13 | **Branch**: `001-supervisor-otlp-relay`

## Session Protocol Transport

**Decision**: Add a `TelemetryData` variant to the existing `SupervisorMessage.oneof payload` in `proto/openshell.proto`, gated by capability negotiation.

**Rationale**: The bidi-streaming `ConnectSupervisor` RPC already provides a multiplexed, ordered channel between supervisor and gateway. Adding a new `oneof` variant is the established extension pattern (existing variants: `SupervisorHello`, `SupervisorHeartbeat`, `RelayOpenResult`, `RelayClose`).

**Critical finding (2026-08-15)**: The gateway's `handle_supervisor_message()` at `supervisor_session.rs:883-930` has a `_ =>` catch-all that **logs a warning per unknown message variant**. This means an older gateway would spam warnings on every telemetry batch from a newer supervisor. Protobuf's unknown-field behavior applies at the wire level, but the generated Rust code deserializes the new variant into `Some(Payload::TelemetryData(...))`, which reaches the application-level match. **Mitigation**: Add a `capabilities` repeated-string (or bitmask) field to `SupervisorHello` and `SessionAccepted`. The supervisor advertises `"telemetry_relay"` and only sends `TelemetryData` if the gateway echoes it back. Additionally, the `_ =>` catch-all should be softened to `debug!` for future extensibility.

**Alternatives considered**:
- Separate gRPC stream for telemetry: Rejected. Adds connection management complexity, TLS overhead, and a second authentication path for marginal isolation benefit. The existing session stream already carries mixed traffic (hello, heartbeat, relay control).
- Direct OTLP export from supervisor to external collector: Rejected. Requires the supervisor to have network access to the collector, violating the sandbox isolation boundary. The gateway is the designated egress point.
- No capability negotiation (rely on proto unknown fields): Rejected. Application-level catch-all in the gateway logs warnings. Rolling upgrades would produce log spam until both sides are updated.

## OTLP HTTP Receiver Implementation

**Decision**: Use `hyper` (already a dependency) to build a minimal HTTP server that accepts `POST /v1/traces` with both `application/x-protobuf` and `application/json` content types.

**Rationale**: The supervisor already depends on `hyper` via `tonic`. A lightweight HTTP server avoids pulling in the full `opentelemetry-collector` crate ecosystem. The receiver needs only one endpoint (`/v1/traces`) with two content type decoders.

**Alternatives considered**:
- `opentelemetry-proto` crate for decoding: Currently not a workspace dependency. The proto types can be generated from the OpenTelemetry proto definitions or hand-rolled for the subset needed (ExportTraceServiceRequest). The workspace already uses `prost`/`tonic` for proto codegen. Decision: add `opentelemetry-proto` as a dependency for correct, maintained proto types.
- `actix-web` or `axum` HTTP server: Rejected. Unnecessary framework overhead for a single endpoint.

## Span Enrichment Strategy

**Decision**: Enrich at the protobuf level by inserting resource attributes into the deserialized `ExportTraceServiceRequest` before re-serializing for transport.

**Rationale**: Working at the proto message level allows attribute injection regardless of whether the input was protobuf or JSON. The supervisor has all needed context (sandbox ID, workspace ID, policy, user, image, driver) available at the point of enrichment. The `openshell.telemetry.source: "agent"` attribute is always injected regardless of the enrichment toggle, since it is a relay routing marker (not sandbox metadata) required for collector-side filtering.

**Alternatives considered**:
- Enrich at the gateway: Rejected. The gateway would need sandbox context it currently lacks. The supervisor owns the sandbox identity.
- Enrich via OTel SDK resource: Rejected. The supervisor is not an OTel SDK participant for agent spans; it's a relay.

## Buffer Architecture

**Decision**: Bounded `tokio::sync::mpsc` channel with capacity 4096 slots (shared between trace batches and OCSF events), ring-buffer drop semantics.

**Rationale**: The supervisor already uses `mpsc` channels for denial and activity aggregation. A bounded channel provides natural backpressure. When the channel is full, the oldest entries are dropped (via a custom wrapper that tracks drops). A separate forwarding task drains the channel and batches entries into `TelemetryData` session messages. OCSF events share the same buffer since they are already rate-limited upstream; a single shared buffer simplifies implementation and provides one tuning knob for operators.

**Alternatives considered**:
- `VecDeque` with manual locking: Rejected. More complex than an mpsc channel for the same semantics.
- Unbounded channel with size checks: Rejected. No backpressure guarantee; relies on periodic polling to enforce limits.

## Environment Variable Injection

**Decision**: Always `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318` and `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf`. The supervisor sets these via `child_env.rs` for agent child processes (Docker, Podman, VM). The Kubernetes driver sets them in the agent container spec (agent is a separate container, not a supervisor child). Only injected when the relay is active.

**Rationale (evolved 2026-08-17)**: The receiver always binds to `127.0.0.1:4318`, regardless of driver. This uniform address simplifies env var injection to a single constant across all topologies. The previous design used two injection points with different addresses (`10.200.0.1` for native process, `host.openshell.internal` for containers), which was incorrect: the receiver binds inside the sandbox network namespace, not on the Docker host.

**Implementation note**: `child_env.rs` provides `otel_env_vars(endpoint, protocol) -> [(&str, String); 2]`, called from `process.rs` and `ssh.rs` alongside existing `proxy_env_vars()` and `tls_env_vars()`. Docker and Podman drivers should NOT set OTEL env vars (their container env is for the supervisor, not the agent). The K8s driver is the exception because the agent runs in a separate container.

**Alternatives considered**:
- Driver-level injection for Docker/Podman: Rejected. Those env vars targeted the supervisor container, not the agent process. The supervisor handles env injection via child_env.
- Different addresses per topology: Rejected. Always localhost is simpler and correct for all topologies.
- Modify existing `proxy_env_vars()` array: Rejected. A separate `otel_env_vars()` function keeps concerns separated.

## Traceparent Injection

**Decision**: Add W3C `traceparent` header injection in the CONNECT proxy's forward-HTTP path using the existing `openshell-otel` propagation utilities (`TraceContextInterceptor`, `MetadataMapInjector`).

**Rationale**: The proxy in `openshell-supervisor-network` already intercepts forward HTTP requests for policy evaluation. Adding inject-if-missing `traceparent` headers at the same point requires minimal new code. The `openshell-otel` crate already provides the propagation primitives.

**Alternatives considered**:
- Inject on CONNECT tunnels: Not feasible. CONNECT creates an opaque TCP tunnel; the proxy cannot inject HTTP headers into the tunneled stream without full L7 interception.
- Agent-side propagation only: Rejected. Many agent frameworks do not propagate trace context on outbound HTTP by default.

## OCSF Event Relay

**Decision**: OCSF events share the `TelemetryData` session message with a separate payload field (logs vs. traces). Per-sandbox rate limiting uses a token bucket in the supervisor.

**Rationale**: OCSF events are already emitted via `tracing` layers. A new `tracing::Layer` subscriber can capture OCSF events and feed them into the same telemetry buffer/channel as spans, with a rate limiter gate. The `TelemetryData` proto message carries both traces and logs in separate fields.

**Alternatives considered**:
- Separate session message for OCSF: Rejected. Shares the same transport semantics (buffered, drainable, non-blocking). A single message type is simpler.
- Relay OCSF as OTLP logs: Considered but deferred. OCSF events have their own schema; converting to OTLP log records would lose OCSF structure. Instead, forward as opaque JSON bytes and let the gateway dispatch to the appropriate log sink.

## Gateway-Side Span Re-export

**Decision**: The gateway extracts `trace_data` bytes from `TelemetryData` messages and forwards them to the configured OTLP collector using a dedicated `SpanExporter`, separate from the gateway's own `SdkTracerProvider`.

**Rationale**: The gateway's `SdkTracerProvider` (built in `crates/openshell-otel/src/lib.rs:151` via `build_provider()`) is designed to create new spans, not relay pre-built ones. Feeding relayed spans through the gateway's provider would overwrite the supervisor-enriched resource attributes (sandbox ID, workspace ID, telemetry source) with the gateway's own resource (service.name = "openshell-gateway"). A dedicated exporter preserves the relayed spans exactly as the supervisor enriched them. The exporter reuses the same OTLP endpoint configuration (`[openshell.gateway.otlp]`) so no additional configuration surface is needed.

**Config placement note (2026-08-15)**: `OtlpConfig` at `crates/openshell-server/src/config_file.rs:197` uses `#[serde(deny_unknown_fields)]`. Relay-specific settings (buffer capacity, enrichment toggle, OCSF rate limit) cannot live in `[openshell.gateway.otlp]`. They belong in a new section, likely `[openshell.supervisor.telemetry]`, since the relay is a supervisor-side component. The gateway reads this section only to know whether to expect telemetry from supervisors.

**Alternatives considered**:
- Feed through existing `SdkTracerProvider`: Rejected. TracerProviders merge their configured Resource into all spans, corrupting the supervisor-enriched attributes.
- Forward raw proto bytes directly to collector HTTP endpoint: Simpler but loses batching, retry, and connection management. The `SpanExporter` API provides these.
- Separate OTLP endpoint configuration: Adds configuration surface area without clear benefit. The external collector receives all spans at one endpoint.
