# Implementation Plan: Warm Pool E2E with SSH (Milestone 2)

**Branch**: `6113-warm-pool-grpc-poc` | **Date**: 2026-07-13 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/003-warm-pool-e2e-ssh/spec.md`

## Summary

Extract the post-identity networking/SSH/process startup from `run_sandbox()` into shared code that `bootstrap_sandbox()` also calls after ActivateSandbox provides identity. This gives warm pool sandboxes full functional parity with cold-started sandboxes (SSH, proxy, OPA, entrypoint).

## Technical Context

**Language/Version**: Rust (edition 2021, workspace)
**Primary Crate**: `openshell-sandbox` (lib.rs)
**Key Dependencies**: `openshell-supervisor-network` (proxy/OPA), `openshell-supervisor-process` (SSH/entrypoint/netns)
**Target Platform**: Linux (sandbox pod), macOS (compile check)
**Performance Goal**: SSH shell within 3 seconds of CLI invocation (claim ~1.4s + activation + networking + SSH < 1.6s)
**Constraint**: Cold-start path (`run_sandbox()`) must remain identical in behavior

## Architecture Analysis: `run_sandbox()` Phases

The 650-line `run_sandbox()` function has these phases:

| Phase | Lines | Description | Warm Pool Needs? |
|-------|-------|-------------|-----------------|
| 1. OCSF context | 112-137 | Hostname, sandbox context | Already done in activation handler |
| 2. Sidecar detection | 139-160 | Sidecar topology check | Skip (warm pool = combined topology) |
| 3. Policy loading | 162-177 | OPA engine from gateway/sidecar | Already done in `bootstrap_sandbox()` |
| 4. UID/GID override | 179-210 | Process identity from env vars | **YES** (env vars in pod) |
| 5. Provider credentials | 213-294 | Fetch from gateway, credential state | Partially done, needs full version |
| 6. Proposals flag + PID | 296-310 | Agent proposals, entrypoint PID tracker | **YES** |
| 7. Network namespace | 317-322 | Create netns for proxy | **YES** |
| 8. Denial/activity channels | 329-356 | Event channels for aggregator | **YES** |
| 9. Networking | 358-386 | `run_networking()` - proxy, OPA | **YES** (core missing piece) |
| 10. Sidecar control | 388-460 | Sidecar server/handler | Skip (combined topology) |
| 11. Aggregators | 462-529 | Denial + activity flush tasks | **YES** |
| 12. Policy poll | 532-575 | Settings refresh loop | **YES** |
| 13. GCE metadata | 577-614 | Metadata loopback server | **YES** (if provider uses GCE) |
| 14. Process startup | 660-683 | `run_process()` - SSH, entrypoint | **YES** (core missing piece) |
| 15. Lifecycle | 685-737 | Wait for process exit | **YES** (block on signals) |

**Phases to extract**: 4, 5 (full version), 6, 7, 8, 9, 11, 12, 13, 14, 15
**Phases to skip**: 2, 10 (sidecar topology, not used in warm pool)
**Phases already done**: 1, 3 (OCSF context and OPA in activation handler)

## Implementation Approach

### Core Design: Shared `post_identity_bootstrap()` Function

Instead of creating a struct-based abstraction, extract a single async function that encapsulates phases 4-15:

```
async fn post_identity_bootstrap(ctx: BootstrapContext) -> Result<i32>
```

Where `BootstrapContext` is a struct holding all the parameters both paths need:

```rust
struct BootstrapContext {
    command: Vec<String>,
    workdir: Option<String>,
    timeout_secs: u64,
    interactive: bool,
    sandbox_id: String,
    sandbox_name: String,
    gateway_endpoint: String,
    policy: SandboxPolicy,
    opa_engine: Arc<OpaEngine>,
    retained_proto: Option<SandboxPolicy>,
    ssh_socket_path: Option<String>,
    inference_routes: Option<String>,
    ocsf_enabled: Arc<AtomicBool>,
    network_enabled: bool,
    process_enabled: bool,
    provider_credentials: ProviderCredentialState,
    provider_env: HashMap<String, String>,
}
```

**Cold-start path**: `run_sandbox()` builds `BootstrapContext` from its parameters (policy loading, provider env fetch, etc.) and calls `post_identity_bootstrap()`.

**Warm pool path**: `bootstrap_sandbox()` builds `BootstrapContext` from the ActivateSandbox request + `GetSandboxConfig` response and calls the same function.

### What Changes in `bootstrap_sandbox()`

Current `bootstrap_sandbox()` does:
1. Connect to gateway with JWT
2. Fetch sandbox config (GetSandboxConfig)
3. Compile OPA policies
4. Fetch provider environment
5. Spawn ConnectSupervisor session

After this change, it will additionally:
6. Build `BootstrapContext` from the above data
7. Call `post_identity_bootstrap()` which starts networking, SSH, process, and blocks until exit

### What Changes in `run_sandbox()`

`run_sandbox()` currently does everything inline. After refactoring:
1. Phases 1-3 remain inline (OCSF, sidecar detection, policy loading)
2. Phase 5 (provider env) stays inline (builds the full `ProviderCredentialState`)
3. Everything from phase 4 onward is replaced by building `BootstrapContext` and calling `post_identity_bootstrap()`

The sidecar topology path (phases 2, 10) stays in `run_sandbox()` since warm pool doesn't use it. `post_identity_bootstrap()` receives a flag indicating combined vs sidecar topology.

## Project Structure

### Files Changed

```text
crates/openshell-sandbox/src/
├── lib.rs                    # MODIFY: extract post_identity_bootstrap(), update run_sandbox() and bootstrap_sandbox()
```

### No New Files

The extraction is a refactoring within `lib.rs`. No new modules needed. `BootstrapContext` and `post_identity_bootstrap()` live in the same file.

## Implementation Phases

### Phase 1: Define BootstrapContext Struct

**Goal**: Create the shared parameter struct.

**Files**: `crates/openshell-sandbox/src/lib.rs`

Add `BootstrapContext` struct after the existing `BootstrapError` enum (~line 792). Include all fields from the analysis table. Both `run_sandbox()` and `bootstrap_sandbox()` will construct this struct.

### Phase 2: Extract post_identity_bootstrap()

**Goal**: Move phases 4-15 of `run_sandbox()` into a standalone function.

**Files**: `crates/openshell-sandbox/src/lib.rs`

1. Create `async fn post_identity_bootstrap(ctx: BootstrapContext) -> Result<i32>`
2. Move the following code blocks from `run_sandbox()`:
   - UID/GID override (lines 179-210)
   - Proposals flag + PID tracker (lines 296-310)
   - Network namespace creation (lines 317-322)
   - Denial/activity channels (lines 329-356)
   - `run_networking()` call (lines 358-386)
   - Sidecar control server (lines 388-460, gated on sidecar flag)
   - Aggregator spawn (lines 462-529)
   - Policy poll loop (lines 532-575)
   - GCE metadata server (lines 577-614)
   - Process policy resolution (line 616)
   - `run_process()` call (lines 660-683)
   - Process lifecycle (lines 685-737)
3. Replace the moved code in `run_sandbox()` with a `BootstrapContext` construction + `post_identity_bootstrap()` call

**Critical**: The sidecar topology paths (sidecar bootstrap, sidecar control server) need to stay accessible. Pass a `sidecar_bootstrap: Option<SidecarBootstrapData>` field in `BootstrapContext` or handle sidecar setup before calling `post_identity_bootstrap()`.

### Phase 3: Wire bootstrap_sandbox() to post_identity_bootstrap()

**Goal**: After activation, call the shared function to start networking/SSH/process.

**Files**: `crates/openshell-sandbox/src/lib.rs`

1. In `bootstrap_sandbox()`, after the existing steps (connect, fetch config, compile OPA, fetch provider env, spawn ConnectSupervisor):
   - Read the command from env var `OPENSHELL_SANDBOX_COMMAND` (or default to `/bin/bash`)
   - Read SSH socket path from env var
   - Build `BootstrapContext` with: sandbox_id, sandbox_name, JWT, gateway_endpoint, compiled OPA, fetched provider env, command, etc.
   - Set `network_enabled = true`, `process_enabled = true` (combined topology)
   - Call `post_identity_bootstrap(ctx)` which starts networking, SSH, and the entrypoint

2. Update `run_unidentified()`: instead of blocking on signals after activation, let `bootstrap_sandbox()` handle the full lifecycle (it now calls `post_identity_bootstrap()` which blocks until the process exits or SIGTERM).

### Phase 4: Update run_unidentified() Lifecycle

**Goal**: The unidentified supervisor stays alive via the process lifecycle, not manual signal handling.

**Files**: `crates/openshell-sandbox/src/lib.rs`

After activation, `bootstrap_sandbox()` now calls `post_identity_bootstrap()` which runs the process and blocks. The `run_unidentified()` function's `tokio::select!` on `activation_rx` should await the bootstrap result (which is the process exit code), not just log and block on signals.

Change the activation branch in `run_unidentified()` from:
```rust
Ok(sandbox_id) = activation_rx => {
    info!("Activation complete...");
    // manual signal wait
}
```
To:
```rust
Ok(exit_code) = activation_rx => {
    info!("Activation complete, process exited with {exit_code}");
}
```

Where the oneshot now carries the exit code from `post_identity_bootstrap()` instead of just the sandbox_id.

### Phase 5: Build, Deploy, and Test

**Goal**: Validate E2E on the cluster.

1. Cross-compile supervisor and gateway for amd64
2. Build and push Docker images to quay.io
3. Update SandboxTemplate to use new supervisor image
4. Recreate warm pool
5. Test: `openshell sandbox create --name warm-test --from base` drops into SSH
6. Test: cold start still works
7. Measure and compare: warm pool vs cold start timing

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| `run_sandbox()` refactoring breaks cold start | Extract mechanically (move code, don't rewrite). Run existing tests after each phase. |
| `BootstrapContext` has too many fields | Start with all fields, refactor to sub-structs if needed in future PR. PoC pragmatism > API elegance. |
| Provider env fetch differs between paths | Cold start fetches full credentials; warm pool fetches basic env. Use the same `ProviderCredentialState::from_environment()` in both. |
| Sidecar topology code entanglement | Keep sidecar paths in `run_sandbox()`, only extract combined-topology code into shared function. |
| Networking setup latency at activation time | Expected ~200-400ms for netns + proxy + nftables. Within the 3s budget. |

## Lessons to Capture

This refactoring will reveal:
- Which `run_sandbox()` parameters are truly needed vs artifacts of the monolithic design
- Whether the sidecar topology can also benefit from the shared bootstrap
- The actual latency breakdown of each bootstrap phase (for optimization in the targeted PR)
- Whether `BootstrapContext` is the right abstraction or if a `SandboxRuntime` struct with methods would be better

Document these in a `specs/003-warm-pool-e2e-ssh/LESSONS.md` after testing.
