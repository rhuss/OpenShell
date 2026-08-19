<!-- SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# OTLP Relay Spike: Results and Observability Roadmap

**Date**: 2026-08-18
**Participants**: Roland Huss
**Branch**: `6115-supervisor-otlp-relay`
**Spike Report**: `specs/001-supervisor-otlp-relay/SPIKE-REPORT.md`

## Context

OpenShell sandboxes are network-isolated by design. Agents inside them can't export telemetry to external collectors because the egress proxy blocks OTLP traffic. The workaround we've been using (NemoClaw's diagnostics-otel plugin plus a socat forwarder plus a network policy exception) proved the value of agent traces in backends like MLflow, but it was fragile and NemoClaw-specific. We needed something that works for any OTel-instrumented agent, with zero agent-side configuration.

This spike built that: an OTLP relay inside the sandbox supervisor that accepts agent traces on `127.0.0.1:4318`, enriches them with sandbox identity, and forwards them to the gateway over the existing session protocol. The gateway exports them to an OTel Collector, which routes agent traces to MLflow and infrastructure traces to Tempo.

## What We Built (31/33 Tasks Complete)

The relay is a new module in `openshell-supervisor-network` with four components: an HTTP receiver that accepts both protobuf and JSON OTLP, an enrichment step that injects sandbox resource attributes, a bounded ring buffer (4096 slots), and a forwarder that sends `TelemetryData` messages to the gateway via `try_send` (non-blocking, so telemetry can never stall control operations).

On the gateway side, a dedicated `TelemetryRelayExporter` connects directly to the configured OTLP collector, bypassing the gateway's own `SdkTracerProvider`. This matters because the gateway's tracer would overwrite the supervisor-enriched resource attributes with the gateway's own identity.

The supervisor sets `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318` and `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` in the agent process environment via `child_env.rs`. The agent's OTel SDK picks these up automatically. No agent code changes required.

We validated this end-to-end on a demo.redhat.com OCP 4.22.8 cluster running the SAW (Secure Agent Workspace) pattern with Red Hat products: RHOAI 3.4.3 (MLflow operator), Red Hat Tempo (Jaeger UI), and Red Hat OpenTelemetry (managed collector). A Python agent using the OpenAI SDK with `opentelemetry-instrument` auto-instrumentation made real GPT-4o calls through `inference.local`, and the traces appeared in both MLflow and Jaeger with sandbox resource attributes attached.

## Performance Results

The evidence suggests we don't need a separate telemetry channel. The `try_send` architecture provides complete isolation between telemetry and control messages by design, and the load test data confirms it.

We measured `openshell sandbox exec` round-trip latency at sustained span rates from 100 to 10,000 per second. The baseline (no telemetry load) averaged 684ms. At every rate we tested, the loaded latency was within noise of baseline:

| Rate | Exec Latency | Ratio |
|------|-------------|-------|
| 100/s | 673ms | 0.98x |
| 500/s | 690ms | 1.01x |
| 1,000/s | 672ms | 0.98x |
| 2,000/s | 691ms | 1.01x |
| 5,000/s | 683ms | 1.00x |
| 10,000/s | 672ms | 0.98x |

At 10,000 spans/s the Python OTel SDK's own `BatchSpanProcessor` queue started dropping, not our relay buffer. The relay itself never saturated.

We also tested multi-sandbox load: two sandboxes on the same gateway, each generating 1,000 spans/s (2,000 total through the shared gRPC session). Both sandboxes showed 0.96x latency ratio. Zero drops, zero degradation.

This leads us to a clear conclusion about [#2642](https://github.com/NVIDIA/OpenShell/issues/2642) (dedicated telemetry channel): the shared session channel is sufficient. A separate gRPC stream would add connection management, TLS handshake, and reconnection complexity for zero measurable benefit. We recommend closing #2642 as "won't fix" with a reference to this performance data.

## Roadmap Context

The spike addresses [#2641](https://github.com/NVIDIA/OpenShell/issues/2641) within the broader Enterprise Observability roadmap ([#1055](https://github.com/NVIDIA/OpenShell/issues/1055)):

```
#1055 Enterprise Observability
├── #2507  Gateway OTel export surface
├── #2508  Supervisor OTel span emission
│   └── #2642  Shared telemetry relay infrastructure ←─┐
├── #2641  Agent-level observability (OTLP relay)  ◄── THIS SPIKE
│   └── #2642  (also sub-issue of #2641) ──────────────┘
├── #2639  CLI passive trace propagation
├── #2640  OCSF-to-OTel correlation
├── #909   Metrics instrumentation
├── #1922  Sandbox log collection
└── ...
```

## Issue Abstracts

### [#1055 Enterprise Observability](https://github.com/NVIDIA/OpenShell/issues/1055)
**State**: Open (roadmap)

The umbrella tracking issue for all observability work. Covers logs, metrics, telemetry, and dashboards across gateways and sandboxes. The goal is to give operators enough visibility to understand what OpenShell is doing, why something is slow, and whether policy decisions are correct.

### [#2507 Gateway OTel Export Surface](https://github.com/NVIDIA/OpenShell/issues/2507)
**State**: Open | **Labels**: `topic:observability`, `area:gateway`, `area:cluster`

The gateway process currently offers three observability surfaces (metrics endpoint, structured logs, OCSF events) but none of them answer "what is my gateway doing right now?" This issue tracks adding OpenTelemetry span export from the gateway itself, so operators can see gRPC request latency, compute driver calls, auth decisions, and provider credential refreshes in their trace backend.

### [#2508 Supervisor OTel Span Emission](https://github.com/NVIDIA/OpenShell/issues/2508)
**State**: Open | **Labels**: `topic:observability`, `area:supervisor`, `area:sandbox`

The companion to #2507 for the sandbox side. Tracks emitting OTel spans from the supervisor process itself (proxy decisions, policy evaluation, SSH sessions, Landlock setup). These are infrastructure traces, distinct from agent-emitted traces. The relay infrastructure built in our spike (#2641) provides the transport path these spans would use to reach the gateway.

### [#2641 Agent-Level Trace Collection via Supervisor OTLP Relay](https://github.com/NVIDIA/OpenShell/issues/2641)
**State**: Open (accepted) | **This spike**

Agents inside network-isolated sandboxes can't export traces to external collectors. This issue proposed building an OTLP relay into the supervisor that accepts agent traces on localhost, enriches them with sandbox identity, and forwards them to the gateway. The spike implemented this end-to-end across all four compute drivers (Docker, Podman, Kubernetes, VM).

### [#2642 Shared Telemetry Relay Infrastructure](https://github.com/NVIDIA/OpenShell/issues/2642)
**State**: Open (accepted) | **Recommendation: close as won't fix**

Proposed evaluating whether telemetry should use a dedicated gRPC channel or share the control session. The spike answered this definitively: the shared channel with `try_send` shows zero latency impact up to 10,000 spans/s. A separate channel adds complexity for no benefit. The relay infrastructure built in #2641 already handles both agent and supervisor trace streams.

### [#2639 CLI Passive TRACEPARENT Forwarding](https://github.com/NVIDIA/OpenShell/issues/2639)
**State**: Open (accepted)

CI pipelines that call `openshell sandbox create` lose trace continuity at the CLI boundary. When GitHub Actions or GitLab CI sets `TRACEPARENT`, the CLI should forward it so sandbox operations appear as children of the CI trace. This is orthogonal to the relay but connects to the same observability story: end-to-end trace continuity from CI through gateway through sandbox through agent.

### [#2640 OCSF-to-OTel Correlation](https://github.com/NVIDIA/OpenShell/issues/2640)
**State**: Open (accepted)

OCSF security events and OTel traces exist in separate systems with no link between them. When a security reviewer investigates a deny event, they can't jump to the trace that shows what the agent was doing when the deny happened. This issue proposes adding `trace_id` and `span_id` fields to OCSF event builders so security events carry correlation IDs from the active trace context.

### [#909 Metrics Instrumentation](https://github.com/NVIDIA/OpenShell/issues/909)
**State**: Open

The gateway has no metrics instrumentation today. This issue tracks adding Prometheus-compatible metrics for sandbox lifecycle operations, SSH tunneling, supervisor sessions, and policy evaluation. Connects to the broader observability story alongside tracing (#2507) and logging (#1922).

### [#1922 Sandbox Log Collection](https://github.com/NVIDIA/OpenShell/issues/1922)
**State**: Open | **Labels**: `topic:observability`, `area:sandbox`, `spike`

Operators need durable sandbox log collection, especially on Kubernetes. Today `openshell logs` works for interactive diagnosis but is backed by ephemeral storage. This issue investigates portable collection mechanisms (sidecar log forwarders, journal shipping, OCSF structured log export). The OCSF relay path built in our spike (T019-T021) is one piece of this puzzle.

## Key Findings and Limitations

### What Works

1. **Zero-config agent tracing**: The supervisor sets OTEL env vars automatically. Any OTel-instrumented agent (Python, Node.js, Go, or raw curl) exports to `127.0.0.1:4318` and traces flow to the configured collector.

2. **Sandbox enrichment**: Spans arrive at the collector with `openshell.sandbox.id`, `openshell.telemetry.source: "agent"`, `openshell.sandbox.policy`, `openshell.sandbox.user`, and `openshell.sandbox.driver`. Operators can query by sandbox ID across all agents.

3. **Split-stream routing**: The OTel Collector uses filter processors on `openshell.telemetry.source` to route agent traces to MLflow and infrastructure traces to Tempo. Agent traces also go to Tempo for the complete picture.

4. **Non-interference**: The `try_send` architecture delivers on the US3 non-interference guarantee with zero measurable overhead up to 10,000 spans/s.

5. **Cross-driver support**: The relay works on Docker, Podman, Kubernetes (combined + sidecar), and VM drivers. The binding mechanism differs (netns for Docker/Podman, direct for K8s/VM) but the agent always sees `127.0.0.1:4318`.

### What Doesn't Work Yet

1. **MLflow strips resource attributes**: MLflow's OTLP ingestion converts spans to its internal format and discards resource attributes. The sandbox enrichment is invisible in MLflow's UI. It shows correctly in Jaeger/Tempo. A dual injection approach (resource + span attributes) would fix this.

2. **K8s driver always sets OTEL env vars**: The Kubernetes driver can't know at pod creation time whether the relay will be active. It unconditionally sets the env vars, which causes OTel SDK retry noise in agent logs when no relay is running.

3. **Static workspace_id**: The `openshell.workspace.id` attribute is empty at relay startup because the policy poll loop hasn't fetched the workspace yet. Making the metadata dynamic requires threading a `watch::Receiver` through the receiver and enrichment functions.

4. **No relay activation gating**: The relay starts unconditionally on Linux, even when the gateway has no OTLP endpoint configured. Forwarded telemetry messages are silently dropped on the gateway side.

5. **OCSF relay wiring incomplete (T021)**: The `OcsfRelayLayer` and `RateLimitedOcsfSink` are implemented but not wired into the tracing subscriber. This is a P3 user story that requires resolving a chicken-and-egg ordering issue between subscriber setup and relay startup.

6. **NemoClaw OTEL build broken on demo.redhat.com**: The `make build-nemoclaw OTEL=1` build fails on demo.redhat.com clusters due to a security package inventory mismatch in the NemoClaw Dockerfile. The diagnostics-otel plugin can't be installed at runtime (private npm package). We used a Python OpenAI SDK agent with `opentelemetry-instrument` auto-instrumentation as an alternative demo harness.

## Proposed Next Steps

Given these constraints, here's what we recommend for the observability roadmap, ordered by impact and effort:

### Immediate (Before Merging the Spike Branch)

1. **E2e validation on a clean environment**: The SAW demo.redhat.com setup has accumulated state from debugging. Run the `SMOKE-TEST.md` scenarios on a fresh Linux host with Docker or Podman to confirm the implementation works without SAW-specific workarounds.

2. **Fix the NemoClaw OTEL build**: File an issue against the NemoClaw repo for the security package inventory mismatch. This blocks the demo story for NemoClaw-based agent tracing.

### Short Term (Next Sprint)

3. **Dual span+resource attribute injection** (Limitation #1): Add sandbox attributes at both the resource level (for OTLP-native backends) and span level (for MLflow). Small change in `enrichment.rs`. This makes the enrichment visible everywhere.

4. **Relay activation gating** (Limitation #4): Gate the relay startup on the `telemetry_relay` capability from `SessionAccepted.capabilities`. Requires either lazy relay startup after session handshake, or starting unconditionally but gating env var injection on capability confirmation.

5. **Close [#2642](https://github.com/NVIDIA/OpenShell/issues/2642)**: The performance data conclusively shows a separate telemetry channel is not needed. Close with a reference to the spike report's performance section.

### Medium Term

6. **Dynamic workspace_id** (Limitation #3): Replace `workspace_id: String` with a `watch::Receiver<String>` in `SandboxMetadata`. The receiver is `Clone`, so it flows through the existing clone-per-connection pattern.

7. **K8s conditional env vars** (Limitation #2): Propagate the gateway's OTLP config status to the Kubernetes driver at sandbox creation time. Add a `telemetry_relay_enabled` field to `SandboxSpec` or `SandboxPodParams`.

8. **OCSF relay wiring** (Limitation #5, T021): Create the telemetry buffer early (before subscriber setup), pass the `TelemetrySender` to `OcsfRelayLayer`, pass the `TelemetryReceiver` to the relay forwarder when it starts. Completes the P3 user story for OCSF event relay.

9. **Collector routing documentation**: The split-stream collector configuration (filter processors, error_mode, OTTL conditions) is non-trivial. Document it as part of the OTLP setup guide, including the workspace label requirement and token authentication for MLflow.

### Longer Term (Roadmap Items)

10. **Gateway OTel export** ([#2507](https://github.com/NVIDIA/OpenShell/issues/2507)): The relay infrastructure provides the transport path. The gateway needs to emit spans from its own operations (gRPC requests, compute driver calls, auth decisions).

11. **Supervisor OTel emission** ([#2508](https://github.com/NVIDIA/OpenShell/issues/2508)): The supervisor needs to emit spans from proxy decisions, policy evaluation, and SSH sessions. These would flow through the same relay transport.

12. **CLI trace continuity** ([#2639](https://github.com/NVIDIA/OpenShell/issues/2639)): Forward `TRACEPARENT` from CI environments through the CLI to sandbox operations.

13. **OCSF-OTel correlation** ([#2640](https://github.com/NVIDIA/OpenShell/issues/2640)): Add `trace_id`/`span_id` to OCSF event builders so security events link to the trace context.

14. **Metrics instrumentation** ([#909](https://github.com/NVIDIA/OpenShell/issues/909)): Add Prometheus-compatible metrics to the gateway.

15. **Sandbox log collection** ([#1922](https://github.com/NVIDIA/OpenShell/issues/1922)): Portable log collection for Kubernetes deployments.

## Demo

A demo script is available at `otel-spike/saw-otel-relay-demo.sh` using demo-magic for automated terminal presentation. The demo shows:

1. Zero-config env vars inside the sandbox
2. A Python agent making real GPT-4o calls with auto-instrumented traces
3. Traces appearing in MLflow
4. Manual curl OTLP trace
5. Enriched resource attributes in Jaeger
6. Performance results table

The demo runs against the SAW deployment on `cluster-4xkdx.dyn.redhatworkshops.io`.

## Artifacts

| Artifact | Location |
|----------|----------|
| Spike report | `specs/001-supervisor-otlp-relay/SPIKE-REPORT.md` |
| Spec | `specs/001-supervisor-otlp-relay/spec.md` |
| Tasks | `specs/001-supervisor-otlp-relay/tasks.md` (31/33 done) |
| Brainstorm (cross-driver) | `brainstorm/03-cross-driver-otlp-relay.md` |
| Architecture doc | `architecture/sandbox.md` (Telemetry Relay section) |
| Demo script | `otel-spike/saw-otel-relay-demo.sh` |
| Demo agent | `otel-spike/demo-agent.py` |
| Demo plan | `otel-spike/demo-plan.md` |
