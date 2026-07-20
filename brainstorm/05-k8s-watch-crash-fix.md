# Brainstorm: K8s Watch Stream Crash Fix

**Date:** 2026-07-10
**Status:** active
**Issue:** [#2211](https://github.com/NVIDIA/OpenShell/issues/2211)

## Problem Framing

The Kubernetes driver's `watch_sandboxes()` crashes with a file descriptor leak when non-gateway `Sandbox` resources exist in the namespace. This happens when the Agent Sandbox operator's `SandboxWarmPool` controller creates `Sandbox` objects that the gateway did not create and does not expect.

The gateway watch stream has no label selector, so it receives events for all `Sandbox` objects. Warm pool sandboxes lack the `openshell.ai/sandbox-id` label and the `sandbox-` name prefix, causing `sandbox_id_from_object()` to return an error. This error propagates as a stream failure, triggering a 2-second reconnect loop that leaks HTTP/2 connections until FD exhaustion crashes the process (13+ restarts observed).

The same issue affects `list_sandboxes()`, which also uses unfiltered `ListParams::default()`.

## Approaches Considered

### A: Minimal targeted fix (chosen)
Add a label selector (`openshell.ai/managed-by=openshell`) to both `watch_sandboxes()` and `list_sandboxes()` to filter at the API server level. Additionally, change the watch loop to skip (with `debug!` log) objects that fail `sandbox_from_object()` / `sandbox_id_from_object()` instead of sending errors to the channel.

- Pros: Minimal blast radius (one file, two functions). Reduces API server traffic. Defense-in-depth against future unknown object types.
- Cons: None significant.

### B: Extract shared helper
Same as A, but extract the label selector construction into a helper function to avoid duplication between `watch_sandboxes()` and `list_sandboxes()`.

- Pros: DRY for selector construction.
- Cons: Over-engineering for a one-liner format string used in two places.

### C: Change sandbox_from_object signature
Make `sandbox_from_object` return `Option<Sandbox>` instead of `Result<Sandbox, String>`, changing all callers to skip `None`.

- Pros: Treats unknown objects as normal at the type level.
- Cons: Larger change surface. Doesn't reduce API traffic. Loses error information for genuinely malformed objects.

## Decision

Approach A: minimal targeted fix with both label selector and defensive skip.

## Key Requirements

- Add `openshell.ai/managed-by=openshell` label selector to the `watcher::Config` in `watch_sandboxes()` (`driver.rs:717`)
- Add the same label selector to `ListParams` in `list_sandboxes()` (`driver.rs:488`)
- In the watch loop, all three event branches (Applied at line 741, Deleted at line 761, Restarted at line 782) must `debug!` log and continue instead of sending `Err` to the channel
- PR targets `origin` (fork) first, then submitted upstream to `NVIDIA/OpenShell`
- Unit tests for the skip behavior

## Open Questions

- Should the defensive skip use `debug!` or `warn!` level? (`debug!` seems right since the label filter should prevent this from happening in normal operation; it's only a fallback)
