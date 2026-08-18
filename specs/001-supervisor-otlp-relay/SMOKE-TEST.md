# Guided Demo: Supervisor OTLP Relay

**Feature**: Supervisor OTLP Relay
**Date**: 2026-08-17
**Spec**: specs/001-supervisor-otlp-relay/spec.md
**Result**: Manual walkthrough (not yet executed)

## Prerequisites

You need a Linux environment with one of these sandbox drivers:

- **Docker** (recommended for simplicity)
- **Podman** with rootful mode or user namespace support

Plus:

- An OpenShell gateway binary built from this branch
- An OTLP collector (Jaeger all-in-one is the easiest)
- `curl` for sending test OTLP requests
- A simple OTel-instrumented test agent (Python example provided below)

### Build from this branch

```shell
cargo build --release -p openshell-server -p openshell-sandbox
```

The binaries land at `target/release/openshell-server` and `target/release/openshell-sandbox`.

### Start the OTLP collector (Jaeger)

```shell
docker run -d --name otlp-collector \
  -p 4317:4317 \
  -p 16686:16686 \
  jaegertracing/all-in-one:latest
```

Verify: open `http://localhost:16686` in a browser. You should see the Jaeger UI.

### Configure the gateway

Create or update `gateway.toml` with:

```toml
[openshell.gateway.otlp]
endpoint     = "http://localhost:4317"
service_name = "openshell-gateway"
```

Start the gateway:

```shell
./target/release/openshell-server --config gateway.toml
```

---

## Flow 1: Agent traces appear in collector (SC-001, FR-001, FR-002, FR-003)

**This is the core value proposition test.** An OTel-instrumented agent runs in a sandbox, and its traces show up in Jaeger with sandbox resource attributes, zero agent-side configuration.

### Setup

Create a minimal Python agent that emits OTel traces. Save as `test-agent.py`:

```python
import time
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

# OTEL_EXPORTER_OTLP_ENDPOINT is set automatically by the sandbox supervisor.
# No explicit configuration needed.
provider = TracerProvider()
processor = BatchSpanProcessor(OTLPSpanExporter())
provider.add_span_processor(processor)
trace.set_tracer_provider(provider)

tracer = trace.get_tracer("test-agent")

with tracer.start_as_current_span("agent-main") as span:
    span.set_attribute("agent.task", "smoke-test")
    with tracer.start_as_current_span("tool-call"):
        time.sleep(0.5)
    with tracer.start_as_current_span("llm-invocation"):
        time.sleep(0.3)

provider.shutdown()
print("Agent complete. Traces exported.")
```

Build a container image with this agent:

```dockerfile
FROM python:3.12-slim
RUN pip install opentelemetry-api opentelemetry-sdk opentelemetry-exporter-otlp-proto-http
COPY test-agent.py /app/test-agent.py
CMD ["python", "/app/test-agent.py"]
```

```shell
docker build -t test-otel-agent:latest -f Dockerfile.test-agent .
```

### Execute

Create a sandbox using the Docker driver:

```shell
openshell sandbox create \
  --image test-otel-agent:latest \
  --name trace-smoke-test
```

Wait for the agent to complete (it exits after ~1 second).

### Verify

1. **Check agent environment** (before the agent exits, or in logs):

   ```shell
   openshell sandbox exec trace-smoke-test -- env | grep OTEL
   ```

   **Expected**:
   ```
   OTEL_EXPORTER_OTLP_ENDPOINT=http://host.openshell.internal:4318
   OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
   ```

   If these are missing, FR-002 is not working.

2. **Check Jaeger UI** at `http://localhost:16686`:

   - Select the service dropdown. Look for spans from the test agent.
   - Click on a trace. You should see spans: `agent-main`, `tool-call`, `llm-invocation`.

3. **Check resource attributes** on any span:

   Click a span, expand "Process" or "Resource" section. Verify these attributes:

   | Attribute | Expected |
   |-----------|----------|
   | `openshell.sandbox.id` | Non-empty sandbox ID |
   | `openshell.telemetry.source` | `"agent"` (always present) |
   | `openshell.sandbox.image` | `test-otel-agent:latest` |
   | `openshell.sandbox.driver` | `container` or topology value |

   If `openshell.telemetry.source: "agent"` is missing, FR-003 enrichment is broken.

4. **Check supervisor logs** for the relay:

   Look for:
   ```
   telemetry relay started bind=10.200.0.1:4318 buffer_capacity=4096 enrichment=true
   ```

### Verdict criteria

- **PASS**: All three spans visible in Jaeger with `openshell.sandbox.id` and `openshell.telemetry.source` attributes
- **FAIL**: Spans missing, attributes missing, or agent logged connection errors

---

## Flow 2: Short-lived agent drain (SC-004, FR-008)

**Verifies that agents exiting before the OTel batch exporter flushes still get their spans forwarded.**

### Execute

The test agent from Flow 1 already exits quickly (~1 second). After the sandbox tears down:

### Verify

