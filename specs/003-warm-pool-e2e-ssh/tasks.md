# Tasks: Warm Pool E2E with SSH (Milestone 2)

**Input**: Design documents from `specs/003-warm-pool-e2e-ssh/`
**Prerequisites**: plan.md, spec.md

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Phase 1: Define BootstrapContext Struct

**Purpose**: Create the shared parameter struct that both cold-start and warm pool paths construct.

- [ ] T001 [US1] Define `BootstrapContext` struct in `crates/openshell-sandbox/src/lib.rs` (after `BootstrapError` at ~line 792). Fields: command (Vec<String>), workdir (Option<String>), timeout_secs (u64), interactive (bool), sandbox_id (String), sandbox_name (String), gateway_endpoint (String), policy (SandboxPolicy), opa_engine (Option<Arc<OpaEngine>>), retained_proto (Option<SandboxPolicy>), ssh_socket_path (Option<String>), inference_routes (Option<String>), ocsf_enabled (Arc<AtomicBool>), network_enabled (bool), process_enabled (bool), provider_credentials (ProviderCredentialState), provider_env (HashMap<String, String>), loaded_policy_origin (Option<String>). Include sidecar fields gated behind a `sidecar_bootstrap: Option<SidecarBootstrapData>` for the cold-start sidecar path.

**Checkpoint**: `cargo check -p openshell-sandbox` passes with the new struct.

---

## Phase 2: Extract post_identity_bootstrap()

**Purpose**: Move phases 4-15 of `run_sandbox()` into a shared function.

- [ ] T002 [US2] Create `async fn post_identity_bootstrap(ctx: BootstrapContext) -> Result<i32>` in `crates/openshell-sandbox/src/lib.rs`. Start with an empty body that returns `Ok(0)`.
- [ ] T003 [US2] Move UID/GID override code (lines 179-210 of `run_sandbox()`) into `post_identity_bootstrap()`. Read `OPENSHELL_SANDBOX_UID` and `OPENSHELL_SANDBOX_GID` from env, validate, and apply to `ctx.policy`.
- [ ] T004 [US2] Move proposals flag + entrypoint PID tracker setup (lines 296-310) into `post_identity_bootstrap()`.
- [ ] T005 [US2] Move network namespace creation (lines 317-322) into `post_identity_bootstrap()`. Guard with `ctx.network_enabled && !sidecar_network_enforcement`.
- [ ] T006 [US2] Move denial and activity channel creation (lines 329-356) into `post_identity_bootstrap()`.
- [ ] T007 [US2] Move `run_networking()` call (lines 358-386) into `post_identity_bootstrap()`. Pass policy, OPA engine, entrypoint PID, provider credentials, sandbox_id, endpoint, channels from `ctx`.
- [ ] T008 [US2] Move sidecar control server setup (lines 388-460) into `post_identity_bootstrap()`, gated on `ctx.sidecar_bootstrap.is_some()`.
- [ ] T009 [US2] Move denial and activity aggregator spawns (lines 462-529) into `post_identity_bootstrap()`.
- [ ] T010 [US2] Move policy poll loop spawn (lines 532-575) into `post_identity_bootstrap()`.
- [ ] T011 [US2] Move GCE metadata loopback server setup (lines 577-614) into `post_identity_bootstrap()`.
- [ ] T012 [US2] Move process startup (`run_process()` call, lines 616-683) and process lifecycle (lines 685-737) into `post_identity_bootstrap()`.
- [ ] T013 [US2] Replace the moved code in `run_sandbox()` with: construct `BootstrapContext` from existing local variables, call `post_identity_bootstrap(ctx).await`. Verify `run_sandbox()` is now ~50-80 lines (OCSF init, sidecar detection, policy loading, provider env fetch, BootstrapContext construction, call).

**Checkpoint**: `cargo check -p openshell-sandbox` passes. `cargo test -p openshell-sandbox` passes (all existing tests unchanged).

---

## Phase 3: Wire bootstrap_sandbox() to post_identity_bootstrap()

**Purpose**: After activation, start the full networking/SSH/process stack.

