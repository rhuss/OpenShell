# Contract: Session Protocol Telemetry Message

**Scope**: Supervisor-to-gateway telemetry transport via the existing `ConnectSupervisor` bidi stream.

## Proto Definition

New message type and `SupervisorMessage` variant in `proto/openshell.proto`:

```protobuf
// Telemetry data forwarded from supervisor to gateway.
// Contains agent trace spans and/or OCSF events from a single sandbox.
message TelemetryData {
  // Source sandbox identifier.
  string sandbox_id = 1;

  // Serialized OTLP ExportTraceServiceRequest (protobuf encoding).
  // Contains pre-enriched spans with sandbox resource attributes.
  // Empty when only OCSF events are being forwarded.
  bytes trace_data = 2;

  // Serialized OCSF events (each entry is a JSON-encoded OCSF event).
  // Empty when only trace data is being forwarded.
  repeated bytes ocsf_events = 3;
}

message SupervisorMessage {
  oneof payload {
    SupervisorHello hello = 1;
    SupervisorHeartbeat heartbeat = 2;
    RelayOpenResult relay_open_result = 3;
    RelayClose relay_close = 4;
    TelemetryData telemetry = 5;  // NEW
  }
}
```

## Capability Negotiation

The supervisor advertises `"telemetry_relay"` in `SupervisorHello.capabilities` (repeated string field). The gateway echoes confirmed capabilities in `SessionAccepted.capabilities`. The supervisor MUST NOT send `TelemetryData` unless the gateway confirmed the `"telemetry_relay"` capability.

```protobuf
message SupervisorHello {
  // ... existing fields ...
  repeated string capabilities = N;  // NEW: e.g., ["telemetry_relay"]
}

message SessionAccepted {
  // ... existing fields ...
  repeated string capabilities = N;  // NEW: confirmed capabilities
}
```

## Behavioral Contract

- The supervisor MUST only send `TelemetryData` messages after `SessionAccepted` confirms the `"telemetry_relay"` capability.
- The gateway MUST handle `TelemetryData` by extracting `trace_data` and feeding it to its OTLP exporter.
- The gateway MUST handle `ocsf_events` by dispatching each to its log sink.
- `TelemetryData` messages MUST NOT affect heartbeat timing or session liveness.
- The supervisor SHOULD batch multiple spans into a single `TelemetryData` message to reduce message overhead.

## Backward Compatibility

- Older gateways: do not echo `"telemetry_relay"` capability, so the supervisor never sends `TelemetryData`. The `_ =>` catch-all in `handle_supervisor_message()` is never reached.
- Older supervisors: do not advertise capabilities; gateway sees no telemetry (no-op).
- The `_ =>` catch-all in the gateway should be softened to `debug!` for future extensibility.
