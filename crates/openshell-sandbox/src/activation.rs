// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicBool, Ordering};

use openshell_core::proto::supervisor::v1::supervisor_server::Supervisor;
use openshell_core::proto::supervisor::v1::{
    ActivateSandboxRequest, ActivateSandboxResponse, ErrorCode,
};
use openshell_ocsf::{
    ActionId, ActivityId, AppLifecycleBuilder, DetectionFindingBuilder, DispositionId, FindingInfo,
    SandboxContext, SeverityId, StatusId, ocsf_emit,
};
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};
use tracing::info;

use crate::BootstrapError;

pub struct SupervisorService {
    activated: AtomicBool,
    activation_tx: std::sync::Mutex<Option<oneshot::Sender<i32>>>,
}

impl SupervisorService {
    pub fn new(activation_tx: oneshot::Sender<i32>) -> Self {
        Self {
            activated: AtomicBool::new(false),
            activation_tx: std::sync::Mutex::new(Some(activation_tx)),
        }
    }
}

impl BootstrapError {
    fn to_error_code(&self) -> ErrorCode {
        match self {
            Self::PolicyCompilation(_) => ErrorCode::PolicyCompilationFailed,
            Self::GatewayUnreachable(_) => ErrorCode::GatewayUnreachable,
            Self::TokenInvalid(_) => ErrorCode::TokenInvalid,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }
}

#[tonic::async_trait]
impl Supervisor for SupervisorService {
    async fn activate_sandbox(
        &self,
        request: Request<ActivateSandboxRequest>,
    ) -> Result<Response<ActivateSandboxResponse>, Status> {
        let req = request.into_inner();
        info!(sandbox_id = %req.sandbox_id, "ActivateSandbox request received");

        if self.activated.swap(true, Ordering::AcqRel) {
            return Ok(Response::new(ActivateSandboxResponse {
                success: false,
                error_message: "supervisor already activated".into(),
                error_code: ErrorCode::AlreadyActivated.into(),
            }));
        }

        if req.sandbox_id.is_empty()
            || req.sandbox_token.is_empty()
            || req.gateway_endpoint.is_empty()
        {
            self.activated.store(false, Ordering::Release);
            return Ok(Response::new(ActivateSandboxResponse {
                success: false,
                error_message: "sandbox_id, sandbox_token, and gateway_endpoint are required"
                    .into(),
                error_code: ErrorCode::InvalidRequest.into(),
            }));
        }

        let Some(policy) = req.policy else {
            self.activated.store(false, Ordering::Release);
            return Ok(Response::new(ActivateSandboxResponse {
                success: false,
                error_message: "policy is required".into(),
                error_code: ErrorCode::InvalidRequest.into(),
            }));
        };

        let hostname = std::fs::read_to_string("/etc/hostname").map_or_else(
            |_| "openshell-sandbox".to_string(),
            |h| h.trim().to_string(),
        );
        openshell_ocsf::ctx::set_ctx(SandboxContext {
            sandbox_id: req.sandbox_id.clone(),
            sandbox_name: req.sandbox_name.clone(),
            container_image: std::env::var("OPENSHELL_CONTAINER_IMAGE").unwrap_or_default(),
            hostname,
            product_version: openshell_core::VERSION.to_string(),
            proxy_ip: std::net::IpAddr::from([127, 0, 0, 1]),
            proxy_port: 3128,
        });

        ocsf_emit!(
            AppLifecycleBuilder::new(crate::ocsf_ctx())
                .activity(ActivityId::Reset)
                .severity(SeverityId::Informational)
                .status(StatusId::Success)
                .message(format!(
                    "Warm pool activation starting [sandbox_id:{}]",
                    req.sandbox_id
                ))
                .build()
        );

        let bootstrap_ctx = match crate::bootstrap_sandbox(
            &req.sandbox_id,
            &req.sandbox_name,
            req.sandbox_token,
            &req.gateway_endpoint,
            policy,
        )
        .await
        {
            Ok(ctx) => ctx,
            Err(e) => {
                self.activated.store(false, Ordering::Release);
                let error_code = e.to_error_code();
                let error_msg = e.to_string();

                ocsf_emit!(
                    AppLifecycleBuilder::new(crate::ocsf_ctx())
                        .activity(ActivityId::Fail)
                        .severity(SeverityId::Medium)
                        .status(StatusId::Failure)
                        .message(format!(
                            "Warm pool activation failed [sandbox_id:{}]: {error_msg}",
                            req.sandbox_id
                        ))
                        .build()
                );
                ocsf_emit!(
                    DetectionFindingBuilder::new(crate::ocsf_ctx())
                        .activity(ActivityId::Open)
                        .severity(SeverityId::Medium)
                        .action(ActionId::Denied)
                        .disposition(DispositionId::Blocked)
                        .finding_info(
                            FindingInfo::new(
                                "warm-pool-activation-failure",
                                "Warm Pool Activation Failure",
                            )
                            .with_desc(&format!(
                                "Supervisor activation failed for sandbox {}: {error_msg}",
                                req.sandbox_id,
                            )),
                        )
                        .message(format!(
                            "Activation bootstrap failed [sandbox_id:{}]",
                            req.sandbox_id
                        ))
                        .build()
                );

                return Ok(Response::new(ActivateSandboxResponse {
                    success: false,
                    error_message: error_msg,
                    error_code: error_code.into(),
                }));
            }
        };

        ocsf_emit!(
            AppLifecycleBuilder::new(crate::ocsf_ctx())
                .activity(ActivityId::Reset)
                .severity(SeverityId::Informational)
                .status(StatusId::Success)
                .message(format!(
                    "Warm pool activation succeeded [sandbox_id:{}]",
                    req.sandbox_id
                ))
                .build()
        );

        let sender = self.activation_tx.lock().unwrap().take();
        if let Some(sender) = sender {
            let sandbox_id = req.sandbox_id.clone();
            tokio::spawn(async move {
                let exit_code = match crate::post_identity_bootstrap(bootstrap_ctx).await {
                    Ok(code) => code,
                    Err(e) => {
                        tracing::error!(
                            sandbox_id = %sandbox_id,
                            error = %e,
                            "post_identity_bootstrap failed"
                        );
                        1
                    }
                };
                let _ = sender.send(exit_code);
            });
        }

        Ok(Response::new(ActivateSandboxResponse {
            success: true,
            error_message: String::new(),
            error_code: ErrorCode::Unspecified.into(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_request() -> ActivateSandboxRequest {
        ActivateSandboxRequest {
            sandbox_id: "test-sandbox-id".into(),
            sandbox_name: "test-sandbox".into(),
            sandbox_token: "test-jwt-token".into(),
            gateway_endpoint: "https://gateway:8443".into(),
            policy: Some(openshell_core::proto::sandbox::v1::SandboxPolicy::default()),
        }
    }

    #[tokio::test]
    async fn rejects_empty_sandbox_id() {
        let (tx, _rx) = oneshot::channel();
        let svc = SupervisorService::new(tx);
        let mut req = make_valid_request();
        req.sandbox_id = String::new();
        let resp = svc
            .activate_sandbox(Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert_eq!(resp.error_code, ErrorCode::InvalidRequest as i32);
    }

    #[tokio::test]
    async fn rejects_missing_policy() {
        let (tx, _rx) = oneshot::channel();
        let svc = SupervisorService::new(tx);
        let mut req = make_valid_request();
        req.policy = None;
        let resp = svc
            .activate_sandbox(Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert_eq!(resp.error_code, ErrorCode::InvalidRequest as i32);
    }

    #[tokio::test]
    async fn second_call_returns_already_activated() {
        let (tx, _rx) = oneshot::channel();
        let svc = SupervisorService::new(tx);
        let req = make_valid_request();

        // First call will attempt bootstrap_sandbox which will fail in test
        // (no real gateway), but the activated flag gets set first
        let _resp1 = svc
            .activate_sandbox(Request::new(req.clone()))
            .await
            .unwrap()
            .into_inner();

        // Second call should return AlreadyActivated regardless
        // (only if first succeeded - if it failed, activated is reset)
        // For this test, we just verify the idempotency guard works
        // by checking the second call after the flag is manually set
        svc.activated.store(true, Ordering::Release);
        let resp2 = svc
            .activate_sandbox(Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp2.success);
        assert_eq!(resp2.error_code, ErrorCode::AlreadyActivated as i32);
    }

    #[tokio::test]
    async fn bootstrap_failure_resets_activated_flag() {
        let (tx, _rx) = oneshot::channel();
        let svc = SupervisorService::new(tx);
        let req = make_valid_request();

        // bootstrap_sandbox will fail in test env (no real gateway)
        let resp = svc
            .activate_sandbox(Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        // After failure, the activated flag should be reset
        assert!(!svc.activated.load(Ordering::Acquire));
    }
}
