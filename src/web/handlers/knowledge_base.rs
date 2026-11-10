//! LC 知识库 HTTP 面（LC 知识库模块实施计划 Task 7）。
//!
//! 路由前缀 `/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base`：
//! operation 生命周期（create/active/get/cancel）、状态投影、文件读写面、
//! 变更包三端点与初始化输入模板。DTO 字段与前端
//! `web/src/api/types/knowledge-base.ts` 逐字对齐（snake_case）。
//!
//! 实现按仓内惯例拆 `include!` 片段守住 1200 行门禁（先例：
//! `aggregate_initialization.rs` + `aggregate_initialization/*.inc.rs`）。

use super::support::{product_app_paths, product_store_api_error, require_logical_codebase};
use super::*;

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::product::cadence_skills::CadenceSkillsPaths;
use crate::product::logical_codebase::aggregate_initialization::AggregateProviderSelection;
use crate::product::logical_codebase::knowledge_base::change_package::{
    ChangePackageFieldStatus, ChangePackageViolation, MemberRepoPaths, lint_change_package,
};
use crate::product::logical_codebase::knowledge_base::command::{
    bootstrap_command, update_command, user_input_command,
};
use crate::product::logical_codebase::knowledge_base::coordinator::{
    Clock as KnowledgeBaseClock, KnowledgeBaseCoordinator, KnowledgeBaseError,
};
use crate::product::logical_codebase::knowledge_base::files::{
    KnowledgeBaseFileError, kb_root, list_knowledge_base_files, read_knowledge_base_file,
    write_knowledge_base_file,
};
use crate::product::logical_codebase::knowledge_base::manifest::read_manifest;
use crate::product::logical_codebase::knowledge_base::operation::{
    KnowledgeBaseOperation, KnowledgeBaseOperationInput, KnowledgeBaseOperationKind,
    KnowledgeBaseOperationStatus, derive_operation_id,
};
use crate::product::logical_codebase::knowledge_base::preflight::{
    KnowledgeBasePreflightFailure, preflight_bootstrap, preflight_update, preflight_user_input,
};
use crate::product::logical_codebase::knowledge_base::store::KnowledgeBaseOperationStore;
use crate::product::logical_codebase::knowledge_base::templates::{
    create_bootstrap_input_templates, create_change_package_template,
};
use crate::product::logical_codebase::provider_selection::{
    ProviderSelectionOutcome, provider_from_wire, select_aggregate_provider_for_write,
};
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustPrecondition, ProviderTrustPreparationResult,
};
use crate::product::logical_codebase::store::LogicalCodebaseStore;
use crate::product::models::ProviderName;
use crate::web::state::{InitializationRunKey, InitializationRunRegistry, WebAppState};

include!("knowledge_base/dependencies.inc.rs");
include!("knowledge_base/handlers.inc.rs");
