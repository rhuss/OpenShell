# Data Model: Supervisor OTLP Relay

**Date**: 2026-08-13 | **Branch**: `001-supervisor-otlp-relay`

## Entities

### TelemetryRelay

The supervisor component that manages the full receive-enrich-buffer-forward pipeline.

| Field | Type | Description |
|-------|------|-------------|
| sandbox_id | String | Sandbox identifier for attribute enrichment |
| workspace_id | String | Workspace identifier for attribute enrichment |
| sandbox_metadata | SandboxMetadata | Policy, user, image, driver for enrichment |
| buffer_tx | mpsc::Sender | Bounded channel sender for buffered spans |
| buffer_rx | mpsc::Receiver | Bounded channel receiver (owned by forwarder task) |
| drop_counter | AtomicU64 | Per-sandbox count of dropped entries (trace batches + OCSF events) |
| queue_depth | AtomicUsize | Current number of buffered entries in shared buffer (gauge) |
| config | RelayConfig | Buffer size, enrichment toggle, OCSF rate limit |

**Lifecycle**: Created when sandbox starts (if OTLP endpoint configured at gateway). Drained and dropped during sandbox teardown.

### OtlpReceiver

HTTP server accepting OTLP trace data from agent processes.

| Field | Type | Description |
|-------|------|-------------|
| bind_addr | SocketAddr | Listen address (host-side veth IP or host alias, port 4318) |
| enrichment_attrs | Vec<KeyValue> | Sandbox resource attributes to inject |
| buffer_tx | mpsc::Sender | Channel to the relay buffer |

**Lifecycle**: Starts when TelemetryRelay starts. Accepts requests until shutdown signal.

### RelayConfig

Configuration for the telemetry relay. Lives in `[openshell.supervisor.telemetry]` (not in `[openshell.gateway.otlp]`, which uses `deny_unknown_fields`). The supervisor owns the relay; the gateway only needs its own OTLP endpoint config.

| Field | Type | Description |
|-------|------|-------------|
| enabled | bool | Whether telemetry relay is active |
| buffer_capacity | usize | Max slots in shared buffer for trace batches and OCSF events (default: 4096) |
| enrichment_enabled | bool | Whether to inject sandbox resource attributes (default: true) |
| ocsf_rate_limit | u32 | Max OCSF events per second per sandbox |

### SandboxMetadata

Sandbox identity used for span enrichment (FR-003).

| Field | Type | Description |
|-------|------|-------------|
| sandbox_id | String | `openshell.sandbox.id` |
| workspace_id | String | `openshell.workspace.id` |
| policy | String | `openshell.sandbox.policy` |
| user | String | `openshell.sandbox.user` |
| image | String | `openshell.sandbox.image` |
| driver | String | `openshell.sandbox.driver` |
| telemetry_source | String | `openshell.telemetry.source` (fixed: `"agent"`, distinguishes from gateway infrastructure spans per #2698). **Always injected** regardless of `enrichment_enabled` toggle since it is a relay routing marker, not sandbox metadata. |

### TelemetryData (Proto Message)

Session protocol message carrying telemetry from supervisor to gateway.

| Field | Type | Description |
|-------|------|-------------|
| sandbox_id | string | Source sandbox identifier |
| trace_data | bytes | Serialized OTLP ExportTraceServiceRequest (protobuf) |
| ocsf_events | repeated bytes | Serialized OCSF events (JSON) |

### DropMetrics

Per-sandbox telemetry accounting.

| Field | Type | Description |
|-------|------|-------------|
| spans_dropped | u64 | Total spans dropped due to buffer overflow |
| ocsf_events_dropped | u64 | Total OCSF events dropped due to rate limiting |
| spans_forwarded | u64 | Total spans successfully forwarded |
| queue_depth | usize | Current buffer occupancy (gauge) |

## Relationships

```
Gateway ──[configures]──> TelemetryRelay (via OTLP endpoint presence + capability negotiation)
TelemetryRelay ──[owns]──> OtlpReceiver
TelemetryRelay ──[owns]──> RelayConfig
TelemetryRelay ──[owns]──> DropMetrics
OtlpReceiver ──[writes to]──> Buffer (mpsc channel)
ForwarderTask ──[reads from]──> Buffer
ForwarderTask ──[sends]──> TelemetryData ──[via]──> SupervisorMessage ──[to]──> Gateway
Gateway ──[re-exports via]──> Dedicated SpanExporter ──[to]──> External Collector
OcsfLayer ──[writes to]──> Buffer (via rate limiter)
```

Note: The gateway uses a **dedicated `SpanExporter`**, not its own `SdkTracerProvider`.
The `SdkTracerProvider` would overwrite supervisor-enriched resource attributes with gateway identity.

## State Transitions

### TelemetryRelay Lifecycle

```
Disabled ──[gateway has OTLP endpoint]──> Initializing
Initializing ──[receiver bound, buffer created]──> Active
Active ──[sandbox teardown signal]──> Draining
Draining ──[buffer empty or timeout]──> Stopped
```

- **Disabled**: No OTLP endpoint configured at gateway. Relay components not created.
- **Initializing**: Binding OTLP receiver, creating buffer channel, starting forwarder task.
- **Active**: Accepting spans, enriching, buffering, forwarding. Drop semantics active when buffer full.
- **Draining**: Sandbox is shutting down. Receiver closed. Forwarder flushes remaining buffer contents.
- **Stopped**: All resources released.

### Span Lifecycle

```
Received ──[deserialized]──> Enriched ──[buffered]──> Forwarded
                                         │
                                         └──[buffer full]──> Dropped (counter incremented)
```

### OCSF Event Lifecycle

```
Emitted ──[captured by layer]──> Rate-checked ──[under limit]──> Buffered ──> Forwarded
                                      │
                                      └──[over limit]──> Dropped (counter incremented)
```

## Validation Rules

- `buffer_capacity` must be > 0 (enforced at config parse time)
- `ocsf_rate_limit` must be > 0 (enforced at config parse time)
- `sandbox_id` and `workspace_id` must be non-empty for enrichment
- OTLP receiver port must not conflict with proxy port (3128) or SSH port
- Enrichment attributes are a fixed set (6 sandbox attributes from SandboxMetadata + 1 relay marker `telemetry_source`); custom attributes are not supported
- `openshell.telemetry.source` is always injected even when `enrichment_enabled = false`
