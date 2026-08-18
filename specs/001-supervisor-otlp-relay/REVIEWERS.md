# Review Guide: Supervisor OTLP Relay

**Generated**: 2026-08-15 | **Spec**: [spec.md](spec.md)

## Why This Change

Agent developers running OTel-instrumented agents (LangChain, CrewAI, AutoGen, etc.) inside OpenShell sandboxes cannot export traces to their observability backends. Sandboxes are network-isolated by design, so the agent's OTel SDK has no path to an external collector. Without this change, developers lose visibility into agent tool calls, LLM invocations, and reasoning steps the moment their agent enters a sandbox. The only alternative today is punching egress holes for the collector, which undermines the isolation guarantees.

## What Changes

The sandbox supervisor gains a telemetry relay that receives OTLP traces from agent processes over HTTP, enriches them with sandbox identity (sandbox ID, workspace ID, policy, user, image, driver), and forwards them to the gateway over the existing session protocol. The gateway re-exports relayed spans through its configured OTLP exporter. Agent developers see their traces in their existing observability backend with zero agent-side configuration. OCSF security events share the same transport for centralized compliance reporting. There are no breaking changes; the feature is additive and gated by capability negotiation so older gateways are unaffected.

## How It Works

The relay lives in `openshell-supervisor-network` alongside the egress proxy, since it shares the network namespace bind address and lifecycle. It has four stages:

1. **Receive**: A `hyper` HTTP server accepts `POST /v1/traces` on port 4318 (OTLP/HTTP standard) inside the sandbox network namespace. Both protobuf and JSON content types are supported.
2. **Enrich**: The deserialized `ExportTraceServiceRequest` gets sandbox resource attributes injected into each `ResourceSpans` entry. The `openshell.telemetry.source: "agent"` marker is always injected (even with enrichment disabled) to enable collector-side filtering.
3. **Buffer**: A bounded `tokio::sync::mpsc` channel (4096 slots, shared between trace batches and OCSF events) absorbs bursts. When full, the oldest entries are dropped with counter accounting.
4. **Forward**: A forwarder task drains the buffer, batches entries into `TelemetryData` proto messages, and sends them via `try_send` on the session channel (non-blocking, never stalls control operations).

On the gateway side, a dedicated `SpanExporter` (not the gateway's `SdkTracerProvider`) re-exports relayed spans to preserve the supervisor-enriched resource attributes. Capability negotiation via `SupervisorHello`/`SessionAccepted` ensures the supervisor only sends telemetry if the gateway confirms support.

Environment variables (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`) are injected into the agent process only when the relay is active, so agents on gateways without OTLP configuration never see connection errors.

## When It Applies

**Applies when**:
- Gateway has `[openshell.gateway.otlp]` endpoint configured
- Sandbox uses Docker or Podman compute driver
- Agent process is instrumented with any OTel SDK
- Capability negotiation confirms gateway support

**Does not apply when**:
- No OTLP endpoint configured at the gateway (relay not created, env vars not set)
- Kubernetes sidecar or VM compute drivers (graceful degradation with warning log, no crash)
- Agent overrides `OTEL_EXPORTER_OTLP_ENDPOINT` to a custom destination (agent's choice honored)
- gRPC OTLP (port 4317) is the only available protocol (HTTP-only for initial delivery)

## Key Decisions

1. **Session protocol transport over separate gRPC stream**: The existing `ConnectSupervisor` bidi stream already carries mixed traffic (hello, heartbeat, relay control). A separate stream would add connection management, TLS overhead, and a second auth path for marginal isolation. Capability negotiation prevents log spam from older gateways that don't understand the new message variant.

2. **Dedicated SpanExporter on gateway, not SdkTracerProvider**: The gateway's `SdkTracerProvider` merges its own resource (service.name = "openshell-gateway") into all spans, which would overwrite the supervisor-enriched sandbox attributes. A dedicated exporter preserves relayed spans exactly as enriched.

3. **Proto-level enrichment in the supervisor, not the gateway**: The supervisor owns sandbox identity. The gateway would need context it currently lacks. Working at the proto message level allows attribute injection regardless of input format (protobuf or JSON).

4. **Bounded mpsc channel (4096 slots) with ring-buffer drops**: Matches existing supervisor patterns (denial/activity aggregation). The `try_send` forwarder pattern ensures telemetry never blocks control operations. Drop counters and queue depth gauge give operators visibility into buffer pressure.

5. **Conditional env var injection**: `OTEL_EXPORTER_OTLP_ENDPOINT` is only set when the relay is active. Setting it unconditionally would cause connection errors in every OTel-instrumented agent on gateways without OTLP configuration.

6. **`openshell.telemetry.source` always injected**: This relay routing marker is always set to `"agent"` regardless of the enrichment toggle. Without it, collectors lose the ability to distinguish agent-relayed spans from gateway infrastructure spans, breaking filtering pipelines.

## Areas Needing Attention

- **Shared file modifications**: `crates/openshell-sandbox/src/lib.rs` is modified by 4 tasks across different phases (T017, T020, T023, T024). Later phases should review prior modifications for conflicts.
- **Capability negotiation backward compatibility**: The `_ =>` catch-all in `handle_supervisor_message()` currently logs a warning per unknown variant. The negotiation prevents this, but the catch-all should also be softened to `debug!` for future extensibility.
- **Buffer capacity semantics**: The 4096-slot buffer is shared between trace batches and OCSF events. Under extreme OCSF event rates, even with rate limiting, OCSF events could displace trace batches. The rate limiter defaults (100 events/sec) should make this unlikely in practice.
- **Enrichment toggle granularity**: The enrichment toggle is all-or-nothing for sandbox metadata attributes. Per-attribute control was explicitly not implemented (spec clarification). If a use case requires partial enrichment, the toggle granularity would need revisiting.

## Open Questions

No open questions identified. All ambiguities were resolved during the clarification sessions on 2026-08-13 and 2026-08-15.

## Review Checklist

- [ ] Key decisions are justified
- [ ] Capability negotiation prevents telemetry to older gateways
- [ ] Buffer drop semantics include counter accounting (no silent loss)
- [ ] Env vars are conditional on relay being active
- [ ] `openshell.telemetry.source` is always injected
- [ ] Shared buffer semantics (traces + OCSF) are correct
- [ ] Scope matches the stated boundaries (Docker/Podman only)
- [ ] Success criteria are achievable and measurable
- [ ] No unstated assumptions

---

<!-- Code phase sections are appended below this line by the phase-manager command -->
