# Tasks: Warm Pool gRPC-Push RFC

**Feature**: Warm Pool Integration with Always-On Supervisor (gRPC-Push)
**Branch**: `6111-warm-pool-feasibility`
**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Phase 1: Setup

- [X] T001 Create RFC directory and initialize from template at rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 2: Foundational (RFC Frontmatter and Shared Sections)

- [X] T002 Write RFC frontmatter (authors, state, links) and Summary section in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T003 Write Motivation section with shared feasibility study measurements in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T004 Write Non-goals section (dynamic pool promotion, non-K8s drivers, code implementation) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 3: User Story 1 - Sub-Second Provisioning via gRPC-Push (P1)

- [X] T005 [US1] Write Proposal subsection: Architecture Overview with Mermaid diagram (gateway -> claim -> gRPC push -> supervisor) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T006 [US1] Write Proposal subsection: Always-On Supervisor with Two-Tier OPA (global at pool time, sandbox-specific at claim time) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T007 [US1] Write Proposal subsection: gRPC Identity Push Protocol (ActivateSandbox endpoint, mTLS, 2s timeout, retry, fallback) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 4: User Story 2 - Gateway-Managed Pool Lifecycle (P1)

- [X] T008 [US2] Write Proposal subsection: Gateway-Managed Pool Lifecycle (config-driven SandboxTemplate + SandboxWarmPool reconciliation, startup behavior, scaling, cleanup) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 5: User Story 3 - Multi-Image Pool Management (P2)

- [X] T009 [US3, US4] Write Proposal subsection: Multi-Image Pool Routing (pool-per-image config, image matching, cold-start fallback for unknown images AND pool exhaustion per US4/FR-007) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 6: User Story 5 - Comparison and Cross-Reference (P2)

- [X] T010 [US5] Write Proposal subsection: Comparison with Annotation-Based Approach (table covering latency, complexity, security, OPA, upstream alignment, resource cost) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T011 [US5] Write Alternatives section (annotation-based link, ConfigMap-based, gateway-native pool, do nothing) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 7: Remaining RFC Sections

- [X] T012 Write Implementation Plan section (phased work items) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T013 Write Risks section (new gRPC surface, gateway complexity, unvalidated approach, resource cost, edge cases: outdated supervisor binary via OnReplenish, K8s API connectivity loss during reconciliation, mTLS cert expiry) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T014 Write Prior Art section (craig-kindo GKE validation, agent-sandbox#1118, feasibility study) in rfc/NNNN-warm-pool-grpc-push/README.md
- [X] T015 Write Open Questions section and Future Extensions (dynamic pool promotion) in rfc/NNNN-warm-pool-grpc-push/README.md

## Phase 8: Cross-Reference Update

- [X] T016 Add Alternatives section to existing annotation RFC at rfc/NNNN-warm-pool-feasibility/README.md with link to gRPC-push RFC and comparison table

## Phase 9: Polish

- [X] T017 Review both RFCs for consistent terminology, fix cross-reference links, verify Mermaid diagrams render

## Dependencies

```
T001 --> T002, T003, T004 (setup before content)
T002, T003, T004 --> T005..T009 (foundational before stories)
T005, T006, T007 --> T010, T011 (proposal before comparison)
T010, T011 --> T012..T015 (comparison before remaining sections)
T012..T015 --> T016 (new RFC complete before cross-referencing)
T016 --> T017 (cross-reference before polish)
```

## Parallel Opportunities

- T002, T003, T004 can run in parallel (independent foundational sections)
- T005, T006, T007 can run in parallel (independent proposal subsections)
- T008, T009 can run in parallel (independent proposal subsections)
- T010, T011 can run in parallel (comparison and alternatives)
- T012, T013, T014, T015 can run in parallel (independent remaining sections)

## Implementation Strategy

**MVP**: T001-T007 (setup + summary/motivation + core proposal with architecture, supervisor, and gRPC protocol). This delivers the core RFC content that differentiates the gRPC-push approach.

**Incremental delivery**: Each phase adds a complete RFC section. The RFC is reviewable after Phase 7. Phase 8-9 add cross-references and polish.

## Summary

- **Total tasks**: 17
- **User Story tasks**: 7 (US1: 3, US2: 1, US3: 1, US5: 2)
- **Parallel opportunities**: 5 groups
- **Independent test**: Each phase produces a complete, reviewable RFC section
