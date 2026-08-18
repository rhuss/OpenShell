# Feature Specification: Supervisor OTLP Relay

**Feature Branch**: `6115-supervisor-otlp-relay`

**Created**: 2026-08-13

**Status**: Draft

**Input**: Brainstorm #02 (supervisor-otlp-relay-spike), upstream issues #2641, #2642

## User Scenarios & Testing

### User Story 1 - Agent developer sees traces from a sandboxed agent (Priority: P1)

An agent developer runs an OTel-instrumented agent (LangChain, CrewAI, or any framework using the OpenTelemetry SDK) inside an OpenShell sandbox. The sandbox is network-isolated, so the agent cannot export traces directly to an external collector. Without any OpenShell-specific code or configuration, the developer opens their observability backend (MLflow, Langfuse, Jaeger) and sees the agent's tool calls, LLM invocations, and reasoning steps annotated with sandbox context.

**Why this priority**: This is the core value proposition. If agent developers cannot see their traces from inside sandboxes, the entire feature has no purpose. Every other story builds on this one.

**Independent Test**: Deploy an OTel-instrumented agent in any supported sandbox driver, run it, then verify traces appear in a configured OTLP collector with sandbox resource attributes attached.

**Acceptance Scenarios**:

1. **Given** a sandbox running on any supported driver with an OTLP endpoint configured at the gateway, **When** an OTel-instrumented agent runs inside the sandbox, **Then** the agent's spans appear in the configured collector within 30 seconds of emission, enriched with `openshell.sandbox.id` and `openshell.workspace.id` resource attributes.
2. **Given** a sandbox with no agent-side OTLP configuration, **When** the agent process starts, **Then** `OTEL_EXPORTER_OTLP_ENDPOINT` is set in the agent's environment pointing at the supervisor's local OTLP receiver. The agent's OTel SDK exports without any explicit setup.
3. **Given** a short-lived agent that exits after 5 seconds, **When** the agent process terminates before flushing its OTel batch exporter, **Then** the supervisor drains any buffered spans and forwards them before sandbox teardown completes.

---

### User Story 2 - Platform operator correlates agent behavior with network activity (Priority: P2)

A platform operator sees elevated 429 (rate limit) responses on outbound connections from a sandbox in the supervisor's network spans. The operator needs to understand what the agent is doing that triggers this volume. By querying their trace backend for `openshell.sandbox.id`, the operator finds both agent traces and supervisor network traces together.

**Why this priority**: Correlation between agent behavior and infrastructure activity is what makes the relay operationally useful beyond basic trace forwarding. It distinguishes this from simply punching an egress hole for the collector.

**Independent Test**: Run an agent that makes repeated API calls, then query the trace backend by sandbox ID and verify both agent spans and supervisor network spans appear with the same `openshell.sandbox.id` attribute.

**Acceptance Scenarios**:

1. **Given** a sandbox with both supervisor infrastructure tracing and an instrumented agent, **When** the operator queries the trace backend by `openshell.sandbox.id`, **Then** both agent-emitted spans and supervisor-emitted network spans appear in the results.
2. **Given** an agent making outbound HTTP requests, **When** the request has no `traceparent` header, **Then** the supervisor's egress proxy injects a `traceparent` from its own span context so the request is visible in the supervisor's trace.

---

### User Story 3 - Telemetry does not interfere with sandbox control operations (Priority: P2)

Under high telemetry load (100 spans/second from the agent, plus supervisor infrastructure spans), sandbox lifecycle operations (create, exec, stop) complete at normal latency. Telemetry forwarding degrades gracefully (drops spans with accounting) rather than blocking control messages.

**Why this priority**: If telemetry can stall the control plane, the feature is a liability rather than a benefit. This is a safety requirement for production deployments.

**Independent Test**: Run a sustained 100 spans/second workload in a sandbox while simultaneously executing sandbox lifecycle commands. Measure command latency and verify it stays within normal bounds.

