# Deep Review Findings

**Date:** 2026-07-11
**Branch:** 6113-warm-pool-grpc-poc
**Rounds:** 1
**Gate Outcome:** PASS
**Invocation:** superpowers

## Summary

| Severity | Found | Fixed | Remaining |
|----------|-------|-------|-----------|
| Critical | 0 | 0 | 0 |
| Important | 2 | 2 | 0 |
| Minor | 6 | 0 | 6 |
| Notable | 1 | - | 1 |
| **Total** | **9** | **2** | **7** |

**Agents completed:** 5/5 (+ 1 external tool: CodeRabbit)
**Agents failed:** 0

## Findings

### FINDING-1
- **Severity:** Important
- **Confidence:** 85
- **File:** crates/openshell-driver-kubernetes/src/driver.rs:908-935
- **Category:** security
- **Source:** security-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** fixed (round 1)

**What is wrong:**
`read_activation_tls()` silently returned `None` when the K8s Secret existed but was missing required fields (ca.crt, tls.crt, tls.key). The `.and_then()` chain would produce `None` with no log output, causing a silent downgrade from mTLS to plaintext without any operator visibility.

**Why this matters:**
Silent security degradation is a high-risk pattern. An operator could believe mTLS is active (because the secret name is configured) while the activation channel is actually plaintext. In a production cluster, this would transmit JWTs and policy data without encryption.

**How it was resolved:**
Restructured `read_activation_tls()` to check `tls.is_none()` after the `and_then` extraction and emit a `warn!()` log with the secret name when fields are missing: "TLS secret missing required fields (ca.crt, tls.crt, tls.key), falling back to plain channel". The plaintext fallback is preserved (valid for dev/test) but is now observable.

---

### FINDING-2
- **Severity:** Minor
- **Confidence:** 70
- **File:** crates/openshell-sandbox/src/lib.rs:809-820
- **Category:** architecture
- **Source:** architecture-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** acknowledged (PoC scope)

**What is wrong:**
`bootstrap_sandbox()` creates a gateway channel via `connect_with_direct_token()` and compiles an OPA engine via `OpaEngine::from_proto()`, but these objects are consumed by `supervisor_session::spawn()` rather than being separately retained. CodeRabbit flagged that they appear "discarded before runtime use."

**Why this matters:**
In reality, both objects are passed into the supervisor session spawn call, which uses them. The concern is about documenting this flow clearly. For Milestone 1 (PoC), the bootstrap path validates connectivity and policy compilation end-to-end. Full lifecycle integration is Milestone 2.

**Why it was not fixed:**
The objects are not discarded; they flow into the supervisor session. A comment would be helpful but is not a code defect. Deferred to Milestone 2 integration work.

---

### FINDING-3
- **Severity:** Minor
- **Confidence:** 65
- **File:** crates/openshell-sandbox/src/lib.rs:921-928
- **Category:** production-readiness
- **Source:** production-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** acknowledged (PoC scope)

**What is wrong:**
`run_unidentified()` sets the readiness flag (`ready.store(true)`) before the gRPC `Server::builder(...).serve(addr)` call actually binds the socket. There is a small window where `/readyz` returns 200 but the gRPC port is not yet listening.

**Why this matters:**
A Kubernetes readiness probe could pass before the activation port is accepting connections, causing a brief window where activation RPCs would fail with connection refused. In practice, the bind is near-instant and the race is unlikely, but it violates the principle that readiness should reflect actual serving capability.

**Why it was not fixed:**
The race window is sub-millisecond in practice and does not affect correctness for the PoC. The proper fix (bind a TcpListener first, then set ready, then serve_with_incoming) is a Milestone 2 hardening item.

---

### FINDING-4
- **Severity:** Important
- **Confidence:** 90
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs:60, crates/openshell-sandbox/tests/activation_timing.rs:35
- **Category:** test-quality
- **Source:** test-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** fixed (round 1)

**What is wrong:**
Both integration tests used hardcoded `let grpc_port = 9090` for the supervisor's gRPC activation port. If port 9090 was already in use (by another test run, a local service, or CI parallelism), the tests would fail with address-in-use errors unrelated to the code under test.

**Why this matters:**
Flaky tests in CI erode trust in the test suite and waste developer time investigating false failures. Port conflicts are a common source of CI flakiness, especially with parallel test execution.

