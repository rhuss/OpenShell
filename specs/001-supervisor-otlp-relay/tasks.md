# Tasks: Supervisor OTLP Relay

**Input**: Design documents from `specs/001-supervisor-otlp-relay/`

**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story. Phases 1-7 are from the initial implementation (completed). Phase 8 is the cross-driver evolution from brainstorm 03.

## Global Constraints

All tasks implicitly inherit these spec-wide invariants:

- **Buffer capacity default**: 4096 slots (shared between trace batches and OCSF events)
- **OTLP HTTP port**: 4318 (hardcoded, standard OTLP/HTTP default)
- **Bind address**: Always `127.0.0.1:4318` (all topologies)
- **Performance threshold**: 100 spans/sec sustained per sandbox
- **Delivery target**: Spans appear in collector within 30 seconds of emission
- **Latency tolerance**: Control plane commands within 2x baseline under telemetry load
- **Default protocol**: `http/protobuf` (set via `OTEL_EXPORTER_OTLP_PROTOCOL`)
- **Relay activation**: Opt-in, only when gateway confirms `telemetry_relay` capability
- **Sandbox resource attributes**: `openshell.sandbox.id`, `openshell.workspace.id`, `openshell.sandbox.policy`, `openshell.sandbox.user`, `openshell.sandbox.image`, `openshell.sandbox.driver`

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add proto definitions, dependencies, and constants needed by all stories.

