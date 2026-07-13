# Warm Pool gRPC PoC: Smoke Test Guide

Branch: `6113-warm-pool-grpc-poc`

## What this proves

Cold-starting a sandbox takes ~16.7s.
This PoC cuts that to **~1.9s** by pre-provisioning pods in a warm pool
and pushing identity via gRPC at claim time.

The key architectural decision (endorsed by the upstream OpenShell team)
is that warm pool pods start with an **unidentified supervisor**: no
gateway connection, no identity, no OPA policies. The supervisor is a
blank slate that only becomes a real sandbox when the gateway pushes
credentials after claim binding. This avoids the re-identification
problem entirely, since there is no stale identity to replace, and
prevents policies from becoming out-of-date between pool time and claim
time.

## How it works

### Cold start (today): ~16.7s

```
CLI request
  -> Gateway creates Sandbox CRD
    -> Operator provisions pod
      -> Image pull + init containers
        -> Supervisor starts, calls IssueSandboxToken
          -> Supervisor calls GetSandboxConfig
            -> OPA compilation
              -> ConnectSupervisor stream
                -> Ready
```

### Warm pool with gRPC activation (this PoC): ~1.9s

```
Pool provisioning (ahead of time, ~30-60s):
  SandboxTemplate ──> SandboxWarmPool ──> N pods with unidentified supervisor
  Each pod: supervisor --unidentified (gRPC:9090, /readyz:8080, no gateway connection)

Claim time (~1.9s):
  CLI request
    -> Gateway finds matching SandboxWarmPool    ─┐
    -> Gateway creates SandboxClaim               │ ~1.4s (operator reconciliation)
    -> Operator binds a warm pod to the claim     ─┘
    -> Gateway reads pod IP from claim status
    -> Gateway calls ActivateSandbox(gRPC)       ─┐
       * Pushes: sandbox_id, JWT, policy, endpoint│ ~0.5s (bootstrap)
       * Supervisor stores identity               │
       * Supervisor compiles OPA from policy       │
       * Supervisor connects back to gateway      ─┘
    -> Ready
```

### The identity pattern

The critical design choice is **"unidentified, then push"** rather than
"pre-identify, then re-bind" or "use annotations/labels":

1. **At pool time**: the supervisor process starts in `--unidentified`
   mode. It has no sandbox ID, no JWT, no OPA policies, and no gateway
   connection. It simply listens on gRPC port 9090 and serves a health
   check on port 8080. The Kubernetes readiness probe gates on `/readyz`,
   so the pod appears Ready in the SandboxWarmPool without needing any
   sandbox-specific configuration.

2. **At claim time**: the gateway creates a `SandboxClaim` that binds a
   warm pod. The operator reports the pod IP in the claim status. The
   gateway then calls `ActivateSandbox` on the supervisor's gRPC
   endpoint, pushing:
   - `sandbox_id` and `sandbox_name` (the identity)
   - `sandbox_token` (a gateway-minted JWT, no K8s SA token exchange)
   - `gateway_endpoint` (where the supervisor connects back to)
   - `policy` (the OPA policy configuration to compile)

3. **On activation**: the supervisor stores the identity, compiles OPA
   policies, and opens a `ConnectSupervisor` stream back to the gateway.
   From this point forward, the pod behaves identically to a cold-started
   sandbox.

This pattern avoids three problems that other approaches hit:

- **Env var injection bypass**: setting `spec.env` on a `SandboxClaim`
  causes the operator to provision a new pod instead of adopting a warm
  one. The gRPC push sidesteps this entirely.

- **Stale identity**: pre-baking identity or global policies at pool time
  means they can become out-of-date by claim time. The unidentified
  approach ensures the supervisor always gets fresh credentials and
  policies.

- **Re-identification complexity**: trying to re-bind an already-identified
  supervisor requires tearing down and rebuilding internal state. Starting
  from a clean slate is simpler and more reliable.

### Proto contract

```protobuf
service Supervisor {
  rpc ActivateSandbox(ActivateSandboxRequest) returns (ActivateSandboxResponse);
}

message ActivateSandboxRequest {
  string sandbox_id = 1;
  string sandbox_name = 2;
  string sandbox_token = 3;     // Gateway-minted JWT (no SA token exchange)
  string gateway_endpoint = 4;
  SandboxPolicy policy = 5;     // OPA rules compiled at activation, not pool time
}
```

