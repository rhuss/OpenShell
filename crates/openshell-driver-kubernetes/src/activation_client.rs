// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use openshell_core::proto::supervisor::v1::ActivateSandboxRequest;
use openshell_core::proto::supervisor::v1::ActivateSandboxResponse;
use openshell_core::proto::supervisor::v1::supervisor_client::SupervisorClient;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tracing::info;

const ACTIVATION_PORT: u16 = 9090;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("activation timed out after {0:?}")]
    Timeout(Duration),
    #[error("connection to supervisor failed: {0}")]
    ConnectionFailed(String),
    #[error("gRPC error: {0}")]
    RpcError(#[from] tonic::Status),
}

pub struct TlsConfig {
    pub ca_cert: Vec<u8>,
    pub client_cert: Vec<u8>,
    pub client_key: Vec<u8>,
}

pub async fn activate_sandbox(
    pod_ip: &str,
    request: ActivateSandboxRequest,
    tls: Option<&TlsConfig>,
) -> Result<ActivateSandboxResponse, ActivationError> {
    let (endpoint_uri, mut endpoint) = if let Some(tls) = tls {
        let uri = format!("https://{pod_ip}:{ACTIVATION_PORT}");
        let mut ep = Channel::from_shared(uri.clone())
            .map_err(|e| ActivationError::ConnectionFailed(e.to_string()))?
            .connect_timeout(Duration::from_secs(2));

        let tls_config = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(&tls.ca_cert))
            .identity(Identity::from_pem(&tls.client_cert, &tls.client_key));
        ep = ep
            .tls_config(tls_config)
            .map_err(|e| ActivationError::ConnectionFailed(e.to_string()))?;

        (uri, ep)
    } else {
        let uri = format!("http://{pod_ip}:{ACTIVATION_PORT}");
        let ep = Channel::from_shared(uri.clone())
            .map_err(|e| ActivationError::ConnectionFailed(e.to_string()))?
            .connect_timeout(Duration::from_secs(2));
        (uri, ep)
    };

    info!(endpoint = %endpoint_uri, sandbox_id = %request.sandbox_id, "Connecting to supervisor for activation");

    let channel = endpoint
        .connect()
        .await
        .map_err(|e| ActivationError::ConnectionFailed(e.to_string()))?;

    let mut client = SupervisorClient::new(channel);

    let response = tokio::time::timeout(ACTIVATION_TIMEOUT, client.activate_sandbox(request))
        .await
        .map_err(|_| ActivationError::Timeout(ACTIVATION_TIMEOUT))?
        .map_err(ActivationError::RpcError)?;

    Ok(response.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connection_to_nonexistent_host_fails() {
        let req = ActivateSandboxRequest {
            sandbox_id: "test-id".into(),
            sandbox_name: "test".into(),
            sandbox_token: "token".into(),
            gateway_endpoint: "gateway:8443".into(),
            policy: None,
        };
        let result = activate_sandbox("192.0.2.1", req, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ActivationError::ConnectionFailed(_) | ActivationError::Timeout(_) => {}
            other @ ActivationError::RpcError(_) => {
                panic!("expected ConnectionFailed or Timeout, got: {other}")
            }
        }
    }
}
