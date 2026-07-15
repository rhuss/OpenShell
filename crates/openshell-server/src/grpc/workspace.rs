// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Workspace lifecycle handlers.

#![allow(clippy::result_large_err)] // gRPC handlers return Result<Response<_>, Status>

use std::sync::Arc;

use openshell_core::ObjectName;
use openshell_core::proto::datamodel::v1::ObjectMeta;
use openshell_core::proto::{
    AddWorkspaceMemberRequest, AddWorkspaceMemberResponse, CreateWorkspaceRequest,
    CreateWorkspaceResponse, DeleteWorkspaceRequest, DeleteWorkspaceResponse, GetWorkspaceRequest,
    GetWorkspaceResponse, InferenceRoute, ListWorkspaceMembersRequest,
    ListWorkspaceMembersResponse, ListWorkspacesRequest, ListWorkspacesResponse, Provider,
    RemoveWorkspaceMemberRequest, RemoveWorkspaceMemberResponse, Sandbox, ServiceEndpoint,
    SshSession, StoredProviderProfile, Workspace, WorkspaceMember, WorkspaceRole,
};
use prost::Message;
use tonic::{Request, Response, Status};

use crate::ServerState;
use crate::persistence::{ObjectType, WriteCondition, current_time_ms};

use super::{MAX_PAGE_SIZE, clamp_limit};

pub const WORKSPACE_OBJECT_TYPE: &str = "workspace";
pub const DEFAULT_WORKSPACE_NAME: &str = "default";
const MAX_WORKSPACE_MEMBERS: u32 = 1000;

impl ObjectType for Workspace {
    fn object_type() -> &'static str {
        WORKSPACE_OBJECT_TYPE
    }
}

impl ObjectType for WorkspaceMember {
    fn object_type() -> &'static str {
        "workspace_member"
    }
}

fn validate_workspace_name(name: &str) -> Result<(), Status> {
    if name.is_empty() {
        return Err(Status::invalid_argument("workspace name is required"));
    }
    super::validation::validate_dns1123_label(name, "workspace name")
}

/// Resolve and validate a workspace name from a request field.
///
/// Empty strings are normalized to `"default"`. The workspace must exist in the
/// store; returns `NOT_FOUND` if it doesn't.
pub async fn resolve_workspace(
    store: &crate::persistence::Store,
    workspace: &str,
) -> Result<String, Status> {
    let name = if workspace.is_empty() {
        DEFAULT_WORKSPACE_NAME.to_string()
    } else {
        workspace.to_string()
    };

    let exists: Option<Workspace> = store
        .get_message_by_name("", &name)
        .await
        .map_err(|e| Status::internal(format!("workspace lookup failed: {e}")))?;

    if exists.is_none() {
        return Err(Status::not_found(format!("workspace '{name}' not found")));
    }

    Ok(name)
}

pub(super) async fn handle_create_workspace(
    state: &Arc<ServerState>,
    request: Request<CreateWorkspaceRequest>,
) -> Result<Response<CreateWorkspaceResponse>, Status> {
    let req = request.into_inner();

    validate_workspace_name(&req.name)?;

    let now_ms = current_time_ms();
    let workspace_id = uuid::Uuid::new_v4().to_string();

    let workspace = Workspace {
        metadata: Some(ObjectMeta {
            id: workspace_id.clone(),
            name: req.name,
            created_at_ms: now_ms,
            labels: req.labels,
            resource_version: 0,
            workspace: String::new(),
        }),
    };

    super::validation::validate_object_metadata(workspace.metadata.as_ref(), "workspace")?;

    let result = state
        .store
        .put_if(
            Workspace::object_type(),
            &workspace_id,
            workspace.object_name(),
            "",
            &workspace.encode_to_vec(),
            None,
            WriteCondition::MustCreate,
        )
        .await
        .map_err(|e| {
            if matches!(
                e,
                crate::persistence::PersistenceError::UniqueViolation { .. }
            ) {
                Status::already_exists("workspace already exists")
            } else {
                Status::internal(format!("persist workspace failed: {e}"))
            }
        })?;

    let mut workspace = workspace;
    if let Some(metadata) = workspace.metadata.as_mut() {
        metadata.resource_version = result.resource_version;
    }

    Ok(Response::new(CreateWorkspaceResponse {
        workspace: Some(workspace),
    }))
}

pub(super) async fn handle_get_workspace(
    state: &Arc<ServerState>,
    request: Request<GetWorkspaceRequest>,
) -> Result<Response<GetWorkspaceResponse>, Status> {
    let name = request.into_inner().name;
    if name.is_empty() {
        return Err(Status::invalid_argument("name is required"));
    }

    let workspace: Workspace = state
        .store
        .get_message_by_name("", &name)
        .await
        .map_err(|e| Status::internal(format!("fetch workspace failed: {e}")))?
        .ok_or_else(|| Status::not_found("workspace not found"))?;

    Ok(Response::new(GetWorkspaceResponse {
        workspace: Some(workspace),
    }))
}

