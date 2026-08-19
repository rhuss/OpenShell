<!-- SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# OTLP Relay Spike Demo Plan

## Narrative

Show that agent traces flow transparently from inside a sandboxed agent to
MLflow, without any agent-side OTLP configuration, through the supervisor's
built-in OTLP relay. The relay enriches spans with sandbox identity and routes
them via the gateway to an OTel Collector that splits agent traces (MLflow)
from infrastructure traces (Tempo/Jaeger).

## Architecture Diagram (update saw-trace-architecture.excalidraw)

```
  Sandbox (VM, Docker container, network namespace)
  ┌──────────────────────────────────────────┐
  │                                          │
  │  Agent process (any OTel-instrumented)   │
  │    OTel SDK -> OTEL_EXPORTER_OTLP_*     │
  │    (env vars set by supervisor,          │
  │     zero agent config needed)            │
  │         │                                │
  │         ▼ POST /v1/traces                │
  │  ┌────────────────────┐                  │
  │  │ Supervisor OTLP    │ 127.0.0.1:4318   │
  │  │ Receiver (netns)   │                  │
  │  │   + enrichment:    │                  │
  │  │     sandbox.id     │                  │
  │  │     telemetry.src  │                  │
  │  │   + buffer (4096)  │                  │
  │  └────────┬───────────┘                  │
  │           │ TelemetryData                │
  │           │ (session protocol,           │
  │           │  try_send non-blocking)      │
  └───────────┼──────────────────────────────┘
              │ gRPC ConnectSupervisor
              ▼
  ┌──────────────────────┐
  │   OpenShell Gateway  │
  │   TelemetryRelay-    │
  │   Exporter           │──────────┐
  └──────────────────────┘          │ gRPC :4317
                                    ▼
                    ┌──────────────────────────┐
                    │     OTel Collector       │
                    │   (Red Hat OpenTelemetry │
                    │    Operator)             │
                    │                          │
                    │  filter processors:      │
                    │  telemetry.source ==     │
                    │    "agent" -> MLflow     │
                    │  no source -> Tempo      │
                    └──────┬──────────┬────────┘
                           │          │
                           ▼          ▼
                    ┌──────────┐ ┌──────────┐
                    │  MLflow  │ │  Tempo   │
                    │  (RHOAI  │ │ (Red Hat │
                    │  3.4 op) │ │  Tempo)  │
                    │  agent   │ │  infra   │
                    │  traces  │ │  traces  │
                    └──────────┘ └──────────┘
```

The relay is built into the supervisor. The agent just exports to
localhost:4318 (set automatically via OTEL_EXPORTER_OTLP_ENDPOINT env var).

## Demo Harness

Python demo agent using OpenAI SDK + `opentelemetry-instrumentation-openai`
auto-instrumentation. Every `chat.completions.create()` call generates OTel
spans with model name, token counts, prompt, and response.

**Script**: `demo-agent.py` deployed at `/sandbox/demo-agent.py` inside the
sandbox.

**Installed packages** (via pip, from container netns which has Docker network):
- `openai`
- `opentelemetry-api`, `opentelemetry-sdk`
- `opentelemetry-exporter-otlp-proto-http`
- `opentelemetry-instrumentation-openai`

## Demo Scenes

### Scene 1: Architecture (30s)

Show the updated Excalidraw diagram. Explain:
- "Agent traces flow from the sandbox to MLflow, infrastructure traces go to
  Tempo. The supervisor handles everything, zero agent config."

### Scene 2: Inside the Sandbox (1 min)

**Automated (demo-magic):**

```bash
# Enter the sandbox
openshell --gateway-insecure sandbox exec openshell-saw -- bash

# Show the OTEL env vars (set automatically by supervisor)
env | grep OTEL
# -> OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318
# -> OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf

# Show the relay is listening inside the sandbox network namespace
ss -tlnp | grep 4318
# -> LISTEN 127.0.0.1:4318
```

Talking point: "No configuration. The supervisor set these env vars
automatically. Any OTel-instrumented agent will export to this endpoint."

### Scene 3: Run the Demo Agent (1 min)

**Semi-automated (demo-magic types, wait for response):**

```bash
# Run a Python agent that makes a real GPT-4o call
python3 /sandbox/demo-agent.py "What is the capital of France?"
```

Expected output:
```
The capital of France is Paris.

Traces exported to http://127.0.0.1:4318
```

Talking point: "The agent used the standard OpenAI SDK with
opentelemetry-instrumentation-openai. Every LLM call is automatically traced.
The traces went through the supervisor relay, got enriched with sandbox
metadata, and are now in MLflow."

### Scene 4: Traces in MLflow (1 min)

**Manual (browser):**

Switch to MLflow UI tab (`https://rh-ai.apps.cluster-4xkdx.dyn.redhatworkshops.io/mlflow`).
Refresh the traces list. Click on the most recent trace. Show:
- The span tree: `agent-task` -> `tool-call` (file read) + `chat` (LLM call)
- The span attributes: `gen_ai.request.model: gpt-4o`, token counts
- The latency: ~5s (real OpenAI API call)

