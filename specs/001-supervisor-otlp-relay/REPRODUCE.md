# Reproducing the OTLP Relay Spike

Step-by-step guide to recreate the OTLP relay spike validation from scratch.

## Prerequisites

- OpenShift 4.22+ cluster with admin access
- `oc` CLI authenticated to the cluster
- `openshell` CLI (v0.0.101+)
- `virtctl` CLI (for SAW VM access)
- `cargo-zigbuild` + `zig` (for cross-compilation)
- Helm 3
- An OpenAI API key (or other inference provider)

## Step 1: Install Cluster Operators

Install the three Red Hat operators needed for the tracing pipeline:

```bash
# RHOAI 3.4+ (provides MLflow operator)
oc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: redhat-ods-operator
---
apiVersion: operators.coreos.com/v1
kind: OperatorGroup
metadata:
  name: rhods-operator
  namespace: redhat-ods-operator
spec: {}
---
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: rhods-operator
  namespace: redhat-ods-operator
spec:
  channel: stable-3.4
  installPlanApproval: Automatic
  name: rhods-operator
  source: redhat-operators
  sourceNamespace: openshift-marketplace
EOF

# Red Hat Tempo
oc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: openshift-tempo-operator
---
apiVersion: operators.coreos.com/v1
kind: OperatorGroup
metadata:
  name: openshift-tempo-operator
  namespace: openshift-tempo-operator
spec:
  upgradeStrategy: Default
---
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: tempo-product
  namespace: openshift-tempo-operator
spec:
  channel: stable
  installPlanApproval: Automatic
  name: tempo-product
  source: redhat-operators
  sourceNamespace: openshift-marketplace
EOF

# Red Hat OpenTelemetry
oc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: openshift-opentelemetry-operator
---
apiVersion: operators.coreos.com/v1
kind: OperatorGroup
metadata:
  name: openshift-opentelemetry-operator
  namespace: openshift-opentelemetry-operator
spec:
  upgradeStrategy: Default
---
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: opentelemetry-product
  namespace: openshift-opentelemetry-operator
spec:
  channel: stable
  installPlanApproval: Automatic
  name: opentelemetry-product
  source: redhat-operators
  sourceNamespace: openshift-marketplace
EOF
```

Wait for all three CSVs to show `Succeeded`:
```bash
oc get csv -n redhat-ods-operator --no-headers | grep rhods
oc get csv -n openshift-tempo-operator --no-headers | grep tempo
oc get csv -n openshift-opentelemetry-operator --no-headers | grep opentelemetry
```

## Step 2: Create DataScienceCluster with MLflow

```bash
oc apply -f - <<EOF
apiVersion: datasciencecluster.opendatahub.io/v1
kind: DataScienceCluster
metadata:
  name: default-dsc
spec:
  components:
    dashboard:
      managementState: Managed
    mlflowoperator:
      managementState: Managed
EOF
```

Wait for MLflow operator:
```bash
oc get datasciencecluster default-dsc \
  -o jsonpath='{.status.conditions[?(@.type=="MLflowOperatorReady")].status}'
# Should return "True"
```

If it returns "Removed", the DSC was created with the v1 API which doesn't
recognize `mlflowoperator`. Patch it:
```bash
oc patch datasciencecluster default-dsc --type=merge \
  -p '{"spec":{"components":{"mlflowoperator":{"managementState":"Managed"}}}}'
```

## Step 3: Create Namespace and Deploy MLflow + Tempo

```bash
# Namespace with MLflow workspace label
oc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: openshell-agents
  labels:
    mlflow.opendatahub.io/workspace: "true"
EOF

# MLflow tracking server (RHOAI operator, SQLite backend)
oc apply -f - <<EOF
apiVersion: mlflow.opendatahub.io/v1
kind: MLflow
metadata:
  name: mlflow
  namespace: openshell-agents
spec:
  replicas: 1
  backendStoreUri: "sqlite:////mlflow/mlflow.db"
  artifactsDestination: "file:///mlflow/artifacts"
  serveArtifacts: true
  storage:
    accessModes: [ReadWriteOnce]
    resources:
      requests:
        storage: 10Gi
  resources:
    requests:
      cpu: 500m
      memory: 2Gi
    limits:
      memory: 4Gi
  workspaceLabelSelector:
    matchLabels:
      mlflow.opendatahub.io/workspace: "true"
EOF

# TempoMonolithic (PV storage, Jaeger UI)
oc apply -f - <<EOF
apiVersion: tempo.grafana.com/v1alpha1
kind: TempoMonolithic
metadata:
  name: openshell-tempo
  namespace: openshell-agents
spec:
  storage:
    traces:
      backend: pv
      size: 10Gi
  jaegerui:
    enabled: true
    route:
      enabled: true
      termination: edge
EOF
```