pub(super) async fn handle_list_workspaces(
    state: &Arc<ServerState>,
    request: Request<ListWorkspacesRequest>,
) -> Result<Response<ListWorkspacesResponse>, Status> {
    let req = request.into_inner();
    let limit = clamp_limit(req.limit, 100, MAX_PAGE_SIZE);

    let workspaces: Vec<Workspace> = if req.label_selector.is_empty() {
        state
            .store
            .list_messages("", limit, req.offset)
            .await
            .map_err(|e| Status::internal(format!("list workspaces failed: {e}")))?
    } else {
        state
            .store
            .list_messages_with_selector("", &req.label_selector, limit, req.offset)
            .await
            .map_err(|e| Status::internal(format!("list workspaces failed: {e}")))?
    };

    Ok(Response::new(ListWorkspacesResponse { workspaces }))
}

pub(super) async fn handle_delete_workspace(
    state: &Arc<ServerState>,
    request: Request<DeleteWorkspaceRequest>,
) -> Result<Response<DeleteWorkspaceResponse>, Status> {
    let name = request.into_inner().name;
    if name.is_empty() {
        return Err(Status::invalid_argument("name is required"));
    }
    if name == DEFAULT_WORKSPACE_NAME {
        return Err(Status::failed_precondition(
            "the default workspace cannot be deleted",
        ));
    }

    let mut blocking = Vec::new();
    for (object_type, label) in [
        (Sandbox::object_type(), "sandbox"),
        (Provider::object_type(), "provider"),
        (StoredProviderProfile::object_type(), "provider profile"),
        (ServiceEndpoint::object_type(), "service"),
        (InferenceRoute::object_type(), "inference route"),
        (SshSession::object_type(), "ssh session"),
    ] {
        let records = state
            .store
            .list(object_type, &name, 1, 0)
            .await
            .map_err(|e| Status::internal(format!("resource check failed: {e}")))?;
        if !records.is_empty() {
            blocking.push(label);
        }
    }
    if !blocking.is_empty() {
        return Err(Status::failed_precondition(format!(
            "workspace '{}' still contains resources: {}",
            name,
            blocking.join(", ")
        )));
    }

    // Clean up membership records before deleting the workspace itself.
    state
        .store
        .delete_all_in_workspace(WorkspaceMember::object_type(), &name)
        .await
        .map_err(|e| Status::internal(format!("delete workspace members failed: {e}")))?;

    let deleted = state
        .store
        .delete_by_name(Workspace::object_type(), "", &name)
        .await
        .map_err(|e| Status::internal(format!("delete workspace failed: {e}")))?;

    Ok(Response::new(DeleteWorkspaceResponse { deleted }))
}

pub(super) async fn handle_add_workspace_member(
    state: &Arc<ServerState>,
    request: Request<AddWorkspaceMemberRequest>,
) -> Result<Response<AddWorkspaceMemberResponse>, Status> {
    let req = request.into_inner();

    let workspace = resolve_workspace(&state.store, &req.workspace).await?;

    if req.principal_subject.is_empty() {
        return Err(Status::invalid_argument("principal_subject is required"));
    }

    let role = WorkspaceRole::try_from(req.role).unwrap_or(WorkspaceRole::Unspecified);
    if role == WorkspaceRole::Unspecified {
        return Err(Status::invalid_argument(
            "role must be USER or ADMIN, not UNSPECIFIED",
        ));
    }

    let count = state
        .store
        .count_in_workspace(WorkspaceMember::object_type(), &workspace)
        .await
        .map_err(|e| Status::internal(format!("count workspace members failed: {e}")))?;
    if count >= u64::from(MAX_WORKSPACE_MEMBERS) {
        return Err(Status::resource_exhausted(format!(
            "workspace has reached the maximum of {MAX_WORKSPACE_MEMBERS} members"
        )));
    }

    let member_id = uuid::Uuid::new_v4().to_string();
    let now_ms = current_time_ms();

    let member = WorkspaceMember {
        metadata: Some(ObjectMeta {
            id: member_id.clone(),
            name: req.principal_subject.clone(),
            created_at_ms: now_ms,
            labels: std::collections::HashMap::new(),
            resource_version: 0,
            workspace: workspace.clone(),
        }),
        principal_subject: req.principal_subject,
        role: req.role,
    };

    let result = state
        .store
        .put_if(
            WorkspaceMember::object_type(),
            &member_id,
            member.object_name(),
            &workspace,
            &member.encode_to_vec(),
            None,
            WriteCondition::MustCreate,
        )
        .await
        .map_err(|e| {
            if matches!(
                e,
                crate::persistence::PersistenceError::UniqueViolation { .. }
            ) {
                Status::already_exists("member already exists in this workspace")
            } else {
                Status::internal(format!("persist workspace member failed: {e}"))
            }
        })?;

    let mut member = member;
    if let Some(metadata) = member.metadata.as_mut() {
        metadata.resource_version = result.resource_version;
    }

    Ok(Response::new(AddWorkspaceMemberResponse {
        member: Some(member),
    }))
}

