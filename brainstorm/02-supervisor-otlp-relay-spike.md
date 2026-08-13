# Brainstorm: Supervisor OTLP Relay Spike

**Date:** 2026-08-13
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

1. **OTLP HTTP receiver in supervisor**: Listen on port 4318, bound to `netns.host_ip()` (reachable from the sandbox's workload network namespace). Accept spans from any OTel-instrumented agent framework
2. **Automatic environment setup**: Set `OTEL_EXPORTER_OTLP_ENDPOINT` in the agent process environment, pointing at the supervisor's OTLP receiver. Zero-config for agents
3. **Span enrichment**: Add resource attributes to forwarded spans: `openshell.sandbox.id`, `openshell.workspace.id`, `openshell.sandbox.policy_hash`
4. **New session protocol message**: `PushTelemetry` message carrying serialized OTLP spans from supervisor to gateway
5. **Gateway forwarding**: Gateway receives `PushTelemetry`, deserializes spans, feeds them to its existing OTLP exporter
6. **Docker/Podman driver target**: Prototype on the local compute driver where the supervisor creates a workload netns via `setns()`

## Scope

**In scope for prototype:**
- OTLP HTTP receiver in supervisor (port 4318)
- `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable injection
- `PushTelemetry` session protocol message (protobuf definition + handler)
- Span enrichment with sandbox resource attributes
- Gateway forwarding to existing OTLP exporter
- Basic in-memory buffering (bounded, drop-oldest on overflow)
- Docker/Podman driver only

**Out of scope for prototype (future work from #2642):**
- Backpressure signaling from gateway to supervisor
- Head sampling under load
- OCSF log relay (separate data model)
- Flush-on-shutdown semantics
- Per-sandbox rate limits with drop counters
- gRPC OTLP receiver (port 4317, HTTP-only for prototype)
- Kubernetes sidecar driver support
- VM driver support

## Open Questions

- What is the existing session protocol message format? Is it protobuf, a custom binary format, or something else? The spike needs to map this before adding `PushTelemetry`
- How does the supervisor currently bind services to `netns.host_ip()`? The OTLP receiver should follow the same pattern as the proxy
- Does the gateway's OTLP exporter support injecting spans from an external source, or does it only export spans from its own tracing provider?
- Should the `PushTelemetry` message carry raw OTLP protobuf bytes (opaque to the session protocol) or a structured representation?
- What is the right buffering strategy for the prototype? A simple bounded channel with drop-oldest, or something more sophisticated?

## Codebase Entry Points to Explore

- `crates/openshell-sandbox/` - Supervisor runtime, process supervision, network namespace setup
- `crates/openshell-supervisor-network/` - Proxy, L7 enforcement, the existing service binding to netns.host_ip()
- `crates/openshell-server/` - Gateway server, session protocol handlers
- `crates/openshell-otel/` - Existing OTLP trace provider and export infrastructure
- `proto/` - Protobuf service contracts
- `crates/openshell-sdk/` - Session protocol client (how supervisor talks to gateway)
