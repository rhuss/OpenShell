# Quickstart Validation: Supervisor OTLP Relay

**Date**: 2026-08-13 | **Branch**: `001-supervisor-otlp-relay`

## Prerequisites

- OpenShell gateway running with `[openshell.gateway.otlp]` configured
- Docker or Podman compute driver available
- An OTLP collector reachable from the gateway (e.g., Jaeger, Tempo, or `otel-collector` in debug mode)
- An OTel-instrumented test agent (any language, any framework)

## Scenario 1: Agent Traces Appear in Collector (SC-001)

**Purpose**: Validate end-to-end trace relay from agent to external collector.

### Setup

1. Configure gateway with OTLP endpoint:
   ```toml
   [openshell.gateway.otlp]
   endpoint = "http://collector:4317"
   ```

2. Start an OTLP collector in debug/logging mode to verify received spans.

### Test

1. Create a sandbox: `openshell sandbox create --image <agent-image>`
2. Run the OTel-instrumented agent inside the sandbox
3. Wait for agent to complete (or at least emit spans)

### Expected Outcome

- Agent process sees `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` in its environment (no agent-side config needed)
- Collector receives spans within 30 seconds of emission
- Each span's resource attributes include `openshell.sandbox.id` and `openshell.workspace.id`
- No errors in supervisor logs related to OTLP receiver or forwarding

### Failure Indicators

- Agent OTel SDK logs connection refused (receiver not listening)
- Collector receives no spans (forwarding broken)
- Spans arrive without `openshell.sandbox.*` attributes (enrichment broken)

## Scenario 2: Correlation Between Agent and Network Spans (SC-001 + US2)

**Purpose**: Validate that agent traces and supervisor network traces share the same sandbox ID.

### Test

1. Create sandbox and run an agent that makes outbound HTTP requests
2. Query collector by `openshell.sandbox.id` for the sandbox

### Expected Outcome

- Both agent-emitted spans and supervisor network spans appear
- Outbound HTTP requests without `traceparent` have one injected by the supervisor proxy

## Scenario 3: Control Plane Isolation Under Load (SC-002)

**Purpose**: Validate telemetry does not block control operations.

### Test

1. Create sandbox running an agent that generates 100+ spans/second sustained
2. While agent runs, execute `openshell sandbox exec` commands repeatedly
3. Measure command latency

### Expected Outcome

- `sandbox exec` latency stays within 2x of baseline (measured with no telemetry load)
- Supervisor logs show no warning about blocked channels or stalled forwarding

## Scenario 4: Buffer Overflow and Drop Accounting (SC-003)

**Purpose**: Validate graceful degradation when buffer fills.

### Test

1. Configure a small buffer capacity (e.g., 64 spans) for testing
2. Run an agent that emits spans at high rate while collector is unreachable
3. Check supervisor metrics after the run

### Expected Outcome

- `spans_dropped` counter is > 0
- `queue_depth` gauge peaked at 64 (buffer capacity)
- No panic, no OOM, no stalled supervisor

## Scenario 5: Short-Lived Agent Drain (SC-004)

**Purpose**: Validate buffered spans are forwarded before teardown.

### Test

1. Run an agent that emits 10 spans and exits immediately
2. Wait for sandbox teardown to complete
3. Check collector

### Expected Outcome

- All 10 spans appear in the collector
- Supervisor logs show drain completion before teardown

## Scenario 6: OCSF Event Relay (US4)

**Purpose**: Validate OCSF deny events reach the gateway.

### Test

1. Create sandbox with a restrictive network policy (deny a known destination)
2. Agent attempts connection to the denied destination
3. Check gateway logs or log collector

### Expected Outcome

- OCSF deny event appears in gateway output within 10 seconds
- Event contains sandbox ID and denied destination details

## Scenario 7: Unsupported Driver Graceful Degradation (FR-011)

**Purpose**: Validate the feature degrades cleanly on non-Docker/Podman drivers.

### Test

1. Attempt to use the telemetry relay with the Kubernetes or VM compute driver

### Expected Outcome

- Clear log message indicating telemetry relay is not available for this driver
- No crash, no error, sandbox operates normally without telemetry relay
