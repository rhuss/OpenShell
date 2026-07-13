#!/usr/bin/env bash

# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Warm Pool gRPC PoC - Smoke Test
#
# Tests the end-to-end warm pool claim + ActivateSandbox flow.
# Run from the repository root on branch 6113-warm-pool-grpc-poc.
#
# Prerequisites:
#   - kubectl configured for your cluster
#   - OpenShell deployed (gateway running, TLS secrets, service account)
#   - Agent Sandbox operator installed (SandboxWarmPool CRDs available)
#   - grpcurl installed (brew install grpcurl / go install github.com/fullstorydev/grpcurl)
#   - Sandbox image pre-pulled on nodes (optional, speeds up init containers)
#
# Usage:
#   ./experiments/smoke-test.sh [--namespace openshell] [--runs 3] [--skip-deploy]

set -euo pipefail

NAMESPACE="${NAMESPACE:-openshell}"
RUNS="${RUNS:-3}"
SKIP_DEPLOY=false
SUPERVISOR_IMAGE="${SUPERVISOR_IMAGE:-quay.io/rhuss/openshell-supervisor:warm-pool-poc}"
SANDBOX_IMAGE="${SANDBOX_IMAGE:-ghcr.io/nvidia/openshell-community/sandboxes/base:latest}"
TEMPLATE_NAME="openshell-warm-unidentified"
POOL_NAME="openshell-grpc-pool"
POOL_REPLICAS="${POOL_REPLICAS:-3}"
LOCAL_PORT=19090
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace)  NAMESPACE="$2"; shift 2 ;;
    --runs)       RUNS="$2"; shift 2 ;;
    --skip-deploy) SKIP_DEPLOY=true; shift ;;
    --image)      SUPERVISOR_IMAGE="$2"; shift 2 ;;
    --help|-h)
      echo "Usage: $0 [--namespace NS] [--runs N] [--skip-deploy] [--image IMG]"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

log()  { echo "$(date +%H:%M:%S) $*"; }
pass() { echo "  PASS: $*"; }
fail() { echo "  FAIL: $*"; FAILURES=$((FAILURES + 1)); }
cleanup_portforward() { kill "$(lsof -ti "tcp:${LOCAL_PORT}" 2>/dev/null)" 2>/dev/null || true; }

FAILURES=0

# ---------------------------------------------------------------------------
# Preflight checks
# ---------------------------------------------------------------------------
log "=== Warm Pool gRPC PoC Smoke Test ==="
log ""

log "Preflight: checking tools..."
for tool in kubectl grpcurl; do
  command -v "$tool" &>/dev/null || { fail "$tool not found"; exit 1; }
done
pass "kubectl and grpcurl available"

log "Preflight: checking cluster access..."
kubectl get nodes &>/dev/null || { fail "Cannot reach cluster"; exit 1; }
pass "Cluster reachable ($(kubectl get nodes --no-headers 2>/dev/null | wc -l | tr -d ' ') nodes)"

log "Preflight: checking CRDs..."
kubectl api-resources --api-group=extensions.agents.x-k8s.io 2>/dev/null | grep -q SandboxWarmPool || {
  fail "SandboxWarmPool CRD not found. Install the Agent Sandbox operator first."
  exit 1
}
pass "Agent Sandbox CRDs present"

log "Preflight: checking namespace $NAMESPACE..."
kubectl get ns "$NAMESPACE" &>/dev/null || {
  fail "Namespace $NAMESPACE not found"
  exit 1
}
pass "Namespace exists"

log ""

# ---------------------------------------------------------------------------
# Step 1: Deploy SandboxTemplate + WarmPool
# ---------------------------------------------------------------------------
if [ "$SKIP_DEPLOY" = false ]; then
  log "Step 1: Deploying SandboxTemplate and WarmPool..."

  cat <<EOF | kubectl apply -f -
apiVersion: extensions.agents.x-k8s.io/v1beta1
kind: SandboxTemplate
metadata:
  name: ${TEMPLATE_NAME}
  namespace: ${NAMESPACE}
