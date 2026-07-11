// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
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

#[test]
fn unidentified_mode_readyz_within_one_second() {
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

    let start = Instant::now();
    let deadline = Duration::from_secs(1);
    let poll_interval = Duration::from_millis(20);

    let mut ready = false;
    while start.elapsed() < deadline {
        if check_readyz(health_port) == Some(200) {
            ready = true;
            break;
        }
        std::thread::sleep(poll_interval);
    }

    let elapsed = start.elapsed();
    eprintln!("SC-002: /readyz returned 200 after {elapsed:?}");

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        ready,
        "/readyz did not return 200 within {deadline:?} (elapsed: {elapsed:?})"
    );
    assert!(
        elapsed < deadline,
        "SC-002 violated: readyz took {elapsed:?}, limit is {deadline:?}"
    );
}
