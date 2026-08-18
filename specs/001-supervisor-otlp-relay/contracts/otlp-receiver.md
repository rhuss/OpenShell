# Contract: OTLP HTTP Receiver

**Scope**: HTTP endpoint on `127.0.0.1:4318` that accepts OTLP trace data from agent processes. For Docker/Podman drivers with a network namespace, the listener is bound inside the namespace via `bind_tcp_in_netns()`. For Kubernetes and VM drivers, the listener binds directly.

## Endpoint

```
POST /v1/traces
Host: 127.0.0.1:4318 (all topologies)
```

Port 4318 is the OTLP/HTTP standard default (hardcoded).

## Request

### Headers

| Header | Required | Values |
|--------|----------|--------|
| `Content-Type` | Yes | `application/x-protobuf` or `application/json` |

### Body

- **Protobuf**: Serialized `opentelemetry.proto.collector.trace.v1.ExportTraceServiceRequest`
- **JSON**: JSON-encoded `ExportTraceServiceRequest` per OTLP/HTTP JSON specification

## Response

### Success (200 OK)

```json
{}
```

Empty `ExportTraceServiceResponse`. Content-Type matches request.

### Errors

| Status | Condition |
|--------|-----------|
| 400 | Malformed request body (parse failure) |
| 415 | Unsupported Content-Type |
| 503 | Receiver shutting down (draining) |

Error responses include a JSON body: `{"error": "<description>"}`.

### Retry Behavior

The receiver relies on standard OTel SDK retry behavior:
- 5xx responses trigger SDK retry with exponential backoff.
- 4xx responses are not retried (client error).
- Connection refused (receiver not yet started) triggers SDK retry.

## Environment Variables

Set automatically in the agent process environment **only when the telemetry relay is active** (gateway has OTLP endpoint configured and capability negotiation succeeded). When the relay is inactive, these variables are not set to avoid agent-side connection errors.

| Variable | Value | Purpose |
|----------|-------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://127.0.0.1:4318` | Points agent SDK at supervisor receiver |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` | Ensures SDK uses HTTP transport with protobuf encoding |

For Docker, Podman, and VM drivers, the supervisor sets these via `child_env.rs`. For the Kubernetes driver, the driver sets them in the agent container spec (agent is a separate container).

## Processing Pipeline

1. Receive HTTP request
2. Deserialize `ExportTraceServiceRequest` (protobuf or JSON)
3. Inject sandbox resource attributes into each `ResourceSpans` entry
4. Re-serialize as protobuf
5. Send to bounded buffer channel
6. Return 200 OK

## Resource Attributes Injected

| Attribute | Source |
|-----------|--------|
| `openshell.sandbox.id` | Sandbox creation context |
| `openshell.workspace.id` | Sandbox creation context |
| `openshell.sandbox.policy` | Active policy name |
| `openshell.sandbox.user` | Authenticated user |
| `openshell.sandbox.image` | Container/process image |
| `openshell.sandbox.driver` | Compute driver type |
| `openshell.telemetry.source` | Fixed value `"agent"` (always injected, even when enrichment is disabled) |
