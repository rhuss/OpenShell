// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use kube::Client;
use kube::api::{Api, ApiResource, ListParams, PostParams};
use kube::core::{DynamicObject, GroupVersionKind, ObjectMeta};
use serde_json::json;
use tracing::{debug, info, warn};

const WARM_POOL_GROUP: &str = "agents.x-k8s.io";
const WARM_POOL_VERSION: &str = "v1alpha1";
const WARM_POOL_KIND: &str = "SandboxWarmPool";
const CLAIM_KIND: &str = "SandboxClaim";

#[derive(Debug, thiserror::Error)]
pub enum WarmPoolError {
    #[error("claim timed out after {0:?}")]
    Timeout(Duration),
    #[error("claim failed with phase: {0}")]
    ClaimFailed(String),
    #[error("claim ready but missing pod IP")]
    MissingPodIp,
    #[error("kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
}

fn warm_pool_api(client: Client, namespace: &str) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, WARM_POOL_KIND);
    let resource = ApiResource::from_gvk(&gvk);
    Api::namespaced_with(client, namespace, &resource)
}

fn claim_api(client: Client, namespace: &str) -> (Api<DynamicObject>, ApiResource) {
    let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, CLAIM_KIND);
    let resource = ApiResource::from_gvk(&gvk);
    let api = Api::namespaced_with(client, namespace, &resource);
    (api, resource)
}

pub async fn list_warm_pools(
    client: &Client,
    namespace: &str,
) -> Result<Vec<DynamicObject>, kube::Error> {
    let api = warm_pool_api(client.clone(), namespace);
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items)
}

pub fn find_matching_pool<'a>(
    pools: &'a [DynamicObject],
    image: &str,
) -> Option<&'a DynamicObject> {
    pools.iter().find(|pool| {
        let image_match = pool
            .data
            .get("spec")
            .and_then(|s| s.get("template"))
            .and_then(|t| t.get("containers"))
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("image"))
            .and_then(|i| i.as_str())
            .is_some_and(|i| i == image);

        let ready = pool
            .data
            .get("status")
            .and_then(|s| s.get("readyReplicas"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        image_match && ready > 0
    })
}

pub async fn create_claim(
    client: &Client,
    namespace: &str,
    pool_name: &str,
    sandbox_id: &str,
) -> Result<String, kube::Error> {
    let (api, resource) = claim_api(client.clone(), namespace);
    let claim_name = format!("claim-{sandbox_id}");
    let mut obj = DynamicObject::new(&claim_name, &resource);
    obj.metadata = ObjectMeta {
        name: Some(claim_name.clone()),
        namespace: Some(namespace.to_string()),
        ..Default::default()
    };
    obj.data = json!({
        "spec": {
            "warmPoolRef": pool_name,
            "sandboxId": sandbox_id,
        }
    });

    info!(claim = %claim_name, pool = %pool_name, sandbox_id = %sandbox_id, "Creating SandboxClaim");
    api.create(&PostParams::default(), &obj).await?;
    Ok(claim_name)
}

pub async fn wait_for_claim_ready(
    client: &Client,
    namespace: &str,
    claim_name: &str,
    timeout: Duration,
) -> Result<String, WarmPoolError> {
    let (api, _resource) = claim_api(client.clone(), namespace);
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(200);

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(WarmPoolError::Timeout(timeout));
        }

        match api.get(claim_name).await {
            Ok(obj) => {
                let phase = obj
                    .data
                    .get("status")
                    .and_then(|s| s.get("phase"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("Pending");

                match phase {
                    "Ready" => {
                        let pod_ip = obj
                            .data
                            .get("status")
                            .and_then(|s| s.get("sandbox"))
                            .and_then(|s| s.get("podIP"))
                            .and_then(|ip| ip.as_str())
                            .map(String::from);

                        match pod_ip {
                            Some(ip) => {
                                info!(claim = %claim_name, pod_ip = %ip, "SandboxClaim ready");
                                return Ok(ip);
                            }
                            None => return Err(WarmPoolError::MissingPodIp),
                        }
                    }
                    "Failed" => {
                        return Err(WarmPoolError::ClaimFailed(
                            obj.data
                                .get("status")
                                .and_then(|s| s.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown failure")
                                .to_string(),
                        ));
                    }
                    _ => {
                        debug!(claim = %claim_name, phase = %phase, "Waiting for SandboxClaim");
                    }
                }
            }
            Err(e) => {
                warn!(claim = %claim_name, error = %e, "Error polling SandboxClaim");
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool(image: &str, ready_replicas: i64) -> DynamicObject {
        let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, WARM_POOL_KIND);
        let resource = ApiResource::from_gvk(&gvk);
        let mut obj = DynamicObject::new("test-pool", &resource);
        obj.data = json!({
            "spec": {
                "template": {
                    "containers": [
                        {"image": image}
                    ]
                }
            },
            "status": {
                "readyReplicas": ready_replicas
            }
        });
        obj
    }

    #[test]
    fn find_matching_pool_exact_image() {
        let pools = vec![
            make_pool("registry/sandbox:v1", 3),
            make_pool("registry/sandbox:v2", 1),
        ];
        let found = find_matching_pool(&pools, "registry/sandbox:v2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().metadata.name.as_deref(), Some("test-pool"));
    }

    #[test]
    fn find_matching_pool_no_match() {
        let pools = vec![make_pool("registry/sandbox:v1", 3)];
        assert!(find_matching_pool(&pools, "registry/other:v1").is_none());
    }

    #[test]
    fn find_matching_pool_zero_replicas() {
        let pools = vec![make_pool("registry/sandbox:v1", 0)];
        assert!(find_matching_pool(&pools, "registry/sandbox:v1").is_none());
    }

    #[test]
    fn find_matching_pool_empty_list() {
        let pools: Vec<DynamicObject> = vec![];
        assert!(find_matching_pool(&pools, "registry/sandbox:v1").is_none());
    }
}
