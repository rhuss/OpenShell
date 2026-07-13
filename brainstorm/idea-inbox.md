# Idea Inbox

Ideas captured from code reviews for future brainstorming.

### supervisor-grpc-authentication

- **Source**: triage
- **Date**: 2026-07-11
- **Reference**: PR #13 (6113-warm-pool-grpc-poc)
- **Summary**: The supervisor's ActivateSandbox gRPC endpoint accepts calls without authentication. Any network-reachable pod can push arbitrary identity and policy. Server-side mTLS with client certificate verification is needed for Milestone 2.

> devin-ai-integration[bot]: "The supervisor's ActivateSandbox gRPC endpoint listens on port 9090 with no authentication or authorization. Any network-reachable entity that can connect to the warm pool pod's IP can call ActivateSandbox and push arbitrary sandbox identity and policy configuration. The AtomicBool idempotency guard prevents a second activation but does not verify the caller's identity."