**Acceptance Scenarios**:

1. **Given** a sandbox generating 100 spans/second sustained, **When** a user runs `sandbox exec` commands, **Then** command latency remains within 2x of the baseline latency measured with no telemetry load.
2. **Given** a supervisor buffer that reaches capacity, **When** new spans arrive, **Then** the oldest buffered spans are dropped and a counter metric increments to record the drops. No spans are silently lost.
3. **Given** the gateway becomes temporarily unreachable, **When** the supervisor cannot forward spans, **Then** the supervisor buffers up to the configured limit, then drops with accounting. Control messages are not affected.

---

### User Story 4 - Supervisor relays OCSF events to the gateway (Priority: P3)

A security reviewer needs all OCSF deny events across sandboxes for a compliance report. OCSF events generated inside each sandbox are forwarded to the gateway alongside agent traces, so the reviewer queries their centralized log aggregator rather than SSHing into each sandbox.

**Why this priority**: OCSF log relay shares the same transport infrastructure as trace relay but serves a different audience (compliance vs. debugging). It can be delivered after trace relay is stable.

**Independent Test**: Trigger a network deny event inside a sandbox, then verify the corresponding OCSF event appears in the gateway's log output or configured log collector.

**Acceptance Scenarios**:

1. **Given** a sandbox with network policy that denies certain destinations, **When** the agent attempts a connection to a denied destination, **Then** the OCSF deny event is forwarded to the gateway within 10 seconds.
2. **Given** an OCSF event rate exceeding the configured per-sandbox limit, **When** the rate limiter kicks in, **Then** excess events are dropped with a counter metric and a local warning log. No events are silently lost.

---

### Edge Cases

- What happens when the supervisor's OTLP receiver is not reachable from the agent process? The agent's OTel SDK should retry with backoff per standard OTel retry behavior. The agent should not crash or hang.
- What happens when the agent overrides `OTEL_EXPORTER_OTLP_ENDPOINT` to point somewhere else? The agent's choice is honored; the supervisor does not intercept or redirect OTLP traffic. Only spans sent to the supervisor's receiver are relayed.
- What happens when two agents in the same sandbox export traces simultaneously? The supervisor's OTLP receiver handles concurrent requests. Spans from both agents are enriched and forwarded independently.
- What happens when the configured external OTLP collector is permanently unreachable? The supervisor drops buffered spans with accounting after the buffer fills. The gateway does not accumulate unbounded memory.
- What happens on non-Linux platforms? The relay skips initialization with a clear log message. No crash, no silent failure.

## Requirements

### Functional Requirements