- [ ] T014 [US1] Update `bootstrap_sandbox()` in `crates/openshell-sandbox/src/lib.rs`: after the existing steps (connect, fetch config, compile OPA, fetch provider env, spawn ConnectSupervisor), build a `BootstrapContext` with: command from `OPENSHELL_SANDBOX_COMMAND` env var (default `/bin/bash`), workdir None, timeout 0 (no timeout), interactive true, sandbox_id/name/endpoint from activation request, policy and OPA engine from compiled policy, ssh_socket_path from `OPENSHELL_SSH_SOCKET_PATH` env, network_enabled true, process_enabled true, provider credentials from fetch, no sidecar bootstrap.
- [ ] T015 [US1] Call `post_identity_bootstrap(ctx).await` at the end of `bootstrap_sandbox()`. Change the return type from `Result<(), BootstrapError>` to `Result<i32, BootstrapError>` to propagate the process exit code.
- [ ] T016 [US1] Update the `ActivateSandbox` handler in `crates/openshell-sandbox/src/activation.rs`: change the oneshot channel from `oneshot::Sender<String>` to `oneshot::Sender<i32>` to carry the exit code from `post_identity_bootstrap()`.

**Checkpoint**: `cargo check -p openshell-sandbox` passes.

---

## Phase 4: Update run_unidentified() Lifecycle

**Purpose**: The supervisor stays alive via the process lifecycle, not manual signal handling.

- [ ] T017 [US1] Update `run_unidentified()` in `crates/openshell-sandbox/src/lib.rs`: change the `activation_rx` branch in `tokio::select!` to receive the exit code (i32) and return it. Remove the manual signal-wait block that was added in Milestone 1.
- [ ] T018 [US1] Update the activation handler to send the exit code: in `activation.rs`, after `bootstrap_sandbox()` succeeds, send the exit code through the oneshot.

**Checkpoint**: `cargo check -p openshell-sandbox` passes. `cargo test -p openshell-sandbox` passes.

---

## Phase 5: Build, Deploy, and Test on Cluster

**Purpose**: Validate the E2E flow on the live cluster.

- [ ] T019 [US1] Cross-compile supervisor for amd64: `PREBUILT_ARCH=amd64 tasks/scripts/stage-prebuilt-binaries.sh supervisor`
- [ ] T020 [P] [US2] Cross-compile gateway for amd64: `PREBUILT_ARCH=amd64 tasks/scripts/stage-prebuilt-binaries.sh gateway` (only if driver changes were made; otherwise reuse existing image)
- [ ] T021 [US1] Build and push supervisor Docker image: `podman build --platform linux/amd64 -f deploy/docker/Dockerfile.supervisor -t quay.io/rhuss/openshell-supervisor:warm-pool-poc-v3 .` and push
- [ ] T022 [US1] Update SandboxTemplate on the cluster to use the new supervisor image. Delete and recreate the warm pool to pick up new pods.
- [ ] T023 [US1] Test warm pool SSH: `openshell sandbox create --name warm-ssh-test --from base` and verify it drops into a shell within 3 seconds.
- [ ] T024 [US2] Test cold start regression: delete all warm pools, run `openshell sandbox create --name cold-test --from base`, verify normal cold-start behavior.
- [ ] T025 [US3] Side-by-side timing: run both cold and warm sandbox creates with `time`, record and compare.
- [ ] T026 [US1] Update `experiments/SMOKE-TEST.md` with the SSH demo instructions.

**Checkpoint**: SC-001 (SSH within 3s), SC-003 (cold-start unchanged), SC-004 (5x improvement).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1**: No dependencies, start immediately
- **Phase 2**: Depends on Phase 1 (BootstrapContext must exist)
- **Phase 3**: Depends on Phase 2 (post_identity_bootstrap must exist)
- **Phase 4**: Depends on Phase 3 (bootstrap_sandbox return type changes)
- **Phase 5**: Depends on Phase 4 (all code changes complete)

### Within Phase 2

Tasks T002-T012 are sequential (each moves a code block that depends on the previous). T013 depends on all of T002-T012.

### Parallel Opportunities

- T019 and T020 can run in parallel (different binaries)
- T023, T024 can run sequentially after T022

---

## Notes

- All warm pool CRD interaction uses kube-rs dynamic API (no generated types)
- The supervisor gRPC server port (9090) is separate from the health port (8080)
- Warm pool path assumes combined topology (no sidecar), so sidecar-specific code paths are skipped
- The `BootstrapContext` approach is intentionally flat (many fields) for PoC clarity. A future PR may refactor into sub-structs.