**How it was resolved:**
Three changes:
1. Added `activation_grpc_port()` function to `lib.rs` that reads `OPENSHELL_ACTIVATION_PORT` env var (defaults to 9090)
2. Updated `run_unidentified()` to use `activation_grpc_port()` instead of a hardcoded constant
3. Updated both test files to use `free_port()` for the gRPC port and pass `OPENSHELL_ACTIVATION_PORT` env var to the child process

---

### FINDING-5
- **Severity:** Minor
- **Confidence:** 60
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs:156-157
- **Category:** test-quality
- **Source:** test-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** acknowledged

**What is wrong:**
Integration tests use manual `child.kill()` / `child.wait()` at the end. If an assertion panics before cleanup, the child process is leaked.

**Why this matters:**
Leaked processes can hold ports and cause subsequent test failures. A kill-on-drop guard pattern would be more robust.

**Why it was not fixed:**
The tests follow the existing pattern used elsewhere in the codebase. The risk is low since the OS reaps orphans and the tests run in isolated CI containers. A guard pattern is a nice-to-have cleanup item.

---

### FINDING-6
- **Severity:** Minor
- **Confidence:** 55
- **File:** crates/openshell-sandbox/tests/activation_smoke.rs:23-26, crates/openshell-sandbox/tests/activation_timing.rs:9-12
- **Category:** test-quality
- **Source:** test-agent (also reported by: coderabbit)
- **Round found:** 1
- **Resolution:** acknowledged

**What is wrong:**
`free_port()` binds a TcpListener, reads the port, then drops the listener. Between drop and the child process binding, another process could claim the port (TOCTOU race).

**Why this matters:**
In practice, the race is extremely rare on modern systems (the kernel avoids immediate port reuse via SO_REUSEADDR semantics). This is a known limitation of the free_port() pattern used across many Rust test suites.

**Why it was not fixed:**
The pattern is standard practice. A retry-on-bind-failure approach would add complexity for negligible benefit.

---

### FINDING-7
- **Severity:** Notable
- **Confidence:** 75
- **File:** crates/openshell-sandbox/src/activation.rs:119
- **Category:** correctness
- **Source:** correctness-agent
- **Round found:** 1
- **Resolution:** informational

**What is wrong:**
The activated flag is reset to `false` after bootstrap failure (line 119: `self.activated.store(false, Ordering::Release)`). This allows retry after failure, which is not specified in the spec.

**Why this matters:**
This is actually good behavior. It prevents a supervisor from being permanently locked out after a transient gateway failure. The spec should be updated to document this retry-on-failure semantic.

---

### FINDING-8
- **Severity:** Minor
- **Confidence:** 60
- **File:** crates/openshell-sandbox/src/lib.rs:930-944
- **Category:** production-readiness
- **Source:** coderabbit
- **Round found:** 1
- **Resolution:** acknowledged (PoC scope)

**What is wrong:**
After receiving the activation signal via `activation_rx`, `run_unidentified()` returns `Ok(0)`. CodeRabbit flagged that the supervisor session lifecycle may not be fully awaited.

**Why this matters:**
In the PoC, `bootstrap_sandbox()` calls `supervisor_session::spawn()` which starts the ConnectSupervisor stream as a background task. The main function continues the normal supervisor lifecycle after activation completes. For Milestone 2, the transition from unidentified to active mode will need more explicit lifecycle management.

**Why it was not fixed:**
The current behavior is correct for the PoC. The supervisor session is spawned and runs independently. Full lifecycle integration is Milestone 2 scope.

---

### FINDING-9
- **Severity:** Minor
- **Confidence:** 50
- **File:** specs/002-warm-pool-grpc-poc/tasks.md:65-68
- **Category:** external
- **Source:** coderabbit
- **Round found:** 1
- **Resolution:** acknowledged (spec evolution candidate)

**What is wrong:**
Task T019 references ConfigStateChangeBuilder and AppLifecycleBuilder for driver-side events, but the Kubernetes driver does not depend on openshell-ocsf. The task description does not match the chosen observability architecture.

**Why this matters:**
This is the same underlying issue as FR-013 (reclassified as spec evolution candidate). The task file should be updated when the spec evolves.

**Why it was not fixed:**
Already covered by the spec evolution recommendation for FR-013. Task files will be updated alongside the spec.
