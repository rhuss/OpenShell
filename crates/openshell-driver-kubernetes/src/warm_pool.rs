// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use kube::Client;
use kube::api::{Api, ApiResource, ListParams, PostParams};
use kube::core::{DynamicObject, GroupVersionKind, ObjectMeta};
use serde_json::json;
use tracing::{debug, info, warn};

const WARM_POOL_GROUP: &str = "extensions.agents.x-k8s.io";
const WARM_POOL_VERSION: &str = "v1beta1";
const WARM_POOL_KIND: &str = "SandboxWarmPool";
const CLAIM_KIND: &str = "SandboxClaim";
const TEMPLATE_KIND: &str = "SandboxTemplate";

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

fn template_api(client: Client, namespace: &str) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, TEMPLATE_KIND);
    let resource = ApiResource::from_gvk(&gvk);
    Api::namespaced_with(client, namespace, &resource)
}

fn extract_template_image(template: &DynamicObject) -> Option<String> {
    template
        .data
        .get("spec")
        .and_then(|s| s.get("podTemplate"))
        .and_then(|pt| pt.get("spec"))
        .and_then(|s| s.get("containers"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("image"))
        .and_then(|i| i.as_str())
        .map(String::from)
}

pub async fn find_matching_pool<'a>(
    client: &Client,
    namespace: &str,
    pools: &'a [DynamicObject],
    image: &str,
) -> Option<&'a DynamicObject> {
    let tmpl_api = template_api(client.clone(), namespace);

    for pool in pools {
        let ready = pool
            .data
            .get("status")
            .and_then(|s| s.get("readyReplicas"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        if ready == 0 {
            continue;
        }

        let template_name = pool
            .data
            .get("spec")
            .and_then(|s| s.get("sandboxTemplateRef"))
            .and_then(|r| r.get("name"))
            .and_then(|n| n.as_str());

        let Some(tmpl_name) = template_name else {
            warn!(pool = ?pool.metadata.name, "SandboxWarmPool missing sandboxTemplateRef");
            continue;
        };

        match tmpl_api.get(tmpl_name).await {
            Ok(template) => {
                if let Some(tmpl_image) = extract_template_image(&template) {
                    if tmpl_image == image {
                        info!(
                            pool = ?pool.metadata.name,
                            template = %tmpl_name,
                            image = %image,
                            ready_replicas = %ready,
                            "Found matching warm pool"
                        );
                        return Some(pool);
                    }
                }
            }
            Err(e) => {
                warn!(template = %tmpl_name, error = %e, "Failed to fetch SandboxTemplate");
            }
        }
    }

    None
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
            "warmPoolRef": {
                "name": pool_name,
            },
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
                let conditions = obj
                    .data
                    .get("status")
                    .and_then(|s| s.get("conditions"))
                    .and_then(|c| c.as_array());

                let ready_condition = conditions.and_then(|conds| {
                    conds.iter().find(|c| {
                        c.get("type").and_then(|t| t.as_str()) == Some("Ready")
                    })
                });

                let is_ready = ready_condition
                    .and_then(|c| c.get("status"))
                    .and_then(|s| s.as_str())
                    == Some("True");

                let is_failed = ready_condition
                    .and_then(|c| c.get("reason"))
                    .and_then(|r| r.as_str())
                    .is_some_and(|r| r.contains("Failed"));

                if is_ready {
                    let pod_ip = obj
                        .data
                        .get("status")
                        .and_then(|s| s.get("sandbox"))
                        .and_then(|s| s.get("podIPs"))
                        .and_then(|ips| ips.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|ip| ip.as_str())
                        .map(String::from);

                    match pod_ip {
                        Some(ip) => {
                            info!(claim = %claim_name, pod_ip = %ip, "SandboxClaim ready");
                            return Ok(ip);
                        }
                        None => return Err(WarmPoolError::MissingPodIp),
                    }
                } else if is_failed {
                    let message = ready_condition
                        .and_then(|c| c.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown failure");
                    return Err(WarmPoolError::ClaimFailed(message.to_string()));
                } else {
                    let reason = ready_condition
                        .and_then(|c| c.get("reason"))
                        .and_then(|r| r.as_str())
                        .unwrap_or("Pending");
                    debug!(claim = %claim_name, reason = %reason, "Waiting for SandboxClaim");
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

    fn make_template(image: &str) -> DynamicObject {
        let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, TEMPLATE_KIND);
        let resource = ApiResource::from_gvk(&gvk);
        let mut obj = DynamicObject::new("test-template", &resource);
        obj.data = json!({
            "spec": {
                "podTemplate": {
                    "spec": {
                        "containers": [
                            {"image": image, "name": "agent"}
                        ]
                    }
                }
            }
        });
        obj
    }

    #[test]
    fn extract_template_image_finds_first_container() {
        let tmpl = make_template("ghcr.io/nvidia/sandbox:latest");
        assert_eq!(
            extract_template_image(&tmpl).as_deref(),
            Some("ghcr.io/nvidia/sandbox:latest")
        );
    }

    #[test]
    fn extract_template_image_returns_none_for_empty() {
        let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, TEMPLATE_KIND);
        let resource = ApiResource::from_gvk(&gvk);
        let mut obj = DynamicObject::new("empty", &resource);
        obj.data = json!({"spec": {}});
        assert!(extract_template_image(&obj).is_none());
    }

    fn make_pool_with_ready(ready_replicas: i64) -> DynamicObject {
        let gvk = GroupVersionKind::gvk(WARM_POOL_GROUP, WARM_POOL_VERSION, WARM_POOL_KIND);
        let resource = ApiResource::from_gvk(&gvk);
        let mut obj = DynamicObject::new("test-pool", &resource);
        obj.data = json!({
            "spec": {
                "sandboxTemplateRef": {"name": "test-template"},
                "replicas": 5,
            },
            "status": {
                "readyReplicas": ready_replicas,
                "replicas": 5,
            }
        });
        obj
    }

    #[test]
    fn pool_ready_replicas_check() {
        let pool = make_pool_with_ready(3);
        let ready = pool
            .data
            .get("status")
            .and_then(|s| s.get("readyReplicas"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        assert_eq!(ready, 3);
    }

    #[test]
    fn pool_zero_replicas_skipped() {
        let pool = make_pool_with_ready(0);
        let ready = pool
            .data
            .get("status")
            .and_then(|s| s.get("readyReplicas"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        assert_eq!(ready, 0);
    }

    #[test]
    fn pool_template_ref_extraction() {
        let pool = make_pool_with_ready(3);
        let tmpl_name = pool
            .data
            .get("spec")
            .and_then(|s| s.get("sandboxTemplateRef"))
            .and_then(|r| r.get("name"))
            .and_then(|n| n.as_str());
        assert_eq!(tmpl_name, Some("test-template"));
    }
}