## Try it on the shared test cluster

A pre-configured ROSA HCP 4.22.3 cluster is available with everything
already deployed (operator, OpenShell gateway, warm pool). You just need
the OpenShell CLI and cluster access.

### What you need

| Tool | Install |
|---|---|
| `oc` (OpenShift CLI) | [mirror.openshift.com](https://mirror.openshift.com/pub/openshift-v4/clients/ocp/latest/) |
| `openshell` CLI | [github.com/NVIDIA/OpenShell](https://github.com/NVIDIA/OpenShell/releases) |
| `grpcurl` (optional) | `brew install grpcurl` or `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest` |

### Step 1: Login and register the gateway

```shell
# Login to the test cluster
oc login -u admin -p '0p3nSh3ll-warm!' \
  https://api.warm-pool-rerun.hkz1.p3.openshiftapps.com:443 \
  --insecure-skip-tls-verify

# Extract mTLS certificates from the cluster
GATEWAY_DIR=~/.config/openshell/gateways/k8s/mtls
mkdir -p "$GATEWAY_DIR"
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.ca\.crt}' | base64 -d > "$GATEWAY_DIR/ca.crt"
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.tls\.crt}' | base64 -d > "$GATEWAY_DIR/tls.crt"
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.tls\.key}' | base64 -d > "$GATEWAY_DIR/tls.key"

# Register the gateway
openshell gateway add \
  --name k8s \
  --local \
  https://openshell-openshell.apps.rosa.warm-pool-rerun.hkz1.p3.openshiftapps.com

# Select it as default
openshell gateway select k8s
```

### Step 2: Verify warm pool is ready

```shell
kubectl -n openshell get sandboxwarmpool openshell-grpc-pool
# Should show READY=3
```

### Step 3: See the difference (the demo)

```shell
# Warm pool: should complete in ~2-3 seconds
time openshell sandbox create --name warm-demo --from base -- echo "hello from warm pool"

# Delete the warm pool to force cold start
kubectl -n openshell delete sandboxwarmpool openshell-grpc-pool

# Cold start: should take ~14-18 seconds
time openshell sandbox create --name cold-demo --from base -- echo "hello from cold start"

# Restore the warm pool
kubectl apply -f experiments/manifests/warm-pool-unidentified.yaml
```

Expected results: warm pool is 5-8x faster than cold start.

### Step 4: Interactive SSH (optional)

```shell
# Drop into a shell inside a warm pool sandbox
openshell sandbox create --name ssh-demo --from base
# You're now in a bash shell inside the sandbox
# Try: ls, whoami, cat /etc/hostname
# Exit with: exit
```

### Step 5: Run the automated smoke test (optional)

```shell
git clone https://github.com/rhuss/OpenShell.git && cd OpenShell
git checkout 6113-warm-pool-grpc-poc
./experiments/smoke-test.sh --skip-deploy --runs 5
```

Console: https://console-openshift-console.apps.rosa.warm-pool-rerun.hkz1.p3.openshiftapps.com

> **Note**: This cluster is temporary and will be torn down after the PoC
> evaluation. Do not rely on it for long-term testing.

## Prerequisites

| Requirement | Why | How to check |
|---|---|---|
| Kubernetes cluster | Needs Agent Sandbox operator | `kubectl get nodes` |
| Agent Sandbox operator | Provides SandboxWarmPool CRDs | `kubectl api-resources \| grep sandboxwarmpools` |
| OpenShell deployed | Gateway, TLS secrets, service account | `kubectl -n openshell get pods` shows `openshell-0` |
| `grpcurl` | Calls ActivateSandbox from your machine | `grpcurl --version` |
| `kubectl` port-forward | Reaches pod gRPC port from outside | `kubectl port-forward --help` |

Install `grpcurl` if missing:

```shell
# macOS
brew install grpcurl

# Linux
go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest
```

## Quick start (automated)

```shell
# Clone and switch to the PoC branch (skip if you already did this above)
git clone https://github.com/rhuss/OpenShell.git && cd OpenShell
git checkout 6113-warm-pool-grpc-poc

# Deploy template + pool, run 3 end-to-end tests
./experiments/smoke-test.sh

# Use a different namespace
./experiments/smoke-test.sh --namespace my-ns

# Skip deployment (if template/pool already exist)
./experiments/smoke-test.sh --skip-deploy

# More runs for statistical confidence
./experiments/smoke-test.sh --runs 10
```

The script deploys the SandboxTemplate and WarmPool, waits for pods,
then runs N end-to-end claim+activation cycles with timing.

## Manual walkthrough

### 1. Deploy the warm pool

```shell
# Apply the SandboxTemplate with unidentified supervisor
kubectl apply -f experiments/manifests/sandbox-template-unidentified.yaml

# Create a warm pool with 3 replicas
kubectl apply -f experiments/manifests/warm-pool-unidentified.yaml

# Watch pods come up
kubectl -n openshell get pods -w -l agents.x-k8s.io/warm-pool-sandbox
```

Wait until all pods show `1/1 Running`. This means:
- The init container copied the workspace
- The supervisor started in `--unidentified` mode
- The readiness probe (`/readyz`) is passing

### 2. Verify unidentified mode

```shell
# Check the process inside a pod
POD=$(kubectl -n openshell get pods -l agents.x-k8s.io/warm-pool-sandbox \
  -o jsonpath='{.items[0].metadata.name}')

kubectl -n openshell exec $POD -- ps -o args= -p 1
# Expected: /opt/openshell/bin/openshell-sandbox --unidentified --health-check --health-port 8080

# Check health endpoint
kubectl -n openshell exec $POD -- wget -qO- http://localhost:8080/readyz
# Expected: ok

# Check logs
kubectl -n openshell exec $POD -- cat /var/log/openshell.*.log
# Expected: lines mentioning "unidentified mode" and "gRPC activation server"
```

### 3. Test the gRPC endpoint directly

```shell
# Port-forward to a warm pod
kubectl -n openshell port-forward pod/$POD 9090:9090 &

# List the gRPC service (needs the proto file)
grpcurl -plaintext -import-path proto -proto supervisor.proto localhost:9090 list
# Expected: openshell.supervisor.v1.Supervisor

# Call ActivateSandbox
grpcurl -plaintext -import-path proto -proto supervisor.proto \
  -d '{
    "sandbox_id": "manual-test-001",
    "sandbox_name": "test-sandbox",
    "sandbox_token": "test-jwt-token",
    "gateway_endpoint": "https://openshell.openshell.svc.cluster.local:8080",
    "policy": {}
  }' \
  localhost:9090 openshell.supervisor.v1.Supervisor/ActivateSandbox
# Expected: { "success": true }

# Clean up
kill %1
```

### 4. Full end-to-end: claim + activate

```shell
# Create a SandboxClaim (this is what the gateway does internally)
CLAIM=manual-e2e-$(date +%s)
cat <<EOF | kubectl apply -f -
apiVersion: extensions.agents.x-k8s.io/v1beta1
kind: SandboxClaim
metadata:
  name: $CLAIM
  namespace: openshell
spec:
  warmPoolRef:
    name: openshell-grpc-pool
EOF

# Wait for Ready (should take ~1-1.5s)
kubectl -n openshell get sandboxclaim $CLAIM -w

# Once Ready, get the bound pod
POD_NAME=$(kubectl -n openshell get sandboxclaim $CLAIM \
  -o jsonpath='{.status.sandbox.name}')
POD_IP=$(kubectl -n openshell get sandboxclaim $CLAIM \
  -o jsonpath='{.status.sandbox.podIPs[0]}')
echo "Claimed pod: $POD_NAME at $POD_IP"

# Port-forward and activate
kubectl -n openshell port-forward pod/$POD_NAME 9090:9090 &
sleep 2

grpcurl -plaintext -import-path proto -proto supervisor.proto \
  -d "{
    \"sandbox_id\": \"$CLAIM\",
    \"sandbox_name\": \"$POD_NAME\",
    \"sandbox_token\": \"test-jwt\",
    \"gateway_endpoint\": \"https://openshell.openshell.svc.cluster.local:8080\",
    \"policy\": {}
  }" \
  localhost:9090 openshell.supervisor.v1.Supervisor/ActivateSandbox

# Clean up
kill %1
kubectl -n openshell delete sandboxclaim $CLAIM
```

### 5. Verify pool auto-replenishment

After each claim, the warm pool operator automatically creates a
replacement pod. Check that the pool refills:

```shell
kubectl -n openshell get sandboxwarmpool openshell-grpc-pool \
  -o jsonpath='{.status.readyReplicas}'
# Should return the original replica count after ~30-60s
```

## What to look for

### Passing results

| Check | Expected | Notes |
|---|---|---|
| Pod readiness | <1s from container start | Measures supervisor startup in unidentified mode |
| SandboxClaim to Ready | ~1.0-1.5s | Operator reconciliation, consistent across pod sizes |
| ActivateSandbox call | ~400-600ms | Includes OPA compile + gateway connection |
| Combined claim + activate | <2.0s | The target metric (SC-001) |
| Pool replenishment | Automatic | Operator creates replacement pods after claims |
| Cold-start fallback | Unchanged | No warm pool for an image = existing cold-start path |

### Known limitations of this PoC

- The `sandbox_token` is a placeholder. In production, the gateway mints
  a real JWT and the supervisor uses it for all subsequent RPCs.

- The `bootstrap_sandbox` function connects to the gateway and calls
  `GetSandboxConfig` and `ConnectSupervisor`. With test JWTs, these
  calls may fail silently (the function has graceful fallbacks). With
  real JWTs, the full bootstrap would add ~100-200ms for OPA compilation
  from the fetched policy.

- Port-forwarding adds ~100-200ms of overhead. The in-cluster latency
  (gateway calling supervisor directly via pod IP) would be lower.

- Each warm pool pod consumes real resources (CPU, memory, PVC).
  Production deployments will need pool sizing guidance.

## Cleanup

```shell
# Remove the warm pool and template
kubectl -n openshell delete sandboxwarmpool openshell-grpc-pool
kubectl -n openshell delete sandboxtemplate openshell-warm-unidentified

# Clean up any leftover claims
kubectl -n openshell delete sandboxclaim -l agents.x-k8s.io/warm-pool-sandbox

# The PVCs from warm pool pods persist. Delete manually if needed:
kubectl -n openshell get pvc | grep workspace-openshell-grpc | awk '{print $1}' | \
  xargs kubectl -n openshell delete pvc
```

## Building the supervisor image from source

If you want to build the PoC supervisor image from this branch instead
of using the pre-built `quay.io/rhuss/openshell-supervisor:warm-pool-poc`:

```shell
# Cross-compile for amd64 (from macOS or Linux)
# Requires: cargo-zigbuild, zig
PREBUILT_ARCH=amd64 tasks/scripts/stage-prebuilt-binaries.sh supervisor

# Build the container image
DOCKER_PLATFORM=linux/amd64 podman build --platform linux/amd64 \
  -f deploy/docker/Dockerfile.supervisor \
  -t your-registry/openshell-supervisor:warm-pool-poc .

# Push
podman push your-registry/openshell-supervisor:warm-pool-poc

# Deploy with your image
SUPERVISOR_IMAGE=your-registry/openshell-supervisor:warm-pool-poc \
  ./experiments/smoke-test.sh
```

## Measured results

### Milestone 1: gRPC activation only (2026-07-13)

```
Run 1: claim=1412ms  activate=464ms  total=1876ms
Run 2: claim=1373ms  activate=499ms  total=1872ms
Run 3: claim=1341ms  activate=526ms  total=1867ms

Average: claim=1375ms  activate=496ms  total=1872ms
Target:  <2000ms
Verdict: PASS
```

### Milestone 2: Full E2E with SSH via CLI (2026-07-13)

Warm pool (with full bootstrap: networking + SSH + process stack):
```
Run 1: 2.686s
Run 2: 2.716s
```

Cold start (no warm pool, same cluster with pre-pulled images):
```
Run 1: 14.000s
Run 2: 18.776s
Run 3: 16.140s
```

**Result: ~2.7s warm vs ~16.3s cold, a 6x improvement.**

The warm pool path includes CLI overhead (~0.5s for TLS handshake,
gateway routing, SSH relay setup) on top of the raw claim+activate
time measured in Milestone 1.
