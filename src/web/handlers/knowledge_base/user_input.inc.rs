// 本片段经 `include!` 进入 `knowledge_base` handler 模块：用户输入面
//（GET /user-input、POST /user-input/confirm）与【知识库初始化】的两段式
// 分叉（REQ-KB-14/15）。两个新端点都经 `knowledge_base_read_context` 统一
// 受 REQ-KB-13 前置门约束（不落在集中门之外）。

use crate::product::logical_codebase::knowledge_base::user_input::{
    self, UserInputConfirmError, UserInputConfirmationStore, UserInputSummary,
};

// ───────────────────────── 稳定码 ─────────────────────────
//
// 注：两码的 HTTP 状态映射尚未在 `src/web/error.rs` 登记（该文件归
// controller 归口管理，本任务不改）；未登记码经 `IntoResponse` 的兜底分支
// 落 500。message 已内联中文可执行文案（不走 failure_copy 表，避免与并发
// 改动面冲突）；登记后即为 4xx 前置失败。

const CODE_USER_INPUT_NOT_CONFIRMED: &str = "knowledge_base_user_input_not_confirmed";
const CODE_USER_INPUT_CONFIRMATION_STALE: &str = "knowledge_base_user_input_confirmation_stale";

/// POST `/user-input/confirm` 请求体（双形态：单份 `{ path, digest }` 或批量
/// `{ all: true, items: [{ path, digest }] }`）。
#[derive(Debug, Deserialize)]
pub struct ConfirmKnowledgeBaseUserInputRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub all: Option<bool>,
    #[serde(default)]
    pub items: Option<Vec<ConfirmKnowledgeBaseUserInputItem>>,
}

/// 批量确认的单项。
#[derive(Debug, Deserialize)]
pub struct ConfirmKnowledgeBaseUserInputItem {
    pub path: String,
    pub digest: String,
}

// ───────────────────────── 错误与 DTO ─────────────────────────

/// 确认失败面：过期确认（stale）携带中文可执行文案与被拒路径；store 故障走
/// `product_store_api_error` 既有映射。
fn knowledge_base_user_input_confirm_api_error(error: UserInputConfirmError) -> ApiError {
    match error {
        UserInputConfirmError::Stale { path } => ApiError::runtime(
            CODE_USER_INPUT_CONFIRMATION_STALE,
            format!(
                "确认已过期：{path} 的当前内容与提交的摘要不一致（文件可能已被修改或删除）。\
                 请刷新读取最新内容后重新确认。"
            ),
            json!({ "path": path }),
        ),
        UserInputConfirmError::Store(store_error) => product_store_api_error(store_error),
    }
}

/// 「输入已准备但未全部确认」失败面：details 列出未确认文件与计数，供前端
/// 输入确认区逐份点名（REQ-KB-14 场景三）。
fn knowledge_base_user_input_not_confirmed_error(summary: &UserInputSummary) -> ApiError {
    let unconfirmed: Vec<&str> = summary
        .files
        .iter()
        .filter(|file| !file.confirmed)
        .map(|file| file.path.as_str())
        .collect();
    ApiError::runtime(
        CODE_USER_INPUT_NOT_CONFIRMED,
        format!(
            "知识库输入还有 {} 份文件未确认（{}）。请先在知识库详情页的输入确认区逐份\
             核对并确认全部输入文件，再执行知识库初始化。",
            unconfirmed.len(),
            unconfirmed.join("、")
        ),
        json!({
            "unconfirmed": unconfirmed,
            "confirmed_count": summary.confirmed_count,
            "total_count": summary.total_count,
        }),
    )
}

/// 汇总投影 → GET /user-input 响应 DTO（snake_case，与知识库面既有 DTO 同款）。
fn knowledge_base_user_input_summary_dto(summary: &UserInputSummary) -> Value {
    json!({
        "files": summary
            .files
            .iter()
            .map(|file| {
                json!({
                    "path": file.path,
                    "filled": file.filled,
                    "confirmed": file.confirmed,
                    "pending": file.pending,
                })
            })
            .collect::<Vec<_>>(),
        "confirmed_count": summary.confirmed_count,
        "total_count": summary.total_count,
    })
}

/// 模板基线根：owner HOME → 插件源根 → `knowledge-base-bootstrap/user-input`
///（与 Task 4 预检②/Task 5 模板同一解析口径；探测不到 ⇒ skills 不可用）。
fn knowledge_base_user_input_template_root() -> ApiResult<PathBuf> {
    let source_root = knowledge_base_skills_source_root()?;
    user_input::user_input_template_root(&source_root).map_err(|_| {
        ApiError::runtime(
            CODE_SKILLS_UNAVAILABLE,
            "the knowledge-base user-input template source is unavailable",
            json!({ "missing": [] }),
        )
    })
}

// ───────────────────────── 用户输入端点 ─────────────────────────

