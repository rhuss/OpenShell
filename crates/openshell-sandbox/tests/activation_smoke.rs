// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Smoke test (SC-005): an activated supervisor exercises the same bootstrap
//! path as a cold-started one. Because no real gateway is available in CI,
//! we verify:
//!   1. The gRPC service accepts `ActivateSandbox` calls.
//!   2. Bootstrap fails with `GATEWAY_UNREACHABLE` (proving it attempted the
//!      real bootstrap path, not a stub).
//!   3. A second call returns `ALREADY_ACTIVATED` or the flag is reset after
//!      failure, allowing a retry.
//!   4. The process remains healthy throughout (no panics, readyz still 200).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use openshell_core::proto::sandbox::v1::SandboxPolicy;
use openshell_core::proto::supervisor::v1::supervisor_client::SupervisorClient;
use openshell_core::proto::supervisor::v1::{ActivateSandboxRequest, ErrorCode};

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_readyz(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check_readyz(port) == Some(200) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn check_readyz(port: u16) -> Option<u16> {
    let addr = format!("127.0.0.1:{port}");
    let mut stream =
        TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    stream
        .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).ok()?;
    let response = std::str::from_utf8(&buf[..n]).ok()?;
    let status_line = response.lines().next()?;
    let status_code: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;
    Some(status_code)
}

#[tokio::test]
async fn activated_supervisor_exercises_cold_start_bootstrap() {
    let health_port = free_port();
    let grpc_port = free_port();

    let mut child = Command::new(env!("CARGO_BIN_EXE_openshell-sandbox"))
        .arg("--unidentified")
        .arg("--health-check")
        .arg("--health-port")
        .arg(health_port.to_string())
        .arg("--")
        .arg("/bin/true")
        .env("OPENSHELL_LOG_LEVEL", "info")
        .env("OPENSHELL_ACTIVATION_PORT", grpc_port.to_string())
        .env_remove("RUST_LOG")
        .env_remove("OPENSHELL_POLICY_RULES")
        .env_remove("OPENSHELL_POLICY_DATA")
        .env_remove("OPENSHELL_SANDBOX_ID")
        .env_remove("OPENSHELL_ENDPOINT")
        .spawn()
        .expect("failed to spawn openshell-sandbox in unidentified mode");

    assert!(
        wait_for_readyz(health_port, Duration::from_secs(2)),
        "readyz did not return 200 in time"
    );

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    let channel = tonic::transport::Channel::from_shared(endpoint)
        .unwrap()
        .connect_timeout(Duration::from_secs(2))
        .connect()
        .await
        .expect("failed to connect to gRPC server");

    let mut client = SupervisorClient::new(channel);

    let activate_start = Instant::now();
    let resp = client
        .activate_sandbox(ActivateSandboxRequest {
            sandbox_id: "smoke-test-sandbox".into(),
            sandbox_name: "smoke-test".into(),
            sandbox_token: "fake-jwt-token".into(),
            gateway_endpoint: "https://gateway.invalid:8443".into(),
            policy: Some(SandboxPolicy::default()),
        })
        .await
        .expect("gRPC call should not return transport error")
        .into_inner();
    let activate_elapsed = activate_start.elapsed();

    eprintln!("SC-001: ActivateSandbox call completed in {activate_elapsed:?}");
    eprintln!(
        "SC-005: response success={}, error_code={}, error_message={}",
        resp.success, resp.error_code, resp.error_message
    );

    assert!(
        !resp.success,
        "activation should fail without a real gateway"
    );
    assert_eq!(
        resp.error_code,
        ErrorCode::GatewayUnreachable as i32,
        "expected GATEWAY_UNREACHABLE proving real bootstrap path was exercised, got error_code={} msg={}",
        resp.error_code,
        resp.error_message,
    );

    // After bootstrap failure, the activated flag should be reset,
    // proving the idempotency guard works correctly on failure recovery
    let resp2 = client
        .activate_sandbox(ActivateSandboxRequest {
            sandbox_id: "smoke-test-sandbox-2".into(),
            sandbox_name: "smoke-test-2".into(),
            sandbox_token: "fake-jwt-token-2".into(),
            gateway_endpoint: "https://gateway.invalid:8443".into(),
            policy: Some(SandboxPolicy::default()),
        })
        .await
        .expect("second gRPC call should not return transport error")
        .into_inner();

    assert!(
        !resp2.success,
        "second activation should also fail (no gateway)"
    );
    assert_ne!(
        resp2.error_code,
        ErrorCode::AlreadyActivated as i32,
        "after bootstrap failure, flag should be reset so retry is allowed"
    );

    // Verify the process is still healthy after failed activations
    assert!(
        check_readyz(health_port) == Some(200),
        "readyz should still return 200 after failed activations"
    );

    let _ = child.kill();
    let _ = child.wait();
}