Wait for pods:
```bash
oc get pods -n redhat-ods-applications -l app=mlflow
oc get pods -n openshell-agents -l app.kubernetes.io/name=tempo
```

## Step 4: Create MLflow Experiment and OTel Collector

```bash
# Service account for collector -> MLflow auth
oc create sa openshell-collector-collector -n openshell-agents
oc adm policy add-cluster-role-to-user mlflow-operator-mlflow-integration \
  -z openshell-collector-collector -n openshell-agents

# Create MLflow "Default" experiment in the workspace
TOKEN=$(oc create token openshell-collector-collector -n openshell-agents --duration=1h)
MLFLOW_URL=$(oc get mlflow mlflow -n openshell-agents -o jsonpath='{.status.url}')
curl -sk -X POST "${MLFLOW_URL}/api/2.0/mlflow/experiments/create" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -H "X-MLflow-Workspace: openshell-agents" \
  -d '{"name": "Default"}'
# Returns: {"experiment_id": "1"}

# Long-lived token for the collector
COLLECTOR_TOKEN=$(oc create token openshell-collector-collector \
  -n openshell-agents --duration=87600h)
```

Deploy the collector with split-stream routing:
```bash
cat <<EOF | sed "s/COLLECTOR_TOKEN_PLACEHOLDER/$COLLECTOR_TOKEN/" | oc apply -f -
apiVersion: opentelemetry.io/v1beta1
kind: OpenTelemetryCollector
metadata:
  name: openshell-collector
  namespace: openshell-agents
spec:
  mode: deployment
  config:
    receivers:
      otlp:
        protocols:
          grpc:
            endpoint: 0.0.0.0:4317
    processors:
      filter/agent-only:
        error_mode: ignore
        traces:
          span:
            - 'resource.attributes["openshell.telemetry.source"] == nil'
            - 'resource.attributes["openshell.telemetry.source"] == ""'
    exporters:
      otlphttp/mlflow:
        traces_endpoint: "https://mlflow.redhat-ods-applications.svc.cluster.local:8443/v1/traces"
        tls:
          insecure_skip_verify: true
        headers:
          Authorization: "Bearer COLLECTOR_TOKEN_PLACEHOLDER"
          X-MLflow-Experiment-Id: "1"
          X-MLflow-Workspace: "openshell-agents"
      otlp/tempo:
        endpoint: "tempo-openshell-tempo.openshell-agents.svc.cluster.local:4317"
        tls:
          insecure: true
    service:
      pipelines:
        traces/all-to-tempo:
          receivers: [otlp]
          exporters: [otlp/tempo]
        traces/agent-to-mlflow:
          receivers: [otlp]
          processors: [filter/agent-only]
          exporters: [otlphttp/mlflow]
EOF
```

The `filter/agent-only` processor drops spans without `openshell.telemetry.source`
(infrastructure traces from the gateway). Agent traces (with `telemetry.source: "agent"`)
go to both Tempo and MLflow. `error_mode: ignore` is required because OTTL
comparisons against nil resource attributes can error without it.

## Step 5: Cross-Compile and Deploy the Relay-Enabled Build

From the `6115-supervisor-otlp-relay` branch of this repo:

```bash
# Cross-compile for Linux x86_64
cargo zigbuild --release --target x86_64-unknown-linux-gnu \
  -p openshell-server --features bundled-z3
cargo zigbuild --release --target x86_64-unknown-linux-gnu \
  -p openshell-sandbox
```

Deploy to the gateway host. For SAW (VM-based):
```bash
SSH_KEY="$HOME/.generated-ssh-keys/sandbox-ssh"

# Upload binaries
virtctl -n openshell-agents scp -i "$SSH_KEY" \
  target/x86_64-unknown-linux-gnu/release/openshell-gateway \
  cloud-user@vm/<SAW_NAME>:/tmp/
virtctl -n openshell-agents scp -i "$SSH_KEY" \
  target/x86_64-unknown-linux-gnu/release/openshell-sandbox \
  cloud-user@vm/<SAW_NAME>:/tmp/

# Replace and restart
virtctl -n openshell-agents ssh cloud-user@vm/<SAW_NAME> -i "$SSH_KEY" -c '
  systemctl --user stop openshell-gateway
  sudo mv /tmp/openshell-gateway /usr/local/bin/openshell-gateway
  sudo mv /tmp/openshell-sandbox /usr/local/bin/openshell-supervisor
  sudo chmod +x /usr/local/bin/openshell-gateway /usr/local/bin/openshell-supervisor
  systemctl --user start openshell-gateway
'
```

For the Docker driver, also replace the cached supervisor binary:
```bash
# Find the cached path
CACHED=$(find ~/.local/share/openshell/docker-supervisor/ -name openshell-sandbox)
cp /usr/local/bin/openshell-supervisor "$CACHED"
```