/// GET `/user-input`：9 份输入文件的状态投影（filled / confirmed / pending）
/// 与汇总。经 `knowledge_base_read_context` 统一受 REQ-KB-13 前置门约束。
pub async fn get_knowledge_base_user_input(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (paths, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let template_root = knowledge_base_user_input_template_root()?;
    let confirmations = UserInputConfirmationStore::for_lc(paths.clone(), lc_id.clone())
        .load(&project_id)
        .map_err(product_store_api_error)?;
    let summary = user_input::summarize_user_input(
        &kb_root(&canonical_root),
        &template_root,
        &confirmations,
    );
    Ok(Json(knowledge_base_user_input_summary_dto(&summary)).into_response())
}

/// POST `/user-input/confirm`：逐份/批量确认。服务端**重算**当前内容摘要与
/// 请求摘要比对，不一致 ⇒ `knowledge_base_user_input_confirmation_stale`
///（整批原子拒绝，不落任何记录）。确认记录只写平台侧 per-LC 存储，绝不写
/// 进 `user-input/`（skill 的生成后自检要求该目录只有 9 份输入文件）。
pub async fn confirm_knowledge_base_user_input(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Json(request): Json<ConfirmKnowledgeBaseUserInputRequest>,
) -> ApiResult<Response> {
    let (paths, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let items = knowledge_base_user_input_confirm_items(request)?;
    let store = UserInputConfirmationStore::for_lc(paths.clone(), lc_id.clone());
    store
        .confirm(&project_id, &kb_root(&canonical_root), &items)
        .map_err(knowledge_base_user_input_confirm_api_error)?;
    // 响应只报本轮确认成功的路径；完整徽标投影由客户端随后 GET /user-input
    // 获取（确认面不依赖模板源，避免无谓的 HOME 解析失败面）。
    let mut confirmed: Vec<&str> = items.iter().map(|(path, _)| path.as_str()).collect();
    confirmed.sort_unstable();
    confirmed.dedup();
    Ok(Json(json!({ "confirmed": confirmed })).into_response())
}

/// 请求体双形态解析：`{ all: true, items: [...] }` ⇒ 批量；`{ path, digest }`
/// ⇒ 单份；两者都缺/为空 ⇒ 400 `invalid_knowledge_base_request`。逐项路径先
/// 做 `user-input/<文件名>` 形状校验。
fn knowledge_base_user_input_confirm_items(
    request: ConfirmKnowledgeBaseUserInputRequest,
) -> ApiResult<Vec<(String, String)>> {
    let items: Vec<(String, String)> = if request.all.unwrap_or(false) {
        request
            .items
            .ok_or_else(|| knowledge_base_invalid_request("items is required when all is true"))?
            .into_iter()
            .map(|item| (item.path, item.digest))
            .collect()
    } else {
        let path = request
            .path
            .ok_or_else(|| knowledge_base_invalid_request("path and digest are required"))?;
        let digest = request
            .digest
            .ok_or_else(|| knowledge_base_invalid_request("path and digest are required"))?;
        vec![(path, digest)]
    };
    if items.is_empty() {
        return Err(knowledge_base_invalid_request(
            "confirm items must not be empty",
        ));
    }
    for (path, digest) in &items {
        user_input::validate_user_input_rel_path(path).map_err(knowledge_base_invalid_request)?;
        if digest.trim().is_empty() {
            return Err(knowledge_base_invalid_request(format!(
                "digest must not be empty for {path:?}"
            )));
        }
    }
    Ok(items)
}

// ───────────────────────── 两段式分叉 ─────────────────────────

/// 两段式分叉（REQ-KB-14，D6：后端按状态决定跑哪一段，前端只消费状态投影）：
/// 同一个【知识库初始化】意图（kind=bootstrap）——
/// - 输入**未准备**（9 份全缺失或全部仍是模板占位）⇒ 改跑**起草**
///   （`UserInput`，起草完成停在待确认，不执行 bootstrap）；
/// - 已准备但有未确认文件 ⇒ `knowledge_base_user_input_not_confirmed`
///   （details 列未确认文件），零 operation；
/// - 全部确认 ⇒ 维持 bootstrap。
///
/// update 与显式 user_input（详情页「重新生成草稿」入口）不走本分叉。
fn fork_knowledge_base_bootstrap_kind(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
    canonical_root: &StdPath,
) -> ApiResult<KnowledgeBaseOperationKind> {
    let template_root = knowledge_base_user_input_template_root()?;
    let confirmations = UserInputConfirmationStore::for_lc(paths.clone(), lc_id.to_string())
        .load(project_id)
        .map_err(product_store_api_error)?;
    let summary = user_input::summarize_user_input(
        &kb_root(canonical_root),
        &template_root,
        &confirmations,
    );
    if !summary.files.iter().any(|file| file.filled) {
        return Ok(KnowledgeBaseOperationKind::UserInput);
    }
    if summary.confirmed_count < summary.total_count {
        return Err(knowledge_base_user_input_not_confirmed_error(&summary));
    }
    Ok(KnowledgeBaseOperationKind::Bootstrap)
}