pub(super) async fn handle_remove_workspace_member(
    state: &Arc<ServerState>,
    request: Request<RemoveWorkspaceMemberRequest>,
) -> Result<Response<RemoveWorkspaceMemberResponse>, Status> {
    let req = request.into_inner();

    let workspace = resolve_workspace(&state.store, &req.workspace).await?;

    if req.principal_subject.is_empty() {
        return Err(Status::invalid_argument("principal_subject is required"));
    }

    let removed = state
        .store
        .delete_by_name(
            WorkspaceMember::object_type(),
            &workspace,
            &req.principal_subject,
        )
        .await
        .map_err(|e| Status::internal(format!("remove workspace member failed: {e}")))?;

    Ok(Response::new(RemoveWorkspaceMemberResponse { removed }))
}

pub(super) async fn handle_list_workspace_members(
    state: &Arc<ServerState>,
    request: Request<ListWorkspaceMembersRequest>,
) -> Result<Response<ListWorkspaceMembersResponse>, Status> {
    let req = request.into_inner();

    let workspace = resolve_workspace(&state.store, &req.workspace).await?;

    let limit = clamp_limit(req.limit, 100, MAX_PAGE_SIZE);

    let members: Vec<WorkspaceMember> = state
        .store
        .list_messages(&workspace, limit, req.offset)
        .await
        .map_err(|e| Status::internal(format!("list workspace members failed: {e}")))?;

    Ok(Response::new(ListWorkspaceMembersResponse { members }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use openshell_core::proto::datamodel::v1::ObjectMeta;
    use tonic::{Code, Request};

    use crate::grpc::test_support::test_server_state;

    #[tokio::test]
    async fn delete_workspace_blocked_by_resources() {
        let state = test_server_state().await;

        handle_create_workspace(
            &state,
            Request::new(CreateWorkspaceRequest {
                name: "ephemeral".to_string(),
                labels: HashMap::new(),
            }),
        )
        .await
        .unwrap();

        let sbx = Sandbox {
            metadata: Some(ObjectMeta {
                id: "sbx-eph-1".to_string(),
                name: "blocker".to_string(),
                created_at_ms: 1_000_000,
                labels: HashMap::new(),
                resource_version: 0,
                workspace: "ephemeral".to_string(),
            }),
            ..Default::default()
        };
        state.store.put_message(&sbx).await.unwrap();

        let err = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "ephemeral".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().contains("sandbox"),
            "error should name the blocking resource type: {}",
            err.message()
        );

        state
            .store
            .delete_by_name(Sandbox::object_type(), "ephemeral", "blocker")
            .await
            .unwrap();

        let resp = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "ephemeral".to_string(),
            }),
        )
        .await
        .unwrap()
        .into_inner();
        assert!(resp.deleted);
    }

    #[tokio::test]
    async fn delete_workspace_blocked_by_ssh_session() {
        let state = test_server_state().await;

        handle_create_workspace(
            &state,
            Request::new(CreateWorkspaceRequest {
                name: "sessioned".to_string(),
                labels: HashMap::new(),
            }),
        )
        .await
        .unwrap();

        let session = SshSession {
            metadata: Some(ObjectMeta {
                id: "ssh-1".to_string(),
                name: "session-ssh-1".to_string(),
                created_at_ms: 1_000_000,
                labels: HashMap::new(),
                resource_version: 0,
                workspace: "sessioned".to_string(),
            }),
            sandbox_id: "sbx-1".to_string(),
            token: "ssh-1".to_string(),
            revoked: false,
            expires_at_ms: 0,
        };
        state.store.put_message(&session).await.unwrap();

        let err = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "sessioned".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().contains("ssh session"),
            "error should name ssh session as blocker: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn delete_workspace_blocked_by_provider_profiles() {
        let state = test_server_state().await;

        handle_create_workspace(
            &state,
            Request::new(CreateWorkspaceRequest {
                name: "profiles-ws".to_string(),
                labels: HashMap::new(),
            }),
        )
        .await
        .unwrap();

        let profile = StoredProviderProfile {
            metadata: Some(ObjectMeta {
                id: "prof-1".to_string(),
                name: "my-profile".to_string(),
                created_at_ms: 1_000_000,
                labels: HashMap::new(),
                resource_version: 0,
                workspace: "profiles-ws".to_string(),
            }),
            ..Default::default()
        };
        state.store.put_message(&profile).await.unwrap();

        let err = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "profiles-ws".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().contains("provider profile"),
            "error should name provider profile as blocking: {}",
            err.message()
        );

        state
            .store
            .delete_by_name(
                StoredProviderProfile::object_type(),
                "profiles-ws",
                "my-profile",
            )
            .await
            .unwrap();

        let resp = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "profiles-ws".to_string(),
            }),
        )
        .await
        .unwrap()
        .into_inner();
        assert!(resp.deleted);
    }

    #[tokio::test]
    async fn delete_default_workspace_rejected() {
        let state = test_server_state().await;

        let err = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "default".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn add_and_list_workspace_members() {
        let state = test_server_state().await;

        let resp = handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "alice@example.com".to_string(),
                role: WorkspaceRole::Admin.into(),
            }),
        )
        .await
        .unwrap()
        .into_inner();

        let member = resp.member.unwrap();
        assert_eq!(member.principal_subject, "alice@example.com");
        assert_eq!(member.role, i32::from(WorkspaceRole::Admin));

        handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "bob@example.com".to_string(),
                role: WorkspaceRole::User.into(),
            }),
        )
        .await
        .unwrap();

        let list = handle_list_workspace_members(
            &state,
            Request::new(ListWorkspaceMembersRequest {
                workspace: "default".to_string(),
                limit: 100,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .into_inner();

        assert_eq!(list.members.len(), 2);
    }

    #[tokio::test]
    async fn remove_workspace_member() {
        let state = test_server_state().await;

        handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "charlie@example.com".to_string(),
                role: WorkspaceRole::User.into(),
            }),
        )
        .await
        .unwrap();

        let resp = handle_remove_workspace_member(
            &state,
            Request::new(RemoveWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "charlie@example.com".to_string(),
            }),
        )
        .await
        .unwrap()
        .into_inner();
        assert!(resp.removed);

        let list = handle_list_workspace_members(
            &state,
            Request::new(ListWorkspaceMembersRequest {
                workspace: "default".to_string(),
                limit: 100,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .into_inner();

        assert!(list.members.is_empty());
    }

    #[tokio::test]
    async fn add_duplicate_member_rejected() {
        let state = test_server_state().await;

        handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "dave@example.com".to_string(),
                role: WorkspaceRole::User.into(),
            }),
        )
        .await
        .unwrap();

        let err = handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "default".to_string(),
                principal_subject: "dave@example.com".to_string(),
                role: WorkspaceRole::Admin.into(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(err.code(), Code::AlreadyExists);
    }

    #[tokio::test]
    async fn delete_workspace_cleans_up_members() {
        let state = test_server_state().await;

        handle_create_workspace(
            &state,
            Request::new(CreateWorkspaceRequest {
                name: "cleanup-test".to_string(),
                labels: HashMap::new(),
            }),
        )
        .await
        .unwrap();

        handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "cleanup-test".to_string(),
                principal_subject: "alice@example.com".to_string(),
                role: WorkspaceRole::Admin.into(),
            }),
        )
        .await
        .unwrap();

        handle_add_workspace_member(
            &state,
            Request::new(AddWorkspaceMemberRequest {
                workspace: "cleanup-test".to_string(),
                principal_subject: "bob@example.com".to_string(),
                role: WorkspaceRole::User.into(),
            }),
        )
        .await
        .unwrap();

        let list = handle_list_workspace_members(
            &state,
            Request::new(ListWorkspaceMembersRequest {
                workspace: "cleanup-test".to_string(),
                limit: 100,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .into_inner();
        assert_eq!(list.members.len(), 2);

        let resp = handle_delete_workspace(
            &state,
            Request::new(DeleteWorkspaceRequest {
                name: "cleanup-test".to_string(),
            }),
        )
        .await
        .unwrap()
        .into_inner();
        assert!(resp.deleted);

        // Membership records should have been cleaned up.
        let remaining: Vec<WorkspaceMember> = state
            .store
            .list_messages("cleanup-test", 100, 0)
            .await
            .unwrap();
        assert!(
            remaining.is_empty(),
            "expected 0 orphaned members, found {}",
            remaining.len()
        );
    }

    #[test]
    fn validate_workspace_name_accepts_single_hyphens() {
        validate_workspace_name("my-workspace").unwrap();
    }

    #[test]
    fn validate_workspace_name_rejects_uppercase() {
        let err = validate_workspace_name("MyWorkspace").unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn validate_workspace_name_rejects_leading_hyphen() {
        let err = validate_workspace_name("-workspace").unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn validate_workspace_name_rejects_consecutive_hyphens() {
        let err = validate_workspace_name("team--ml").unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