## Step 6: Configure Gateway OTLP Endpoint

Add the OTLP config to the gateway's `gateway.toml`:

```toml
[openshell.gateway.otlp]
endpoint = "http://<COLLECTOR_CLUSTER_IP>:4317"
```

Get the collector ClusterIP:
```bash
oc get svc -n openshell-agents \
  -l app.kubernetes.io/name=openshell-collector-collector \
  -o jsonpath='{.items[0].spec.clusterIP}'
```

Restart the gateway after config change.

## Step 7: Create Sandbox and Install Demo Agent

```bash
openshell gateway login --gateway-insecure
openshell --gateway-insecure sandbox create \
  --name test-sandbox \
  --from <SANDBOX_IMAGE> \
  --no-tty -- sh -c "sleep infinity"
```

Install the Python OTel auto-instrumentation in the sandbox:
```bash
# Install from the container's default netns (has network access)
# (sandbox netns can't reach pypi due to egress policy)
docker exec -u sandbox -e HOME=/sandbox <CONTAINER_ID> \
  pip3 install --user --break-system-packages \
    openai opentelemetry-api opentelemetry-sdk \
    opentelemetry-exporter-otlp-proto-http \
    opentelemetry-instrumentation-openai opentelemetry-distro
```

Deploy the demo agent script (`otel-spike/demo-agent.py`) into the sandbox.

## Step 8: Run the Smoke Test

```bash
# Verify env vars
openshell --gateway-insecure sandbox exec -n test-sandbox -- env | grep OTEL

# Verify relay is listening
openshell --gateway-insecure sandbox exec -n test-sandbox -- \
  curl -s -o /dev/null -w 'HTTP %{http_code}\n' \
  http://127.0.0.1:4318/v1/traces

# Run demo agent with auto-instrumentation
openshell --gateway-insecure sandbox exec -n test-sandbox -- \
  env OTEL_METRICS_EXPORTER=none OTEL_LOGS_EXPORTER=none \
  OTEL_SERVICE_NAME=demo-agent \
  PATH=/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin \
  opentelemetry-instrument python3 /sandbox/demo-agent.py \
  "What is the capital of France?"

# Check MLflow
TOKEN=$(oc create token openshell-collector-collector \
  -n openshell-agents --duration=1h)
curl -sk "${MLFLOW_URL}/api/2.0/mlflow/traces?experiment_ids=1" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-MLflow-Workspace: openshell-agents" | jq '.traces | length'

# Check Jaeger for enriched resource attributes
oc port-forward svc/tempo-openshell-tempo-jaegerui \
  -n openshell-agents 16686:16686 &
curl -s "http://localhost:16686/api/services" | jq '.data'
```

## Step 9: Performance Test

```bash
# Baseline (no load)
for i in $(seq 1 10); do
  time openshell --gateway-insecure sandbox exec -n test-sandbox -- echo ok
done

# Load generator (run inside sandbox netns via nsenter)
# See otel-spike/demo-agent.py for the span_ramp.py script
# Generates 100-10000 spans/sec sustained

# Under load
for i in $(seq 1 5); do
  time openshell --gateway-insecure sandbox exec -n test-sandbox -- echo ok
done
```

## Gotchas

- **`docker exec` vs sandbox netns**: `docker exec` enters the container's
  default network namespace, not the sandbox netns where `127.0.0.1:4318` is
  bound. Use `nsenter --net=/proc/<child_pid>/ns/net` or
  `openshell sandbox exec` instead.

- **Stale ClusterIPs after cluster restart**: The gateway's
  `[openshell.gateway.otlp]` endpoint uses a ClusterIP. After a cluster
  restart, verify the IP hasn't changed and restart the gateway if needed.

- **MLflow workspace label**: The namespace must have
  `mlflow.opendatahub.io/workspace: "true"` label. Without it, MLflow returns
  404 for all API calls with "Workspace not found".

- **Keycloak `basic` scope**: The `openshell-cli` Keycloak client needs the
  `basic` scope assigned for the OIDC token to include the `sub` claim. The
  gateway rejects tokens without `sub`.

- **Collector `error_mode: ignore`**: Required for the filter processor's OTTL
  conditions. Without it, spans with missing resource attributes cause errors
  instead of being filtered.

- **pip install inside sandbox netns**: The sandbox egress policy blocks
  pypi.org. Install packages via `docker exec` (container's default netns which
  has Docker network access), not via `nsenter` into the sandbox netns.

- **Docker driver supervisor cache**: The Docker driver caches the supervisor
  binary by digest. Replacing the gateway binary is not enough. Find and
  replace the cached binary under
  `~/.local/share/openshell/docker-supervisor/sha256-*/openshell-sandbox`.