spec:
  envVarsInjectionPolicy: Allowed
  podTemplate:
    spec:
      automountServiceAccountToken: false
      dnsPolicy: ClusterFirst
      serviceAccountName: openshell-sandbox
      securityContext:
        fsGroup: 1000
      initContainers:
      - name: workspace-init
        image: ${SANDBOX_IMAGE}
        command: ["sh", "-c", "if [ ! -f /workspace-pvc/.workspace-initialized ]; then if [ -d /sandbox ]; then tar -C /sandbox -cf - . | tar -C /workspace-pvc -xpf -; fi; touch /workspace-pvc/.workspace-initialized; fi"]
        securityContext:
          runAsUser: 0
        volumeMounts:
        - { mountPath: /workspace-pvc, name: workspace }
      containers:
      - name: agent
        image: ${SANDBOX_IMAGE}
        command: ["/opt/openshell/bin/openshell-sandbox", "--unidentified", "--health-check", "--health-port", "8080"]
        env:
        - { name: OPENSHELL_ENDPOINT,          value: "https://openshell.${NAMESPACE}.svc.cluster.local:8080" }
        - { name: OPENSHELL_SANDBOX_COMMAND,    value: "sleep infinity" }
        - { name: OPENSHELL_TELEMETRY_ENABLED,  value: "true" }
        - { name: OPENSHELL_SSH_SOCKET_PATH,    value: "/run/openshell/ssh.sock" }
        - { name: OPENSHELL_TLS_CA,             value: "/etc/openshell-tls/client/ca.crt" }
        - { name: OPENSHELL_TLS_CERT,           value: "/etc/openshell-tls/client/tls.crt" }
        - { name: OPENSHELL_TLS_KEY,            value: "/etc/openshell-tls/client/tls.key" }
        - { name: OPENSHELL_K8S_SA_TOKEN_FILE,  value: "/var/run/secrets/openshell/token" }
        - { name: OPENSHELL_SANDBOX_UID,        value: "1000" }
        - { name: OPENSHELL_SANDBOX_GID,        value: "1000" }
        ports:
        - { containerPort: 9090, name: grpc,   protocol: TCP }
        - { containerPort: 8080, name: health, protocol: TCP }
        readinessProbe:
          httpGet: { path: /readyz, port: 8080 }
          initialDelaySeconds: 1
          periodSeconds: 2
        securityContext:
          appArmorProfile: { type: Unconfined }
          capabilities:
            add: [SYS_ADMIN, NET_ADMIN, SYS_PTRACE, SYSLOG]
          runAsUser: 0
        volumeMounts:
        - { mountPath: /etc/openshell-tls/client,  name: openshell-client-tls, readOnly: true }
        - { mountPath: /var/run/secrets/openshell,  name: openshell-sa-token,   readOnly: true }
        - { mountPath: /opt/openshell/bin,          name: openshell-supervisor-bin }
        - { mountPath: /sandbox,                    name: workspace }
      volumes:
      - name: openshell-client-tls
        secret: { defaultMode: 256, secretName: openshell-client-tls }
      - name: openshell-sa-token
        projected:
          defaultMode: 256
          sources:
          - serviceAccountToken: { audience: openshell-gateway, expirationSeconds: 3600, path: token }
      - name: openshell-supervisor-bin
        image: { reference: "${SUPERVISOR_IMAGE}" }
  volumeClaimTemplates:
  - metadata: { name: workspace }
    spec:
      accessModes: [ReadWriteOnce]
      resources: { requests: { storage: 2Gi } }
EOF

  cat <<EOF | kubectl apply -f -
apiVersion: extensions.agents.x-k8s.io/v1beta1
kind: SandboxWarmPool
metadata:
  name: ${POOL_NAME}
  namespace: ${NAMESPACE}
spec:
  sandboxTemplateRef:
    name: ${TEMPLATE_NAME}
  replicas: ${POOL_REPLICAS}
EOF

  log "  Waiting for ${POOL_REPLICAS} warm pool pods to become Ready..."
  WAIT_START=$(date +%s)
  while true; do
    READY=$(kubectl -n "$NAMESPACE" get sandboxwarmpool "$POOL_NAME" \
      -o jsonpath='{.status.readyReplicas}' 2>/dev/null || echo 0)
    [ "${READY:-0}" -ge "$POOL_REPLICAS" ] && break
    ELAPSED=$(( $(date +%s) - WAIT_START ))
    [ "$ELAPSED" -gt 300 ] && { fail "Warm pool pods not ready after 5 minutes"; exit 1; }
    sleep 2
  done
  POOL_READY_SECS=$(( $(date +%s) - WAIT_START ))
  pass "Warm pool ready (${POOL_REPLICAS}/${POOL_REPLICAS} replicas, ${POOL_READY_SECS}s)"
else
  log "Step 1: Skipped (--skip-deploy)"
  READY=$(kubectl -n "$NAMESPACE" get sandboxwarmpool "$POOL_NAME" \
    -o jsonpath='{.status.readyReplicas}' 2>/dev/null || echo 0)
  [ "${READY:-0}" -gt 0 ] || { fail "No ready replicas in pool $POOL_NAME"; exit 1; }
  pass "Pool $POOL_NAME has ${READY} ready replicas"
