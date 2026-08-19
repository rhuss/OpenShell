#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
# SAW OTLP Relay Spike Demo
# ==========================
# Demonstrates how the supervisor's built-in OTLP relay transparently
# forwards agent traces from inside a sandbox to MLflow.
#
# Prerequisites:
#   - oc login to the demo cluster
#   - openshell gateway login (OIDC as alice)
#   - SAW sandbox running with relay-enabled supervisor
#   - demo-agent.py deployed in sandbox
#   - Browser tabs: MLflow UI, Jaeger UI
#
# Usage:
#   bash saw-otel-relay-demo.sh          # interactive (press Enter)
#   bash saw-otel-relay-demo.sh --auto   # auto-advance (3s delay)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/demo-magic.sh"

TYPE_SPEED=40
export OPENSHELL_GATEWAY_INSECURE=true

# Sandbox name
SANDBOX="${SANDBOX_NAME:-openshell-saw}"

# Helper: run a command inside the sandbox
sandbox_exec() {
  openshell sandbox exec -n "$SANDBOX" -- "$@"
}

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

comment() { echo -e "${CYAN}# $*${NC}"; }
section() {
  echo ""
  echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${GREEN}  $*${NC}"
  echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
}

clear

########################
# Scene 1: Architecture
########################

section "Supervisor OTLP Relay — Zero-Config Agent Tracing"

echo -e "${CYAN}"
cat << 'DIAGRAM'

  Sandbox (network-isolated)
  ┌──────────────────────────────────────────┐
  │                                          │
  │  Agent (any OTel-instrumented process)   │
  │    OTEL_EXPORTER_OTLP_ENDPOINT           │
  │    = http://127.0.0.1:4318               │
  │    (set by supervisor, zero agent config)│
  │         │                                │
  │         ▼ POST /v1/traces                │
  │  ┌────────────────────────┐              │
  │  │ Supervisor OTLP Relay  │              │
  │  │  • enrichment:         │              │
  │  │    sandbox.id, user,   │              │
  │  │    telemetry.source    │              │
  │  │  • buffer (4096 slots) │              │
  │  │  • try_send (non-block)│              │
  │  └──────────┬─────────────┘              │
  └─────────────┼────────────────────────────┘
                │ gRPC session (shared with control)
                ▼
  ┌──────────────────────┐
  │   OpenShell Gateway  │───────────┐
  └──────────────────────┘           │ gRPC :4317
                                     ▼
               ┌──────────────────────────────┐
               │  OTel Collector (Red Hat)    │
               │  filter: telemetry.source    │
               │   = "agent" → MLflow         │
               │   else     → Tempo           │
               └──────┬──────────────┬────────┘
                      ▼              ▼
               ┌──────────┐   ┌──────────┐
               │  MLflow  │   │  Tempo   │
               │  (RHOAI) │   │ Jaeger UI│
               │  agent   │   │  infra   │
               │  traces  │   │  traces  │
               └──────────┘   └──────────┘

DIAGRAM
echo -e "${NC}"

wait

########################
# Scene 2: Sandbox Env
########################

section "Scene 1: Zero-Config Environment Inside the Sandbox"

comment "The supervisor automatically sets OTEL env vars for agent processes."
comment "The agent doesn't configure anything — it just works."

echo ""
comment "Let's look at the OTEL env vars inside the sandbox:"
pe "openshell sandbox exec -n '${SANDBOX}' -- env | grep OTEL"

echo ""
comment "Let's verify the relay is accepting connections on port 4318:"
pe "openshell sandbox exec -n '${SANDBOX}' -- curl -s -o /dev/null -w 'HTTP %{http_code}\n' http://127.0.0.1:4318/v1/traces"
comment "HTTP 404 = relay is listening (GET not supported, it expects POST)"

wait

########################
# Scene 3: Demo Agent
########################

section "Scene 2: Agent Makes a Real LLM Call — Traces Flow Automatically"

comment "This is a plain Python script using the OpenAI SDK."
comment "It has ZERO OpenTelemetry code — no imports, no setup, nothing."
echo ""
comment "Here's the agent code:"
pe "bat --style=plain --theme=base16 --language=python ${SCRIPT_DIR}/demo-agent.py 2>/dev/null || cat ${SCRIPT_DIR}/demo-agent.py"

wait

comment "We run it with 'opentelemetry-instrument' which auto-discovers"
comment "the OpenAI instrumentor and traces every LLM call. Zero code changes."
echo ""
pe "openshell sandbox exec -n '${SANDBOX}' -- env OTEL_METRICS_EXPORTER=none OTEL_LOGS_EXPORTER=none OTEL_SERVICE_NAME=demo-agent PATH=/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin opentelemetry-instrument python3 /sandbox/demo-agent.py 'What is the capital of France?'"