Talking point: "This trace was generated inside a network-isolated sandbox.
The agent didn't know about MLflow, didn't configure an endpoint. The
supervisor relay handled the export transparently."

### Scene 5: Manual Curl Test (1 min)

**Automated (demo-magic):**

Back in the sandbox shell:

```bash
# Send a manual OTLP trace via curl
NOW=$(date +%s%N) && END=$((NOW + 100000000)) && \
curl -s -X POST http://127.0.0.1:4318/v1/traces \
  -H "Content-Type: application/json" \
  -d "{\"resourceSpans\":[{\"resource\":{\"attributes\":[ \
    {\"key\":\"service.name\",\"value\":{\"stringValue\":\"manual-curl-test\"}}]}, \
    \"scopeSpans\":[{\"scope\":{\"name\":\"curl-test\"}, \
    \"spans\":[{\"traceId\":\"$(openssl rand -hex 16)\", \
    \"spanId\":\"$(openssl rand -hex 8)\", \
    \"name\":\"hello-from-curl\",\"kind\":1, \
    \"startTimeUnixNano\":\"${NOW}\",\"endTimeUnixNano\":\"${END}\", \
    \"attributes\":[{\"key\":\"test.source\",\"value\":{\"stringValue\":\"manual-curl\"}}, \
    {\"key\":\"llm.model\",\"value\":{\"stringValue\":\"gpt-4o\"}}]}]}]}]}"
```

Talking point: "Any tool that speaks OTLP HTTP can send traces to the relay.
curl, Python, Go, Rust, Node.js. The relay accepts both protobuf and JSON."

### Scene 6: Trace in MLflow (from curl) (30s)

**Manual (browser):**

Switch to MLflow. Refresh. Show the curl trace appeared with the
`hello-from-curl` span and `test.source: manual-curl` attribute.

### Scene 7: (Optional) Show Enrichment in Jaeger (1 min)

Switch to Jaeger UI (`https://tempo-openshell-tempo-jaegerui-openshell-agents.apps.cluster-4xkdx.dyn.redhatworkshops.io`).
Search for service `demo-agent`. Show the resource attributes that the relay
injected:
- `openshell.sandbox.id`: the sandbox UUID
- `openshell.telemetry.source: "agent"` (routing marker)
- `openshell.sandbox.policy: "openshell-saw"`
- `openshell.sandbox.user: "998"`
- `openshell.sandbox.driver: "container"`

Talking point: "The supervisor enriched the spans with sandbox identity.
Operators can query by sandbox ID across all their agents. These attributes
are visible in Jaeger/Tempo and any OTLP-native backend. MLflow's OTLP
ingestion currently strips resource attributes, which is a known gap."

## What Can Be Automated

| Scene | Automatable | Notes |
|-------|-------------|-------|
| 1. Architecture | Yes | Static diagram, open in browser |
| 2. Sandbox env | Yes | demo-magic typed commands |
| 3. Demo agent | Yes | demo-magic types command, wait for GPT response |
| 4. MLflow traces | No | Browser click-through |
| 5. Curl test | Yes | demo-magic typed command |
| 6. MLflow curl trace | No | Browser click-through |
| 7. Jaeger enrichment | No | Browser click-through |

## Prerequisites Before Recording

1. Cluster running with SAW deployed (VM, gateway, collector, MLflow, Tempo)
2. Gateway running custom build (v0.0.103-dev.4 from `6115-supervisor-otlp-relay`)
3. Cached supervisor binary replaced with our relay-enabled build
4. Sandbox created and healthy (`openshell sandbox list` shows Ready)
5. Demo agent packages installed (`openai`, `opentelemetry-instrumentation-openai`)
6. `demo-agent.py` deployed at `/sandbox/demo-agent.py`
7. `openshell gateway login` completed (OIDC auth as alice)
8. Browser tabs open: MLflow UI, Jaeger UI
9. Terminal sized for recording (e.g., 120x30)

## Verified Working State (2026-08-18)

- Demo agent ran successfully: "What is the capital of France?" -> "The capital of France is Paris."
- Trace `tr-3b227abcaae9c8fdd43e9530f5ff1265` visible in MLflow (5.2s, status OK)
- Split-stream routing confirmed: agent traces to MLflow only, infra to Tempo only
- Relay started in netns: `telemetry relay started (netns)` in supervisor logs
- OTEL env vars set in child process: confirmed via `/proc/<pid>/environ`

## Updated Diagram Needs

The Excalidraw file `saw-trace-architecture.excalidraw` needs updating:
- Replace "socat forwarder" with "Supervisor OTLP Relay"
- Add "enrichment" step (sandbox attributes)
- Add "bounded buffer" in the relay
- Change routing label from "service.name" to "openshell.telemetry.source"
- Add env var injection arrow from supervisor to agent process
- Remove network policy exception note
- Add "Red Hat" product labels: RHOAI MLflow, Red Hat Tempo, Red Hat OpenTelemetry
