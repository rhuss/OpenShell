# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

#!/usr/bin/env python3
"""
Minimal agent for the OTLP relay spike demo.

This script has NO OpenTelemetry code. Zero. It's a plain OpenAI SDK call.
OTel instrumentation is injected externally via:

  opentelemetry-instrument python3 demo-agent.py "your question"

The supervisor sets OTEL_EXPORTER_OTLP_ENDPOINT automatically.
The opentelemetry-instrument wrapper discovers the OpenAI instrumentor
and traces every LLM call without any code changes.
"""

import sys
import httpx2
from openai import OpenAI

prompt = " ".join(sys.argv[1:]) if len(sys.argv) > 1 else "Explain what OpenShell is in two sentences."

# Read some context (simulates a tool call)
context = open("/etc/os-release").read().strip()

# Make an LLM call via inference.local (OpenShell's inference proxy)
client = OpenAI(
    base_url="https://inference.local/v1",
    api_key="unused",
    http_client=httpx2.Client(proxy="http://10.200.0.1:3128", verify=False),
)

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[
        {"role": "system", "content": f"You are a helpful assistant. Context: {context}"},
        {"role": "user", "content": prompt},
    ],
    max_tokens=150,
)

print(response.choices[0].message.content)
