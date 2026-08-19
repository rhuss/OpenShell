#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Setup the Python demo agent inside the sandbox.
# Run this once after the sandbox is created.
#
# Usage: bash setup-demo-agent.sh [sandbox-name]

set -euo pipefail

SANDBOX_NAME="${1:-openshell-saw}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSH_KEY="$HOME/.generated-ssh-keys/sandbox-ssh"

echo "Setting up demo agent in sandbox '$SANDBOX_NAME'..."

# Find the VM and sandbox container
VM_SSH="virtctl -n openshell-agents ssh cloud-user@vm/${SANDBOX_NAME} -i $SSH_KEY -t -oStrictHostKeyChecking=no -t -oUserKnownHostsFile=/dev/null"

# Get container ID and child PID
SANDBOX_ID=$($VM_SSH -c 'docker ps --filter "name=openshell" --format "{{.ID}}" | head -1' 2>/dev/null)
CHILD_PID=$($VM_SSH -c "docker exec $SANDBOX_ID pgrep -f 'sleep infinity' | head -1" 2>/dev/null)

echo "Container: $SANDBOX_ID, Child PID: $CHILD_PID"

# Install Python packages inside the sandbox netns
echo "Installing OTel SDK and OpenAI instrumentation..."
$VM_SSH -c "
docker exec $SANDBOX_ID nsenter --net=/proc/$CHILD_PID/ns/net -- \
  su -s /bin/sh sandbox -c '
    export HOME=/sandbox
    export PATH=/sandbox/.local/bin:\$PATH
    export NODE_TLS_REJECT_UNAUTHORIZED=0
    pip3 install --user --break-system-packages -q \
      openai \
      opentelemetry-api \
      opentelemetry-sdk \
      opentelemetry-exporter-otlp-proto-http \
      opentelemetry-instrumentation-openai
  '
" 2>/dev/null

# Copy the demo agent script into the sandbox
echo "Deploying demo-agent.py..."
$VM_SSH -c "
docker exec -u sandbox -e HOME=/sandbox $SANDBOX_ID sh -c 'cat > /sandbox/demo-agent.py'" < "$SCRIPT_DIR/demo-agent.py" 2>/dev/null

# Verify
echo "Verifying setup..."
$VM_SSH -c "
docker exec $SANDBOX_ID nsenter --net=/proc/$CHILD_PID/ns/net -- \
  su -s /bin/sh sandbox -c '
    export HOME=/sandbox
    export PATH=/sandbox/.local/bin:\$PATH
    python3 -c \"import openai; from opentelemetry.instrumentation.openai import OpenAIInstrumentor; print(\\\"OK: openai + otel instrumentation ready\\\")\"
  '
" 2>/dev/null

echo ""
echo "Setup complete. Run the demo agent with:"
echo "  openshell --gateway-insecure sandbox exec $SANDBOX_NAME -- python3 /sandbox/demo-agent.py"
echo ""
echo "Or from the sandbox shell:"
echo "  python3 /sandbox/demo-agent.py \"What is the capital of France?\""