1. **Check span count in Jaeger**: All 3 spans (`agent-main`, `tool-call`, `llm-invocation`) should appear. If the drain didn't work, you'd see 0 spans (batch exporter hadn't flushed before process exit).

2. **Check supervisor logs** for drain completion: look for relay shutdown messages after the agent exits.

### Verdict criteria

- **PASS**: All 3 spans present in Jaeger despite the agent exiting in ~1 second
- **FAIL**: 0 spans or partial spans (drain didn't complete before teardown)

---

## Flow 3: traceparent injection on outbound HTTP (FR-009)

**Verifies the egress proxy injects W3C `traceparent` headers on outbound requests when the agent doesn't set them.**

### Execute

From inside a running sandbox with a permissive network policy:

```shell
openshell sandbox exec trace-smoke-test -- \
  curl -sS http://httpbin.org/headers
```

### Verify

The response JSON from httpbin echoes all request headers. Look for:

```json
{
  "headers": {
    "Traceparent": "00-<trace-id>-<span-id>-01",
    ...
  }
}
```

Then verify pass-through: send a request WITH an explicit traceparent:

```shell
openshell sandbox exec trace-smoke-test -- \
  curl -sS -H "traceparent: 00-aaaaaaaaaaaaaaaaaaaa-bbbbbbbb-01" \
  http://httpbin.org/headers
```

The response should show your explicit value, not a supervisor-injected one.

### Verdict criteria

- **PASS**: `traceparent` injected when absent, passed through when present
- **FAIL**: No `traceparent` in response, or existing header overwritten

---

## Flow 4: Buffer overflow and drop accounting (SC-003, FR-006, FR-012)

**Verifies graceful degradation when the buffer fills up.**

### Setup

For a quick test, temporarily set `buffer_capacity: 8` in `RelayConfig::default()` (in `crates/openshell-supervisor-network/src/otlp/mod.rs`), rebuild, and redeploy.

Create a high-rate span emitter (`test-flood.py`):

```python
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

provider = TracerProvider()
processor = BatchSpanProcessor(OTLPSpanExporter())
provider.add_span_processor(processor)
trace.set_tracer_provider(provider)
tracer = trace.get_tracer("flood-agent")

for i in range(200):
    with tracer.start_as_current_span(f"span-{i}"):
        pass

provider.shutdown()
print("Flood agent complete. 200 spans emitted.")
```

### Execute

Run the flood agent in a sandbox with the small buffer config.

### Verify

1. **Check Jaeger**: Some spans arrive (not all 200, since buffer is only 8 slots).
2. **Check supervisor logs**: Look for any drop-related output. Currently drops are tracked by `AtomicU64` counters but not logged, so you may need to add a temporary log line or check the unit test: `cargo test -p openshell-supervisor-network -- buffer_drops_when_full`.
3. **Verify no crash**: The supervisor must not OOM, panic, or stall.

### Verdict criteria

- **PASS**: Some spans arrive, supervisor stays healthy, no crash or hang
- **FAIL**: Supervisor hangs, OOM, or all spans silently lost

---

## Flow 5: Unsupported driver graceful degradation (FR-011)

**Verifies the feature degrades cleanly on non-Docker/Podman drivers.**

### Execute (if Kubernetes available)

Create a sandbox on the Kubernetes driver and check supervisor logs.

### Verify

1. **Check supervisor logs**: The telemetry relay should not start (no network namespace veth available on Kubernetes sidecar topology). Look for absence of the `telemetry relay started` log line.
2. **Verify no crash**: The sandbox should operate normally without the relay.

### Alternative (code verification)

If no Kubernetes driver is available, verify by checking that `proxy_bind_ip` is `None` when no Linux network namespace exists:

```shell
grep -A5 "proxy_bind_ip" crates/openshell-sandbox/src/lib.rs | head -10
```

The relay only starts when `proxy_bind_ip` is `Some(...)`.

### Verdict criteria

- **PASS**: Sandbox works normally without relay, no crash
- **FAIL**: Crash, panic, or error that blocks sandbox creation

---

## Flow 6: Control plane isolation under telemetry load (SC-002, FR-007)

**Verifies sandbox commands remain responsive under sustained telemetry load.**

### Setup

Create a long-running agent that emits ~100 spans/second (`test-load.py`):

```python
import time
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

provider = TracerProvider()
processor = BatchSpanProcessor(OTLPSpanExporter())
provider.add_span_processor(processor)
trace.set_tracer_provider(provider)
tracer = trace.get_tracer("load-agent")

while True:
    for i in range(100):
        with tracer.start_as_current_span(f"span-{i}"):
            pass
    time.sleep(1)
```

### Execute

1. Measure baseline latency (no telemetry load):

   ```shell
   time openshell sandbox exec <sandbox-name> -- echo hello
   ```

   Run 5 times, record average.

2. Start the load agent in a sandbox, then while it runs:

   ```shell
   time openshell sandbox exec <sandbox-name> -- echo hello
   ```

   Run 5 times, record average.

3. Compare the two averages.

### Verdict criteria

- **PASS**: Loaded latency within 2x of baseline
- **FAIL**: Latency exceeds 2x baseline or commands hang

---

## FR Coverage

| FR | Flow | Classification |
|----|------|----------------|
| FR-001 | Flow 1 | observable (OTLP HTTP receiver) |
| FR-002 | Flow 1 | observable (env var injection) |
| FR-003 | Flow 1 | observable (span enrichment) |
| FR-004 | - | internal-only (session protocol variant) |
| FR-005 | - | internal-only (dedicated SpanExporter) |
| FR-006 | Flow 4 | observable (buffer drops) |
| FR-007 | Flow 6 | observable (non-blocking control plane) |
| FR-008 | Flow 2 | observable (drain before teardown) |
| FR-009 | Flow 3 | observable (traceparent injection) |
| FR-010 | - | not yet wired (OCSF relay layer not connected) |
| FR-011 | Flow 5 | observable (unsupported driver degradation) |
| FR-012 | Flow 4 | observable (queue depth gauge) |

## Known Gaps (Spike Status)

1. **FR-010 (OCSF relay)**: `OcsfRelayLayer` exists but is not wired into the tracing subscriber. OCSF events won't flow until this integration is complete.
2. **FR-003 (partial metadata)**: `workspace_id` and `user` fields in `SandboxMetadata` are empty strings. The workspace is learned asynchronously via policy poll.
3. **FR-012 (metric exposure)**: Buffer metrics exist as atomic counters but are not exposed via a queryable endpoint or periodic log emission.