echo ""
comment "The trace was exported to the relay at 127.0.0.1:4318."
comment "It flows: relay → gateway → OTel collector → MLflow."
echo ""
echo -e "${YELLOW}  ➜ Switch to MLflow UI to see the trace${NC}"

wait

########################
# Scene 4: Manual Curl
########################

section "Scene 3: Raw OTLP — Any Tool Can Send Traces"

comment "The relay accepts standard OTLP HTTP (protobuf and JSON)."
comment "Any tool that speaks OTLP can send traces — curl, Python, Go, Rust."

echo ""
echo -e "${YELLOW}  ➜ In the sandbox shell, run:${NC}"
echo ""
echo -e "${CYAN}  NOW=\$(date +%s%N) && END=\$((NOW + 100000000)) && \\\\${NC}"
echo -e "${CYAN}  curl -s -X POST http://127.0.0.1:4318/v1/traces \\\\${NC}"
echo -e "${CYAN}    -H \"Content-Type: application/json\" \\\\${NC}"
echo -e "${CYAN}    -d '{\"resourceSpans\":[{\"resource\":{\"attributes\":[{\"key\":\"service.name\",${NC}"
echo -e "${CYAN}    \"value\":{\"stringValue\":\"manual-curl-test\"}}]},\"scopeSpans\":[{\"scope\":${NC}"
echo -e "${CYAN}    {\"name\":\"curl\"},\"spans\":[{\"traceId\":\"'\$(openssl rand -hex 16)'\",${NC}"
echo -e "${CYAN}    \"spanId\":\"'\$(openssl rand -hex 8)'\",\"name\":\"hello-from-curl\",\"kind\":1,${NC}"
echo -e "${CYAN}    \"startTimeUnixNano\":\"\${NOW}\",\"endTimeUnixNano\":\"\${END}\",\"attributes\":${NC}"
echo -e "${CYAN}    [{\"key\":\"test\",\"value\":{\"stringValue\":\"manual\"}}]}]}]}]}'${NC}"
echo ""

wait

########################
# Scene 5: Jaeger
########################

section "Scene 4: Sandbox Enrichment — Visible in Jaeger"

comment "The relay enriches every span with sandbox resource attributes:"
comment "  openshell.sandbox.id       — sandbox UUID"
comment "  openshell.telemetry.source — 'agent' (routing marker)"
comment "  openshell.sandbox.policy   — sandbox name"
comment "  openshell.sandbox.user     — sandbox UID"
comment "  openshell.sandbox.driver   — compute driver"
echo ""
comment "These are visible in Jaeger/Tempo (any OTLP-native backend)."
comment "MLflow's OTLP ingestion currently strips resource attributes"
comment "— a known gap documented in the spike report."
echo ""
echo -e "${YELLOW}  ➜ Switch to Jaeger UI, search for service 'demo-agent'${NC}"

wait

########################
# Scene 6: Performance
########################

section "Scene 5: Non-Interference — Zero Overhead"

comment "The relay shares the gRPC session with control messages."
comment "try_send is non-blocking: telemetry never stalls control ops."
echo ""
comment "Load test results (SAW on OCP 4.22.8):"
echo ""
echo -e "${CYAN}  Rate (spans/s) │ Exec Latency │ Ratio to Baseline${NC}"
echo -e "${CYAN}  ───────────────┼──────────────┼──────────────────${NC}"
echo -e "${CYAN}  0 (baseline)   │ 684ms        │ 1.00x${NC}"
echo -e "${CYAN}  100            │ 673ms        │ 0.98x${NC}"
echo -e "${CYAN}  1,000          │ 672ms        │ 0.98x${NC}"
echo -e "${CYAN}  5,000          │ 683ms        │ 1.00x${NC}"
echo -e "${CYAN}  10,000         │ 672ms        │ 0.98x${NC}"
echo ""
comment "Zero measurable impact up to 10,000 spans/sec."
comment "A separate telemetry channel is not needed (#2642)."

wait

########################
# Wrap-up
########################

section "Summary"

echo -e "${CYAN}"
cat << 'SUMMARY'

  ✓ Zero agent config — supervisor sets OTEL env vars automatically
  ✓ Any OTel framework — Python, Node.js, Go, curl, anything
  ✓ Sandbox enrichment — traces tagged with sandbox identity
  ✓ Split-stream routing — agent traces → MLflow, infra → Tempo
  ✓ Non-interference — 0 overhead at 10,000 spans/sec
  ✓ Red Hat products — RHOAI MLflow, Red Hat Tempo, Red Hat OTel

  Branch:  6115-supervisor-otlp-relay
  Issue:   #2641
  Report:  specs/001-supervisor-otlp-relay/SPIKE-REPORT.md

SUMMARY
echo -e "${NC}"