fi

log ""

# ---------------------------------------------------------------------------
# Step 2: Verify unidentified supervisor mode
# ---------------------------------------------------------------------------
log "Step 2: Verifying unidentified supervisor in warm pool pods..."

POD=$(kubectl -n "$NAMESPACE" get pods \
  -l "agents.x-k8s.io/warm-pool-sandbox" \
  -o jsonpath='{.items[0].metadata.name}' \
  --field-selector=status.phase=Running 2>/dev/null)

[ -n "$POD" ] || { fail "No running warm pool pods found"; exit 1; }

# Check process
PROC=$(kubectl -n "$NAMESPACE" exec "$POD" -- ps -o args= -p 1 2>/dev/null || echo "")
if echo "$PROC" | grep -q -- "--unidentified"; then
  pass "Supervisor running with --unidentified flag ($POD)"
else
  fail "Supervisor not running in unidentified mode: $PROC"
fi

# Check health endpoint: if the pod is Ready (1/1), the readiness probe
# (httpGet /readyz:8080) already confirmed the endpoint works.
POD_READY=$(kubectl -n "$NAMESPACE" get pod "$POD" \
  -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' 2>/dev/null)
if [ "$POD_READY" = "True" ]; then
  pass "Health endpoint /readyz confirmed via readiness probe (pod is Ready)"
else
  fail "Pod $POD is not Ready (readiness probe failing)"
fi

# Check gRPC port
GRPC_LISTEN=$(kubectl -n "$NAMESPACE" exec "$POD" -- \
  cat /proc/net/tcp6 2>/dev/null | awk '$2 ~ /:2382$/ {print "listening"}' || echo "")
if [ -n "$GRPC_LISTEN" ]; then
  pass "gRPC port 9090 is listening"
else
  # Fallback: try via port-forward
  cleanup_portforward
  kubectl -n "$NAMESPACE" port-forward "pod/$POD" "${LOCAL_PORT}:9090" &>/dev/null &
  sleep 2
  if grpcurl -plaintext -import-path "${REPO_ROOT}/proto" -proto supervisor.proto \
      "localhost:${LOCAL_PORT}" list &>/dev/null 2>&1; then
    pass "gRPC port 9090 reachable (verified via port-forward)"
  else
    fail "gRPC port 9090 not reachable"
  fi
  cleanup_portforward
fi

# Check log file
LOG_LINES=$(kubectl -n "$NAMESPACE" exec "$POD" -- \
  cat /var/log/openshell.*.log 2>/dev/null | head -5 || echo "")
if echo "$LOG_LINES" | grep -q "unidentified mode"; then
  pass "Logs confirm unidentified startup"
else
  log "  INFO: Log file check inconclusive (non-blocking writer may not have flushed)"
fi

log ""

# ---------------------------------------------------------------------------
# Step 3: End-to-end claim + activation
# ---------------------------------------------------------------------------
log "Step 3: Running end-to-end claim + activation ($RUNS runs)..."
log ""

TOTAL_CLAIM=0
TOTAL_ACTIVATE=0
PASSED=0

for run in $(seq 1 "$RUNS"); do
  # Wait for pool to have ready replicas
  for _ in $(seq 1 30); do
    READY=$(kubectl -n "$NAMESPACE" get sandboxwarmpool "$POOL_NAME" \
      -o jsonpath='{.status.readyReplicas}' 2>/dev/null || echo 0)
    [ "${READY:-0}" -gt 0 ] && break
    sleep 1
  done
  [ "${READY:-0}" -gt 0 ] || { fail "Run $run: No ready replicas after 30s wait"; continue; }

  CLAIM_NAME="smoke-run${run}-$(date +%s)"

  # Create claim and measure
  CLAIM_START=$(date +%s%N)
  cat <<EOF | kubectl apply -f - &>/dev/null
apiVersion: extensions.agents.x-k8s.io/v1beta1
kind: SandboxClaim
metadata:
  name: ${CLAIM_NAME}
  namespace: ${NAMESPACE}
spec:
  warmPoolRef:
    name: ${POOL_NAME}
EOF

  # Poll until Ready
  CLAIM_TIMEOUT=15
  for _ in $(seq 1 $((CLAIM_TIMEOUT * 10))); do
    STATUS=$(kubectl -n "$NAMESPACE" get sandboxclaim "$CLAIM_NAME" \
      -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' 2>/dev/null || echo "")
    [ "$STATUS" = "True" ] && break
    sleep 0.1
  done
  CLAIM_END=$(date +%s%N)
  CLAIM_MS=$(( (CLAIM_END - CLAIM_START) / 1000000 ))

  if [ "$STATUS" != "True" ]; then
    fail "Run $run: Claim did not reach Ready within ${CLAIM_TIMEOUT}s"
    kubectl -n "$NAMESPACE" delete sandboxclaim "$CLAIM_NAME" --wait=false &>/dev/null || true
    continue
  fi

  POD_NAME=$(kubectl -n "$NAMESPACE" get sandboxclaim "$CLAIM_NAME" \
    -o jsonpath='{.status.sandbox.name}')
  POD_IP=$(kubectl -n "$NAMESPACE" get sandboxclaim "$CLAIM_NAME" \
    -o jsonpath='{.status.sandbox.podIPs[0]}')

  # Port-forward to claimed pod
  cleanup_portforward
  kubectl -n "$NAMESPACE" port-forward "pod/$POD_NAME" "${LOCAL_PORT}:9090" &>/dev/null &
  for _ in $(seq 1 20); do
    nc -z localhost "$LOCAL_PORT" 2>/dev/null && break
    sleep 0.25
  done

  # Call ActivateSandbox
  ACTIVATE_START=$(date +%s%N)
  RESULT=$(grpcurl -plaintext \
    -import-path "${REPO_ROOT}/proto" -proto supervisor.proto \
    -d "{
      \"sandbox_id\": \"${CLAIM_NAME}\",
      \"sandbox_name\": \"${POD_NAME}\",
      \"sandbox_token\": \"smoke-test-jwt\",
      \"gateway_endpoint\": \"https://openshell.${NAMESPACE}.svc.cluster.local:8080\",
      \"policy\": {}
    }" \
    "localhost:${LOCAL_PORT}" openshell.supervisor.v1.Supervisor/ActivateSandbox 2>&1)
  ACTIVATE_END=$(date +%s%N)
  ACTIVATE_MS=$(( (ACTIVATE_END - ACTIVATE_START) / 1000000 ))

  cleanup_portforward

  SUCCESS=$(echo "$RESULT" | python3 -c "import json,sys; print(json.load(sys.stdin).get('success',False))" 2>/dev/null || echo "False")
  COMBINED=$(( CLAIM_MS + ACTIVATE_MS ))

  if [ "$SUCCESS" = "True" ]; then
    TOTAL_CLAIM=$((TOTAL_CLAIM + CLAIM_MS))
    TOTAL_ACTIVATE=$((TOTAL_ACTIVATE + ACTIVATE_MS))
    PASSED=$((PASSED + 1))
    STATUS_TAG="OK"
    [ "$COMBINED" -lt 2000 ] && STATUS_TAG="OK (<2s)" || STATUS_TAG="OK (>2s)"
    log "  Run $run: claim=${CLAIM_MS}ms  activate=${ACTIVATE_MS}ms  total=${COMBINED}ms  [$STATUS_TAG]"
  else
    fail "Run $run: ActivateSandbox failed: $(echo "$RESULT" | tr '\n' ' ')"
  fi

  # Cleanup claim
  kubectl -n "$NAMESPACE" delete sandboxclaim "$CLAIM_NAME" --wait=false &>/dev/null || true
  [ "$run" -lt "$RUNS" ] && sleep 3
done

log ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log "=== Results ==="

if [ "$PASSED" -gt 0 ]; then
  AVG_CLAIM=$((TOTAL_CLAIM / PASSED))
  AVG_ACTIVATE=$((TOTAL_ACTIVATE / PASSED))
  AVG_TOTAL=$((AVG_CLAIM + AVG_ACTIVATE))

  log ""
  log "  Runs:           $PASSED/$RUNS passed"
  log "  Avg claim:      ${AVG_CLAIM}ms"
  log "  Avg activate:   ${AVG_ACTIVATE}ms"
  log "  Avg total:      ${AVG_TOTAL}ms"
  log "  Target:         <2000ms"
  log ""

  if [ "$AVG_TOTAL" -lt 2000 ]; then
    log "  VERDICT: PASS (${AVG_TOTAL}ms avg, under 2s target)"
  else
    log "  VERDICT: ABOVE TARGET (${AVG_TOTAL}ms avg, target was 2s)"
    log "           Note: port-forward overhead adds ~100-200ms. In-cluster"
    log "           latency (gateway calling supervisor directly) will be lower."
  fi
else
  fail "All runs failed"
fi

log ""

if [ "$FAILURES" -gt 0 ]; then
  log "FAILURES: $FAILURES"
  exit 1
fi
