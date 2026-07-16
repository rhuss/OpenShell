// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "e2e")]

//! E2E tests for workspace resource lifecycle.
//!
//! Covers:
//! - Workspace CRUD
//! - Provider creation scoped to a workspace
//! - Workspace isolation (resources in one workspace are invisible in another)
//! - `--all-workspaces` listing
//! - Deletion guard (workspace cannot be deleted while resources exist)
//! - Successful deletion after resource cleanup

use std::process::Stdio;

use openshell_e2e::harness::binary::{openshell_bin, openshell_cmd};
use openshell_e2e::harness::output::strip_ansi;

const WORKSPACE: &str = "lifecycle-test";
const PROVIDER: &str = "lifecycle-prov";

struct CliResult {
    output: String,
    success: bool,
}

async fn run_cli(args: &[&str]) -> CliResult {
    let mut cmd = openshell_cmd();
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd.output().await.expect("spawn openshell command");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}{stderr}");

    CliResult {
        output: strip_ansi(&combined),
        success: output.status.success(),
    }
}

struct WorkspaceCleanup;

impl Drop for WorkspaceCleanup {
    fn drop(&mut self) {
        let bin = openshell_bin();
        let _ = std::process::Command::new(&bin)
            .args([
                "provider",
                "delete",
                PROVIDER,
                "--workspace",
                WORKSPACE,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::process::Command::new(&bin)
            .args(["workspace", "delete", WORKSPACE])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[tokio::test]
async fn workspace_full_crud_lifecycle() {
    let _cleanup = WorkspaceCleanup;

    // 1. Create workspace.
    let res = run_cli(&["workspace", "create", "--name", WORKSPACE]).await;
    assert!(res.success, "workspace create failed: {}", res.output);

    // 2. Verify workspace exists.
    let res = run_cli(&["workspace", "get", WORKSPACE]).await;
    assert!(res.success, "workspace get failed: {}", res.output);
    assert!(
        res.output.contains(WORKSPACE),
        "workspace get output should contain workspace name: {}",
        res.output
    );

    // 3. Create provider in the workspace.
    let res = run_cli(&[
        "provider",
        "create",
        "--name",
        PROVIDER,
        "--type",
        "github",
        "--runtime-credentials",
        "--workspace",
        WORKSPACE,
    ])
    .await;
    assert!(
        res.success,
        "provider create in workspace failed: {}",
        res.output
    );

    // 4. List providers in workspace — should see our provider.
    let res = run_cli(&["provider", "list", "--workspace", WORKSPACE]).await;
    assert!(res.success, "provider list failed: {}", res.output);
    assert!(
        res.output.contains(PROVIDER),
        "workspace-scoped list should show the provider: {}",
        res.output
    );

    // 5. List providers in default workspace — should NOT see our provider.
    let res = run_cli(&["provider", "list"]).await;
    assert!(res.success, "provider list (default) failed: {}", res.output);
    assert!(
        !res.output.contains(PROVIDER),
        "default workspace should not contain the workspace-scoped provider: {}",
        res.output
    );

    // 6. List providers with --all-workspaces — should see our provider.
    let res = run_cli(&["provider", "list", "--all-workspaces"]).await;
    assert!(
        res.success,
        "provider list --all-workspaces failed: {}",
        res.output
    );
    assert!(
        res.output.contains(PROVIDER),
        "--all-workspaces should include the workspace-scoped provider: {}",
        res.output
    );

    // 7. Attempt workspace deletion — should fail because provider exists.
    let res = run_cli(&["workspace", "delete", WORKSPACE]).await;
    assert!(
        !res.success,
        "workspace delete should fail while resources exist: {}",
        res.output
    );
    assert!(
        res.output.contains("still contains resources"),
        "error should mention blocking resources: {}",
        res.output
    );

    // 8. Delete the provider.
    let res = run_cli(&[
        "provider",
        "delete",
        PROVIDER,
        "--workspace",
        WORKSPACE,
    ])
    .await;
    assert!(res.success, "provider delete failed: {}", res.output);

    // 9. Workspace deletion should now succeed.
    let res = run_cli(&["workspace", "delete", WORKSPACE]).await;
    assert!(
        res.success,
        "workspace delete should succeed after resources removed: {}",
        res.output
    );

    // 10. Verify workspace is gone.
    let res = run_cli(&["workspace", "get", WORKSPACE]).await;
    assert!(
        !res.success,
        "workspace get should fail after deletion: {}",
        res.output
    );
}
