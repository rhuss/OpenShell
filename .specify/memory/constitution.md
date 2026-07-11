<!--
Sync Impact Report
==================
Version change: (new) → 1.0.0
Added principles:
  - I. Safe Arithmetic in Bash Loops
  - II. Non-Zero Exit on CLI Error
Added sections:
  - Scope (defines applicability to experiment/tooling scripts)
  - Governance
Removed sections: none (initial constitution)
Templates requiring updates:
  - plan-template.md: ✅ no update needed (Constitution Check is generic)
  - spec-template.md: ✅ no update needed (no principle-specific refs)
  - tasks-template.md: ✅ no update needed (no principle-specific refs)
Follow-up TODOs: none
-->

# OpenShell Constitution

## Core Principles

### I. Safe Arithmetic in Bash Loops

All bash wait/polling loops MUST use `elapsed=$((elapsed + 1))` or
`(( ++elapsed ))` instead of `(( elapsed++ ))` when running under
`set -e` (or `set -euo pipefail`).

**Rationale**: Bash `(( expr ))` returns exit code 1 when the expression
evaluates to 0. Post-increment (`elapsed++`) returns the value *before*
incrementing, so when `elapsed=0`, `(( elapsed++ ))` evaluates to 0,
producing exit code 1. Under `set -e`, this terminates the script on the
first loop iteration. The substitution form `$((elapsed + 1))` avoids
this because it is a value expansion, not a command whose exit code is
checked. Pre-increment (`++elapsed`) also works because it returns the
value *after* incrementing (1, not 0).

### II. Non-Zero Exit on CLI Error

Usage/help functions called from error paths (unknown options, invalid
arguments) MUST exit with non-zero status. The recommended pattern is
`exit "${1:-0}"` in the usage function body, with `usage 0` for help
requests and `usage 1` for error call sites.

**Rationale**: When `usage` unconditionally calls `exit 0`, scripts that
receive invalid arguments appear to succeed. CI pipelines, wrapper
scripts, and experiment harnesses that check exit codes will miss the
failure, leading to silent misconfiguration.

## Scope

These principles apply to all bash scripts under `experiments/` and any
operator-facing shell tooling in the repository. They do not govern Rust,
Python, or other non-shell code.

## Governance

- This constitution supersedes conflicting practices within its scope.
- Amendments require documentation and a version bump.
- Version follows semantic versioning: MAJOR for removed/redefined
  principles, MINOR for new principles, PATCH for clarifications.
- Bot review triage may surface recurring patterns that become candidate
  principles (see triage workflow Step 14).

**Version**: 1.0.0 | **Ratified**: 2026-07-10 | **Last Amended**: 2026-07-10