- **FR-001**: The supervisor MUST accept OTLP trace data over HTTP on `127.0.0.1:4318` (localhost). For Docker/Podman drivers where the supervisor creates a network namespace, the receiver MUST bind inside the namespace via `bind_tcp_in_netns()`. For Kubernetes and VM drivers where supervisor and agent share the same network namespace, the receiver binds directly. The receiver MUST accept both `application/x-protobuf` and `application/json` content types. The receiver MUST set `TCP_NODELAY` on accepted connections (via `openshell_core::net::set_tcp_nodelay_best_effort`) to avoid delayed-ACK stalls on small OTLP batches. gRPC OTLP support is out of scope for the initial delivery.
- **FR-002**: The supervisor MUST set `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318` and `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` in the agent process environment, only when the telemetry relay is active (i.e., the gateway confirmed `telemetry_relay` capability via FR-004a). For Docker, Podman, and VM drivers, the supervisor sets these via `child_env.rs` (agent is a supervisor child process). For the Kubernetes driver, the driver MUST set these in the agent container's environment (agent runs in a separate container, not as a supervisor child). When the relay is inactive, the environment variables MUST NOT be set. No agent-side configuration is required when the relay is active.
- **FR-003**: The supervisor MUST enrich forwarded spans with sandbox resource attributes: `openshell.sandbox.id`, `openshell.workspace.id`, `openshell.sandbox.policy`, `openshell.sandbox.user`, `openshell.sandbox.image`, `openshell.sandbox.driver`, and `openshell.telemetry.source` (fixed value: `"agent"`). The `openshell.telemetry.source` attribute distinguishes agent-relayed spans from gateway infrastructure spans (see #2698), enabling collector-side filtering. Enrichment of sandbox metadata attributes MUST be configurable and support being disabled for pass-through forwarding. The `openshell.telemetry.source` attribute MUST always be injected regardless of the enrichment toggle, since it is a relay routing marker required for collector-side filtering.
- **FR-004**: The supervisor MUST forward enriched spans to the gateway using a `TelemetryData` variant in the `SupervisorMessage.oneof payload`, separate from control messages.
- **FR-004a**: The session protocol MUST support capability negotiation. `SupervisorHello` MUST include a `capabilities` field advertising supported features (e.g., telemetry relay). `SessionAccepted` MUST include a corresponding `capabilities` field confirming which features the gateway supports. The supervisor MUST NOT send `TelemetryData` messages unless the gateway acknowledged the telemetry capability.
- **FR-005**: The gateway MUST receive forwarded spans and export them through a **dedicated `SpanExporter`** to the configured external collector. The gateway MUST NOT feed relayed spans through its own `SdkTracerProvider`, as that would overwrite supervisor-enriched resource attributes with the gateway's identity.
- **FR-006**: The supervisor MUST buffer telemetry data (trace batches and OCSF events) in a single shared bounded in-memory buffer with a default capacity of 4096 slots (configurable). When the buffer reaches capacity, the oldest entries MUST be dropped and a counter metric MUST record each drop.
- **FR-007**: Telemetry forwarding MUST NOT block or degrade sandbox control operations (create, exec, stop, connect) under sustained telemetry load.
- **FR-008**: The supervisor MUST drain buffered spans before sandbox teardown completes, so short-lived agents do not lose their final spans.
- **FR-009**: The supervisor's egress proxy MUST propagate W3C `traceparent` context on forward-proxy HTTP requests (non-CONNECT) using inject-if-missing behavior: pass through existing headers, inject from the supervisor's span context when absent. Note: CONNECT tunnels create opaque TCP streams where HTTP header injection is not feasible; this requirement applies only to the forward-HTTP code path, which is a subset of agent traffic.
- **FR-010**: The supervisor MUST forward OCSF events to the gateway using the same telemetry transport, with per-sandbox rate limiting and drop counters.
- **FR-011**: The feature MUST support all four compute drivers: Docker, Podman, Kubernetes (combined and sidecar topologies), and VM (libkrun/QEMU). For Docker/Podman, the receiver binds inside the supervisor-created network namespace via `bind_tcp_in_netns()`. For Kubernetes combined, the receiver binds directly on pod-shared localhost. For Kubernetes sidecar, the relay runs in the network sidecar container (which owns the gateway session), gated on `network_enabled`. The process supervisor in the agent container does not run a relay. For VM, the receiver binds on guest localhost. On non-Linux platforms, the relay MUST skip initialization with a clear log message (no crash, no silent failure).
- **FR-012**: The supervisor MUST expose a per-sandbox queue depth gauge metric alongside drop counters, so operators can monitor buffer pressure without waiting for drops to occur. Metrics MUST be emitted as structured log fields on relay shutdown and available programmatically via the `BufferMetrics` API for future Prometheus or OCSF integration.

### Key Entities

- **Telemetry Relay**: The supervisor component that receives, buffers, enriches, and forwards agent traces and OCSF events.
- **OTLP Receiver**: The HTTP endpoint at `127.0.0.1:4318` reachable from agent processes (bound inside the network namespace for Docker/Podman, or on shared localhost for Kubernetes/VM).
- **Telemetry Message**: The session protocol message type that carries serialized trace and log data from supervisor to gateway.
- **Span Enrichment**: The process of attaching sandbox resource attributes to agent-emitted spans before forwarding.
- **Drop Counter**: A per-sandbox metric that records how many spans or events were dropped due to buffer overflow or rate limiting.

## Success Criteria

### Measurable Outcomes

- **SC-001**: An OTel-instrumented agent running in any supported sandbox driver (Docker, Podman, Kubernetes, VM) sees its traces in the configured collector within 30 seconds of emission, with zero agent-side OpenShell configuration.
- **SC-002**: Sandbox lifecycle command latency stays within 2x of baseline when a sandbox generates 100 spans/second sustained telemetry.
- **SC-003**: No telemetry data is silently lost. Every dropped span or event is accounted for by a counter metric queryable by operators.
- **SC-004**: Short-lived agents (under 10 seconds) have their buffered spans forwarded before sandbox teardown, with zero span loss for agents that emit fewer than the buffer capacity.
- **SC-005**: The feature works with any OTel-instrumented agent framework (LangChain, CrewAI, AutoGen, custom) without framework-specific code in the supervisor.

## Out of Scope

- gRPC OTLP receiver (port 4317). HTTP-only for initial delivery.
- Per-workspace OTLP endpoint routing (requires a workspace settings tier that does not exist today).
- Head or tail sampling at the gateway (collector-side concern).
- Agent stdout/stderr relay (interactive session output, not structured telemetry).
- Span content sanitization or attribute filtering (collector-side concern).
- Backpressure signaling from gateway to supervisor (future production hardening).

## Clarifications

### Session 2026-08-13

- Q: Should the OTLP HTTP receiver accept protobuf, JSON, or both content types? → A: Both (`application/x-protobuf` and `application/json`).
- Q: What should the default buffer capacity be (in spans)? → A: 4096 spans.
- Q: Is span content sanitization in scope for the relay? → A: Out of scope; relay forwards as-is, sanitization is the collector's responsibility.
- Q: Should the relay expose operational metrics beyond drop counters? → A: Yes, add a current queue depth gauge metric alongside drop counters.
- Q: How should an older gateway handle unknown telemetry messages from a newer supervisor? → A: ~~Silently ignore (protobuf default behavior for unknown fields).~~ **Revised**: The gateway's `handle_supervisor_message()` catch-all logs a warning, not silent discard. Capability negotiation (FR-004a) prevents the mismatch entirely.

### Session 2026-08-15

- Q: Should `OTEL_EXPORTER_OTLP_ENDPOINT` be set unconditionally or only when the relay is active? → A: Only when the relay is active (conditional on gateway OTLP config). Setting the env var when no receiver is listening causes unnecessary error noise from OTel-instrumented agents.
- Q: When enrichment is disabled (pass-through mode), should `openshell.telemetry.source: "agent"` still be injected? → A: Yes, always inject `telemetry.source` regardless of the enrichment toggle. It is a relay routing marker, not sandbox metadata, and collectors depend on it to filter agent spans from infrastructure spans.
- Q: Does the buffer capacity count trace data and OCSF events together, or separately? → A: Shared buffer (4096 slots for both traces and OCSF events combined). OCSF events are already rate-limited (FR-010), so a single buffer simplifies implementation and provides one tuning knob.
- Q: Where should relay configuration live in the TOML? → A: Not in `[openshell.gateway.otlp]` (uses `deny_unknown_fields`). A new section is needed, likely `[openshell.supervisor.telemetry]`, since the relay is a supervisor-side component.
- Q: How should OTEL env vars be injected given `child_env.rs` uses fixed-size arrays? → A: Add a new `otel_env_vars(endpoint, protocol)` function returning a small array, rather than modifying the existing `proxy_env_vars()` or `tls_env_vars()` functions.
- Q: How do agent spans interact with the existing noisy infrastructure traces (#2698)? → A: Add `openshell.telemetry.source: "agent"` as an enrichment attribute so collectors can filter. Gateway-side infrastructure span tagging is tracked separately by #2698.

### Session 2026-08-17 (Spec Evolution: Cross-Driver Support)

Evolved from brainstorm 03-cross-driver-otlp-relay.md. The initial Docker/Podman-only scope is expanded to all four compute drivers.

- Q: Should the OTLP receiver bind to the veth host IP (10.200.0.1) or localhost? -> A: Always `127.0.0.1:4318`. For netns topologies (Docker/Podman), use `bind_tcp_in_netns()` to bind inside the namespace. For shared-namespace topologies (K8s, VM), bind directly. The agent always reaches the receiver at localhost.
- Q: Should OTEL env vars be set by the driver or the supervisor? -> A: By the supervisor via `child_env.rs`, except for Kubernetes where the agent is a separate container and the driver must set them in the container spec.
- Q: Should K8s sidecar and VM drivers be in scope? -> A: Yes. The architecture naturally supports all topologies. The receiver binds to localhost everywhere; only the binding mechanism differs (netns vs direct).
- Q: Should the receiver accept a pre-bound listener or always bind itself? -> A: Accept either. When a netns exists, the caller provides a `TcpListener` from `bind_tcp_in_netns()`. Otherwise, the receiver binds a `SocketAddr` directly.

### Session 2026-08-17 (Clarification: Post-Evolution)

- Q: What triggers relay activation (opt-in vs always-on)? -> A: Opt-in. The relay starts only when the gateway has `[openshell.gateway.otlp]` configured. The supervisor learns this via capability negotiation (FR-004a). When the gateway lacks OTLP config, the relay is not started and no resources are consumed.
- Q: In K8s sidecar topology, which container runs the relay? -> A: The network sidecar (`--mode=network`). It owns the gateway session channel needed to forward `TelemetryData`. The relay starts when `network_enabled` is true. The process supervisor in the agent container only sets OTEL env vars for agent child processes. The receiver at `127.0.0.1:4318` is reachable from the agent container via shared pod network.
- Q: Should port 4318 be configurable or hardcoded? -> A: Hardcoded. 4318 is the standard OTLP/HTTP port. No operator demand for configurability. Can be made configurable later if needed.

## Assumptions

- The gateway's OTLP exporter is configured and functional (the `[openshell.gateway.otlp]` configuration already exists).
- The session protocol between supervisor and gateway can carry additional message types. However, the current gateway `handle_supervisor_message()` catch-all logs a warning per unknown variant (not silent discard). Capability negotiation (FR-004a) is required to prevent log spam during rolling upgrades.
- The OTLP receiver always binds to `127.0.0.1:4318`. For Docker/Podman drivers with a network namespace, the bind happens inside the namespace via `bind_tcp_in_netns()`. For Kubernetes and VM drivers, the supervisor and agent share the same network namespace, so a direct bind suffices.
- Agent frameworks use the standard `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable for OTLP export configuration.
- The bounded buffer defaults to 4096 slots (configurable), shared between trace batches and OCSF events, balancing memory usage (~20 MB worst case) and ~40 seconds of retention at 100 spans/second.
- OCSF event relay shares the transport infrastructure with trace relay but has its own rate limiting configuration.
- Older gateways that do not understand the new telemetry message type will hit the catch-all `_ =>` arm in `handle_supervisor_message()`, which currently logs a warning. The capability negotiation in FR-004a prevents this scenario: the supervisor only sends telemetry if the gateway confirmed support. The catch-all should also be softened to `debug!` or silenced for future extensibility.
- For the Kubernetes driver, the agent runs in a separate container (not a supervisor child process), so the K8s driver must set OTEL env vars in the agent container's environment. This is the one case where the driver, not the supervisor, sets the OTEL env vars.