- [x] T001 [P] Add `opentelemetry-proto` crate to workspace dependencies in `Cargo.toml` with features `gen-tonic`, `trace`, `with-serde`
- [x] T002 [P] Define `TelemetryData` message (sandbox_id, trace_data bytes, repeated ocsf_events bytes) and add as variant 5 in `SupervisorMessage.oneof payload` in `proto/openshell.proto`
- [x] T003 [P] Add `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` constants in `crates/openshell-core/src/sandbox_env.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core telemetry relay infrastructure that MUST be complete before ANY user story can be implemented. Provides the buffer, enrichment, session transport layer, and capability negotiation.

**CRITICAL**: No user story work can begin until this phase is complete.

- [x] T004 Create `otlp` module directory and public API surface (`TelemetryRelay`, `RelayConfig`) in `crates/openshell-supervisor-network/src/otlp/mod.rs`. **Interfaces**: `TelemetryRelay::new(config: RelayConfig, metadata: SandboxMetadata, session_tx: mpsc::Sender<SupervisorMessage>) -> Self`, `TelemetryRelay::start(bind_addr: SocketAddr) -> Result<RelayHandle, StartError>`, `RelayHandle::shutdown() -> impl Future` (drains buffer then stops)
- [x] T005 [P] Implement `RelayConfig` struct with `enabled`, `buffer_capacity` (default 4096), `enrichment_enabled` (default true), `ocsf_rate_limit: u32` (default 100 events/sec) fields in `crates/openshell-supervisor-network/src/otlp/mod.rs`
- [x] T006 [P] Implement shared bounded buffer (trace batches + OCSF events) with `TelemetryItem` enum (`Trace(Vec<u8>)`, `Ocsf(Vec<u8>)`), ring-buffer drop semantics, `AtomicU64` drop counter, `AtomicUsize` queue depth gauge, and forwarder drain method in `crates/openshell-supervisor-network/src/otlp/buffer.rs`. **Interfaces**: `TelemetryBuffer::new(capacity: usize) -> (TelemetrySender, TelemetryReceiver)`, `TelemetrySender::send_trace(data: Vec<u8>)`, `TelemetrySender::send_ocsf(data: Vec<u8>)`, `TelemetryBuffer::drop_count() -> u64`, `TelemetryBuffer::queue_depth() -> usize`, `TelemetryReceiver::drain() -> Vec<TelemetryItem>`
- [x] T007 [P] Implement span enrichment: deserialize `ExportTraceServiceRequest`, inject sandbox resource attributes, and always inject `openshell.telemetry.source: "agent"` regardless of enrichment toggle. Define `enum ContentType { Protobuf, Json }`. Re-serialize as protobuf in `crates/openshell-supervisor-network/src/otlp/enrichment.rs`. **Interfaces**: `enrich_spans(raw: &[u8], content_type: ContentType, attrs: &SandboxMetadata, enrichment_enabled: bool) -> Result<Vec<u8>, EnrichmentError>`
- [x] T008 Implement forwarder tokio task: drain buffer channel, route `TelemetryItem::Trace` to `TelemetryData.trace_data` and `TelemetryItem::Ocsf` to `TelemetryData.ocsf_events`, send via session tx channel using `try_send` (non-blocking). In `crates/openshell-supervisor-network/src/otlp/mod.rs`
- [x] T009 [P] Add `TelemetryData` variant handling to supervisor session outbound path in `crates/openshell-supervisor-process/src/supervisor_session.rs`
- [x] T010 [P] Handle incoming `TelemetryData` on gateway side: dedicated `TelemetryRelayExporter` for trace data, `tracing::info!(target: "ocsf_relay", ...)` for OCSF events. In `crates/openshell-server/src/telemetry_relay.rs` and `crates/openshell-server/src/supervisor_session.rs`
- [x] T011 [P] Add `capabilities` repeated-string field to `SupervisorHello` and `SessionAccepted` messages in `proto/openshell.proto`
- [x] T012 Implement capability negotiation: supervisor advertises `"telemetry_relay"` in `SupervisorHello.capabilities`, gateway confirms via `confirm_capabilities()`, supervisor gates `TelemetryData` sending on confirmation. In `crates/openshell-supervisor-process/src/supervisor_session.rs` and `crates/openshell-server/src/supervisor_session.rs`

**Checkpoint**: Telemetry transport layer ready. Buffer, enrichment, session send/receive, capability negotiation all functional.

---

## Phase 3: User Story 1 - Agent Developer Sees Traces (Priority: P1) MVP

**Goal**: An OTel-instrumented agent in a sandbox sees its traces in the configured collector within 30 seconds, with zero agent-side configuration.

**Independent Test**: Deploy an OTel-instrumented agent in any supported sandbox driver, run it, verify traces appear in the collector with `openshell.sandbox.id` and `openshell.telemetry.source` resource attributes.

### Implementation for User Story 1

- [x] T013 [P] [US1] Implement OTLP HTTP receiver: `hyper` server accepting `POST /v1/traces` with protobuf and JSON. Set `TCP_NODELAY` on accepted connections. In `crates/openshell-supervisor-network/src/otlp/receiver.rs`
- [x] T014 [P] [US1] Add `otel_env_vars()` function in `crates/openshell-supervisor-process/src/child_env.rs`
- [x] T015 [US1] Wire `TelemetryRelay` into `run_sandbox()` task tree, store `RelayHandle` for drain-before-teardown in `crates/openshell-sandbox/src/lib.rs`
- [x] T016 [US1] Implement drain-before-teardown: call `RelayHandle::shutdown()` before `drop(networking)` on sandbox teardown in `crates/openshell-sandbox/src/lib.rs`

**Checkpoint**: Agent traces flow end-to-end from sandbox to external collector. US1 acceptance scenarios 1, 2, 3 are satisfied.

---

## Phase 4: User Story 2 - Trace Correlation (Priority: P2)

**Goal**: Platform operators can query their trace backend by `openshell.sandbox.id` and find both agent-emitted spans and supervisor network spans together.

**Independent Test**: Run an agent that makes outbound HTTP requests, query the collector by sandbox ID, verify both agent and supervisor spans appear.

### Implementation for User Story 2

- [x] T017 [US2] Add W3C `traceparent` inject-if-missing on forward HTTP requests in `crates/openshell-supervisor-network/src/proxy.rs`, using `inject_traceparent_if_missing()` from `crates/openshell-otel/src/propagation.rs`
- [x] T018 [US2] Ensure supervisor's own infrastructure tracing includes `openshell.sandbox.id` as a resource attribute in `crates/openshell-sandbox/src/lib.rs`

**Checkpoint**: Agent spans and supervisor network spans are correlated by sandbox ID.

---

## Phase 5: User Story 3 - Non-Interference (Priority: P2)

**Goal**: Sandbox lifecycle commands stay within 2x baseline latency under 100 spans/sec sustained load.

**Note**: The non-blocking architecture is fully implemented in the foundational phase: bounded channel (T006), separate forwarder task with `try_send` semantics (T008), and queue depth gauge (T006). US3 has no additional implementation tasks. Its acceptance criteria are validated by quickstart Scenarios 3 and 4.

---

## Phase 6: User Story 4 - OCSF Event Relay (Priority: P3)

**Goal**: OCSF deny events generated inside sandboxes are forwarded to the gateway via the telemetry transport, with per-sandbox rate limiting.

**Independent Test**: Trigger a network deny event inside a sandbox, verify the OCSF event appears in the gateway's log output within 10 seconds.

### Implementation for User Story 4

- [x] T019 [P] [US4] Create `OcsfRelayLayer` implementing `tracing::Layer` that captures events on the `OCSF_TARGET` target in `crates/openshell-ocsf/src/tracing_layers/relay_layer.rs`
- [x] T020 [US4] Implement per-sandbox token bucket rate limiter (`RateLimitedOcsfSink`) with configurable rate and drop counter in `crates/openshell-supervisor-network/src/otlp/mod.rs`
- [ ] T021 [US4] Wire `OcsfRelayLayer` into supervisor tracing subscriber setup: install layer when relay is active, connect captured events through rate limiter to telemetry buffer in `crates/openshell-sandbox/src/lib.rs`

**Checkpoint**: OCSF events flow from sandbox to gateway alongside traces.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Graceful degradation, configurability, documentation.

- [x] T022 [P] Add enrichment disable configuration: when `enrichment_enabled = false`, relay forwards spans without sandbox metadata attributes but still injects `openshell.telemetry.source: "agent"`. In `crates/openshell-supervisor-network/src/otlp/enrichment.rs`
- [x] T023 [P] Add structured log emission of buffer metrics (drops, depth) on relay shutdown for FR-012 operator visibility. In `crates/openshell-supervisor-network/src/otlp/mod.rs`
- [x] T024 [P] Update architecture documentation for the telemetry relay subsystem in `architecture/`

---

## Phase 8: Cross-Driver Evolution (Always-Localhost)

**Purpose**: Evolve the relay from Docker/Podman-only to all four compute drivers. Change bind address from `10.200.0.1` to `127.0.0.1:4318`. Move OTEL env var injection from drivers to supervisor `child_env.rs`. Add K8s driver support.

### 8a. Fix bind address and receiver (FR-001, FR-011)

- [x] T025 [P] Modify `spawn_receiver()` in `crates/openshell-supervisor-network/src/otlp/receiver.rs` to accept either a pre-bound `TcpListener` (for netns topologies) or a `SocketAddr` (for direct bind). Add a second constructor or enum parameter.
- [x] T026 Update relay wiring in `crates/openshell-sandbox/src/lib.rs`: when netns exists, use `bind_tcp_in_netns(SocketAddr::new(Ipv4Addr::LOCALHOST, 4318))` to obtain a `TcpListener` and pass it to the receiver. When no netns (K8s combined, K8s sidecar, VM), bind `127.0.0.1:4318` directly. Gate relay startup on `network_enabled` (not just `process_enabled`) so the K8s sidecar starts the relay in the network container.
- [x] T027 Populate `SandboxMetadata` fields from actual sandbox context in `crates/openshell-sandbox/src/lib.rs`: `workspace_id` from the policy poll workspace watch channel (subscribe to updates), `user` from `resolved_process_identity` if available.

### 8b. Fix env var injection (FR-002)

- [x] T028 [P] Remove `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` from Docker driver's `build_environment_for_oci_user()` in `crates/openshell-driver-docker/src/lib.rs`
- [x] T029 [P] Remove `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_PROTOCOL` from Podman driver's `build_env()` in `crates/openshell-driver-podman/src/container.rs`
- [x] T030 [P] Wire `otel_env_vars("http://127.0.0.1:4318", "http/protobuf")` into entrypoint spawn in `crates/openshell-supervisor-process/src/process.rs`, conditional on relay being active (pass a `telemetry_relay_active: bool` parameter through `run_process`)
- [x] T031 [P] Wire `otel_env_vars()` into SSH session child env in `crates/openshell-supervisor-process/src/ssh.rs`, conditional on relay being active
- [x] T032 [P] Add OTEL env vars to Kubernetes agent container spec in `crates/openshell-driver-kubernetes/src/driver.rs` for both combined and sidecar topologies (agent is a separate container, not a supervisor child)

### 8c. Validation

- [ ] T033 Run quickstart.md validation scenarios end-to-end against a Docker or Podman sandbox with a configured OTLP collector. Verify traces arrive at `127.0.0.1:4318` with correct resource attributes.

**Checkpoint**: Relay works across all four compute drivers with uniform `127.0.0.1:4318` binding.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies, can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion, BLOCKS all user stories
- **User Stories (Phase 3-6)**: All depend on Foundational phase completion
  - US1 (Phase 3): No dependencies on other stories
  - US2 (Phase 4): No dependencies on other stories (can run in parallel with US1)
  - US3 (Phase 5): No implementation tasks (verified by architecture)
  - US4 (Phase 6): No dependencies on other stories (can run in parallel with US1/US2)
- **Polish (Phase 7)**: Depends on at least US1 (MVP) being complete
- **Cross-Driver (Phase 8)**: Depends on Phases 1-3 being complete. Can run in parallel with Phases 4-7.

### Parallel Opportunities

- All Setup tasks (T001-T003) can run in parallel
- Foundational tasks T005, T006, T007 can run in parallel (after T004)
- Foundational tasks T009, T010, T011 can run in parallel (independent crates)
- T012 depends on T011 (proto must exist before negotiation logic)
- US1 tasks T013-T014 can run in parallel (different crates)
- Cross-driver removal tasks T028, T029 can run in parallel
- Cross-driver env injection tasks T030, T031, T032 can run in parallel (different files)

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (proto, deps, constants)
2. Complete Phase 2: Foundational (buffer, enrichment, session transport, capability negotiation)
3. Complete Phase 3: User Story 1 (receiver, env vars, wiring, drain)
4. **STOP and VALIDATE**: Run quickstart Scenario 1 (agent traces in collector)
5. Deploy/demo if ready

### Cross-Driver Evolution

After MVP is stable:

1. Complete Phase 8a: Fix bind address and receiver (T025-T027)
2. Complete Phase 8b: Fix env var injection (T028-T032)
3. Complete Phase 8c: Validation (T033)

### Incremental Delivery

1. Setup + Foundational -> Transport layer ready
2. Add US1 -> Agent traces flow -> MVP complete
3. Add US2 -> Trace correlation via traceparent
4. Add US4 -> OCSF events relayed
5. Cross-Driver Evolution -> All topologies supported
6. Polish -> Docs, metrics exposure -> Ship-ready

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story is independently completable and testable
- Port 4318 is hardcoded (OTLP/HTTP standard default)
- The `try_send` pattern on the session channel is critical for US3 non-interference guarantee
- **Shared file**: `crates/openshell-sandbox/src/lib.rs` is modified by T015, T016, T018, T021, T026, T027 across phases. Later tasks should review prior modifications to avoid conflicts.
