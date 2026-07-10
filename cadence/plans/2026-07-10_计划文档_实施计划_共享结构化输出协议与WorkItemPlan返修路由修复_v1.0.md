# 共享结构化输出协议与 WorkItemPlan 返修路由修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Workspace Review 的流式结构化输出解析统一下沉到 Provider Adapter，增加一次可验证等价性的封装修复与清晰诊断，并确保 Work Item Plan Outline 在降级人工确认后仍进入专用返修链路。

**Architecture:** 新增共享 `structured_output` 模块，流式 Provider 通过 `ProviderCompletion` 返回原始文本、可读文本和类型化结构化状态；Workspace Engine 只做 Review 业务 Schema 校验、一次可验证封装修复编排和路由。诊断字段贯通 timeline persistence、WebSocket、刷新重建和前端 Review 卡片，Work Item Plan Outline 的 Human Confirm 修改请求复用专用 revision 入口。

**Tech Stack:** Rust 2024、Tokio、Serde/serde_json、Axum WebSocket、React 19、TypeScript、Zustand、Vitest、pnpm。

## Global Constraints

- 技术方案来源：`cadence/designs/2026-07-10_技术方案_共享结构化输出协议与WorkItemPlan返修路由修复_v1.2.md`。
- 必须直接使用宿主机 Rust 环境；禁止以 Docker 代替本地开发、测试和检查。
- Rust 工具链只以根目录 `rust-toolchain.toml` 为准。
- Cargo 命令禁止携带 `-j 1` 或其他显式 `-j`；并行度由 `.cargo/config.toml` 托管。
- Bug 修复必须遵循 TDD：先增加失败测试，确认失败，再实现最小代码并复测。
- 前端必须使用 `pnpm`；禁止使用 npm 或 yarn。
- 不新增第三方依赖。
- 保持首尾 nonce 严格一致；不得通过接受缺失结束 nonce 来“修复”本案例。
- 文本 sentinel 本期继续作为 Provider 内部兼容格式，不要求一次性切换原生 JSON Schema API。
- 不自动修改 `.aria` 中 `workspace_session_0003` 或其他业务数据。
- 通用 Review 行为必须覆盖 Story Spec、Design Spec、Work Item；Work Item Plan 另补 Outline/Item/Batch 专用覆盖。
- 新增持久化字段必须使用 `serde(default)` 保持旧 workspace 数据可恢复。
- 当前已有 `review/feedback.rs` 与 item/batch `plan_reopen_required` 修复必须保留，不得回退。

---

## File Structure

### 新建文件

- `src/cross_cutting/structured_output.rs`
  - 共享 sentinel/nonce/JSON 语法解析。
  - 定义 `StructuredOutputContract`、`StructuredOutputState`、`StructuredOutputError` 与错误码。
- `src/product/workspace_engine/review/structured_output.rs`
  - 将 `ProviderCompletion` 转换为可信 `ReviewVerdict`。
  - 定义 Review 业务 Schema 错误、诊断构造和 repair payload 等价性校验。
- `src/product/workspace_engine/prompts/review_repair.rs`
  - 构建一次性 reviewer 结构化格式修复 Prompt/Input。
- `web/src/components/chat-workspace/entries/StructuredOutputDiagnostic.tsx`
  - 展示修复成功提示或最终失败告警、Reviewer 可读说明和原始输出预览入口。
- `web/src/state/structured-output-diagnostic.ts`
  - 集中实现 WebSocket 与刷新重建共用的 runtime type guard。

### 主要修改文件

- `src/cross_cutting/mod.rs`
- `src/cross_cutting/provider_adapter.rs`
- `src/cross_cutting/streaming_provider/mod.rs`
- `src/cross_cutting/streaming_provider/fake.rs`
- `src/cross_cutting/streaming_provider/tests.rs`
- `src/cross_cutting/codex_provider/session.rs`
- `src/cross_cutting/codex_provider/tests.rs`
- `src/cross_cutting/claude_code_provider/mod.rs`
- `src/cross_cutting/claude_code_provider/stream.rs`
- `src/cross_cutting/claude_code_provider/tests/streaming.rs`
- `src/product/workspace_engine/prompts.rs`
- `src/product/workspace_engine/prompts/review.rs`
- `src/product/workspace_engine/review.rs`
- `src/product/workspace_engine/review/drive.rs`
- `src/product/workspace_engine/review/routing.rs`
- `src/product/workspace_engine/parsers.rs`
- `src/product/workspace_engine/decisions.rs`
- `src/product/workspace_engine/plan_outline/revision.rs`
- `src/product/workspace_engine/session_state.rs`
- `src/product/workspace_engine/types.rs`
- `src/web/workspace_ws_types/review.rs`
- `src/web/workspace_ws_types/out.rs`
- `src/web/workspace_ws_handler/mapping.rs`
- `web/src/api/types/workspace.ts`
- `web/src/state/workspace-ws-store-types.ts`
- `web/src/hooks/workspace-ws-message-handler.ts`
- `web/src/state/workspace-chat-rebuild.ts`
- `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`

### 测试重点

- `src/cross_cutting/structured_output.rs`
- `src/cross_cutting/streaming_provider/tests.rs`
- `src/cross_cutting/codex_provider/tests.rs`
- `src/cross_cutting/claude_code_provider/tests/streaming.rs`
- `src/product/workspace_engine/tests/part_03/part_01.rs`
- `src/product/workspace_engine/tests/part_03/part_03.rs`
- `src/product/workspace_engine/tests/part_03/part_05.rs`
- `src/product/workspace_engine/tests/part_08.rs`
- `src/product/workspace_engine/tests/part_10.rs`
- `src/web/workspace_ws_types/tests.rs`
- `tests/it_web/web_workspace_recovery_consistency/part_01.rs`
- `web/src/hooks/useWorkspaceWs.timeline.test.tsx`
- `web/src/state/workspace-ws-store.test.ts`
- `web/src/components/chat-workspace/entries/p1-entries.test.tsx`

---

### Task 1: 建立共享结构化输出语法解析器

**Files:**
- Create: `src/cross_cutting/structured_output.rs`
- Modify: `src/cross_cutting/mod.rs:1-28`
- Modify: `src/cross_cutting/provider_adapter.rs:1-330`
- Test: `src/cross_cutting/structured_output.rs`
- Test: `tests/it_provider/provider_adapter_baseline.rs`
- Test: `tests/it_provider/provider_error_routes.rs`

**Interfaces:**
- Consumes: 原始 Provider 文本和 Prompt 构建器提供的期望 nonce。
- Produces:
  - `StructuredOutputContract { nonce: String, schema_name: String }`
  - `StructuredOutputState::{NotRequested, Parsed(Value), Failed(StructuredOutputError)}`
  - `StructuredOutputParse { readable_output: String, state: StructuredOutputState }`
  - `parse_structured_output(output, contract)`
  - `parse_last_structured_output_value(output)`，供旧非流式 Adapter 复用。

- [ ] **Step 1: 写 nonce 与错误分类失败测试**

在新文件中先写表驱动测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract() -> StructuredOutputContract {
        StructuredOutputContract {
            nonce: "96aca42f".to_string(),
            schema_name: "workspace_review".to_string(),
        }
    }

    #[test]
    fn parses_matching_nonce_and_removes_structured_block_from_readable_output() {
        let output = "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"pass\"}\n</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        assert_eq!(parsed.readable_output, "审核说明");
        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn classifies_missing_end_nonce_and_keeps_recoverable_value() {
        let output = "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"revise\"}\n</ARIA_STRUCTURED_OUTPUT>";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::MissingEndNonce);
        assert_eq!(error.expected_nonce.as_deref(), Some("96aca42f"));
        assert_eq!(error.recoverable_value, Some(json!({"verdict": "revise"})));
        assert_eq!(parsed.readable_output, "审核说明");
    }

    #[test]
    fn classifies_nonce_mismatch() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::NonceMismatch);
        assert_eq!(error.observed_nonce.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn classifies_invalid_json_without_trusting_value() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{invalid}</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::InvalidJson);
        assert!(error.recoverable_value.is_none());
    }

    #[test]
    fn parses_fenced_json_inside_matching_nonce_block() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n```json\n{\"verdict\":\"pass\"}\n```\n</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run:

```bash
cargo test --locked --lib structured_output
```

Expected: FAIL，提示 `StructuredOutputContract`、`parse_structured_output` 等符号不存在。

- [ ] **Step 3: 实现共享类型和解析入口**

实现以下公开模型：

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

const START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
const END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputContract {
    pub nonce: String,
    pub schema_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredOutputState {
    NotRequested,
    Parsed(Value),
    Failed(StructuredOutputError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputError {
    pub code: StructuredOutputErrorCode,
    pub message: String,
    pub expected_nonce: Option<String>,
    pub observed_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputErrorCode {
    MissingStartTag,
    MissingEndTag,
    MissingEndNonce,
    NonceMismatch,
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputParse {
    pub readable_output: String,
    pub state: StructuredOutputState,
}

impl StructuredOutputErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingStartTag => "missing_start_tag",
            Self::MissingEndTag => "missing_end_tag",
            Self::MissingEndNonce => "missing_end_nonce",
            Self::NonceMismatch => "nonce_mismatch",
            Self::InvalidJson => "invalid_json",
        }
    }
}
```

`parse_structured_output()` 必须按以下顺序实现，不得以 `rfind` 任意接受其他 nonce：

```rust
pub fn parse_structured_output(
    output: &str,
    contract: &StructuredOutputContract,
) -> StructuredOutputParse {
    let expected_start = format!("{START_PREFIX} nonce=\"{}\">", contract.nonce);
    let Some(start) = output.rfind(&expected_start) else {
        return failed(
            output,
            StructuredOutputErrorCode::MissingStartTag,
            "missing structured output start tag for expected nonce",
            Some(contract.nonce.clone()),
            None,
            None,
        );
    };
    let json_start = start + expected_start.len();
    let after_start = &output[json_start..];
    let Some(end_relative) = after_start.find(END_PREFIX) else {
        return failed(
            output,
            StructuredOutputErrorCode::MissingEndTag,
            "missing structured output end tag",
            Some(contract.nonce.clone()),
            None,
            parse_json_candidate(after_start),
        );
    };
    let json_text = after_start[..end_relative].trim();
    let after_end_prefix = &after_start[end_relative + END_PREFIX.len()..];
    let Some(tag_close) = after_end_prefix.find('>') else {
        return failed(
            output,
            StructuredOutputErrorCode::MissingEndTag,
            "structured output end tag is not closed",
            Some(contract.nonce.clone()),
            None,
            parse_json_candidate(json_text),
        );
    };
    let attrs = after_end_prefix[..tag_close].trim();
    let end_tag_len = END_PREFIX.len() + tag_close + 1;
    let block_end = json_start + end_relative + end_tag_len;
    let readable_output = format!("{}{}", &output[..start], &output[block_end..])
        .trim()
        .to_string();
    if attrs.is_empty() {
        return StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::MissingEndNonce,
                message: "structured output end tag is missing nonce".to_string(),
                expected_nonce: Some(contract.nonce.clone()),
                observed_nonce: None,
                recoverable_value: parse_json_candidate(json_text),
            }),
        };
    }
    let observed_nonce = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))
        .map(str::to_string);
    if observed_nonce.as_deref() != Some(contract.nonce.as_str()) {
        return StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::NonceMismatch,
                message: "structured output nonce mismatch".to_string(),
                expected_nonce: Some(contract.nonce.clone()),
                observed_nonce,
                recoverable_value: parse_json_candidate(json_text),
            }),
        };
    }
    match parse_json_candidate(json_text) {
        Some(value) => StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Parsed(value),
        },
        None => StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::InvalidJson,
                message: "invalid structured output json".to_string(),
                expected_nonce: Some(contract.nonce.clone()),
                observed_nonce: Some(contract.nonce.clone()),
                recoverable_value: None,
            }),
        },
    }
}
```

补充私有 `failed()`、`extract_json_candidate()` 和 `parse_json_candidate()`：

```rust
fn parse_json_candidate(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let candidate = extract_json_candidate(text)?;
        serde_json::from_str(candidate).ok()
    })
}

fn extract_json_candidate(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let close = match text.as_bytes()[start] {
        b'{' => '}',
        b'[' => ']',
        _ => return None,
    };
    let end = text.rfind(close)?;
    (end >= start).then_some(&text[start..=end])
}
```

`failed()` 构造 `StructuredOutputParse`；若已找到目标 start tag 但结束边界不可信，`readable_output` 保留 start tag 之前的可读文本，否则保留整段输出。

- [ ] **Step 4: 让非流式 Adapter 复用共享解析器**

在 `structured_output.rs` 中抽出私有 `parse_block_at(output, start_index, expected_nonce)`，严格入口与旧入口共用它：

1. `parse_structured_output()` 只查找包含 contract nonce 的目标 start tag，并以 `Some(contract.nonce)` 调用 `parse_block_at()`。
2. `parse_last_structured_output_value()` 查找最后一个 start prefix；无 start 时返回 `Ok(None)`。
3. 旧入口解析 start tag 中的可选 nonce，然后以该 `Option<&str>` 调用 `parse_block_at()`；无 nonce 时要求 end tag 同样无 nonce，有 nonce 时要求首尾严格相等。
4. `parse_last_structured_output_value()` 把 `Parsed(value)` 转为 `Ok(Some(value))`，把 `Failed(error)` 转为 `Err(error)`。
5. `provider_adapter.rs::parse_last_structured_output()` 仅调用该函数，并把 typed error 映射为 `ProviderAdapterError::parse_error(error.message, stdout, "")`。

删除 `provider_adapter.rs` 中重复的 `find_structured_output_end()`、`parse_structured_output_tag()`、`parse_structured_output_nonce()` 与 JSON candidate helper。

- [ ] **Step 5: 运行定向测试**

Run:

```bash
cargo test --locked --lib structured_output
cargo test --locked --test it_provider provider_adapter
cargo test --locked --test it_provider provider_error_routes
```

Expected: PASS；错误路由仍映射到 `ProviderParseError`。

- [ ] **Step 6: 提交 Task 1**

```bash
git add src/cross_cutting/mod.rs src/cross_cutting/structured_output.rs src/cross_cutting/provider_adapter.rs tests/it_provider/provider_adapter_baseline.rs tests/it_provider/provider_error_routes.rs
git commit -m "refactor: unify structured output parsing"
```

---

### Task 2: 升级流式 Provider 完成事件协议

**Files:**
- Modify: `src/cross_cutting/streaming_provider/mod.rs:19-365`
- Modify: `src/cross_cutting/streaming_provider/tests.rs`
- Mechanical compatibility updates: all files returned by `rg -l 'ProviderEvent::Completed \{' src tests`，基线为以下 33 个文件：
  - `src/cross_cutting/claude_code_provider/stream.rs`
  - `src/cross_cutting/claude_code_provider/tests/ask_user_question.rs`
  - `src/cross_cutting/claude_code_provider/tests/mod.rs`
  - `src/cross_cutting/claude_code_provider/tests/process.rs`
  - `src/cross_cutting/claude_code_provider/tests/streaming.rs`
  - `src/cross_cutting/codex_provider/session.rs`
  - `src/cross_cutting/codex_provider/tests.rs`
  - `src/cross_cutting/streaming_provider/fake.rs`
  - `src/cross_cutting/streaming_provider/mod.rs`
  - `src/cross_cutting/streaming_provider/tests.rs`
  - `src/product/coding_workspace_engine/provider_stream.rs`
  - `src/product/coding_workspace_engine/testing_provider/execution.rs`
  - `src/product/coding_workspace_engine/tests/provider_driven.rs`
  - `src/product/workspace_engine/provider_drive.rs`
  - `src/product/workspace_engine/provider_drive/work_item_plan.rs`
  - `src/product/workspace_engine/review/drive.rs`
  - `src/product/workspace_engine/tests/part_01.rs`
  - `src/product/workspace_engine/tests/part_03/part_03.rs`
  - `src/product/workspace_engine/tests/part_05.rs`
  - `src/product/workspace_engine/tests/part_06.rs`
  - `src/product/workspace_engine/tests/part_07.rs`
  - `src/product/workspace_engine/tests/part_09.rs`
  - `src/product/workspace_engine/tests/part_10.rs`
  - `src/web/test_controls/provider.rs`
  - `src/web/test_controls/tests.rs`
  - `src/web/workspace_ws_handler/tests.rs`
  - `tests/it_core/workspace_ws_integration/part_04.rs`
  - `tests/it_core/workspace_ws_integration/part_05.rs`
  - `tests/it_product/product_coding_workspace_engine/part_09.rs`
  - `tests/it_product/product_coding_workspace_engine/part_10.rs`
  - `tests/it_product/product_tester_agent_loop.rs`
  - `tests/it_web/web_coding_ws_handler/part_06.rs`
  - `tests/it_web/web_work_item_generation/part_01.rs`

**Interfaces:**
- Consumes: Task 1 的 `StructuredOutputContract` 与 `StructuredOutputState`。
- Produces:
  - `ProviderCompletion`
  - `ProviderCompletion::plain()`
  - `ProviderCompletion::from_output()`
  - `ProviderEvent::Completed(ProviderCompletion)`
  - `StreamingProviderInput.structured_output_contract`

- [ ] **Step 1: 写 ProviderCompletion 失败测试**

在 `streaming_provider/tests.rs` 添加：

```rust
#[test]
fn provider_completion_parses_requested_structured_output() {
    let contract = StructuredOutputContract {
        nonce: "96aca42f".to_string(),
        schema_name: "workspace_review".to_string(),
    };
    let completion = ProviderCompletion::from_output(
        "可读说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">".to_string(),
        Some(&contract),
        Some("provider-session-1".to_string()),
    );

    assert_eq!(completion.readable_output, "可读说明");
    assert!(matches!(
        completion.structured_output,
        StructuredOutputState::Parsed(_)
    ));
    assert_eq!(completion.provider_session_id.as_deref(), Some("provider-session-1"));
}

#[test]
fn provider_completion_plain_marks_structured_output_not_requested() {
    let completion = ProviderCompletion::plain("plain output", None);

    assert_eq!(completion.full_output, "plain output");
    assert_eq!(completion.readable_output, "plain output");
    assert_eq!(completion.structured_output, StructuredOutputState::NotRequested);
}
```

- [ ] **Step 2: 运行测试并确认失败**

```bash
cargo test --locked --lib provider_completion
```

Expected: FAIL，`ProviderCompletion` 不存在。

- [ ] **Step 3: 实现完成结果和输入契约字段**

在 `streaming_provider/mod.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompletion {
    pub full_output: String,
    pub readable_output: String,
    pub structured_output: StructuredOutputState,
    pub provider_session_id: Option<String>,
}

impl ProviderCompletion {
    pub fn plain(
        full_output: impl Into<String>,
        provider_session_id: Option<String>,
    ) -> Self {
        let full_output = full_output.into();
        Self {
            readable_output: full_output.clone(),
            full_output,
            structured_output: StructuredOutputState::NotRequested,
            provider_session_id,
        }
    }

    pub fn from_output(
        full_output: String,
        contract: Option<&StructuredOutputContract>,
        provider_session_id: Option<String>,
    ) -> Self {
        let Some(contract) = contract else {
            return Self::plain(full_output, provider_session_id);
        };
        let parsed = parse_structured_output(&full_output, contract);
        Self {
            full_output,
            readable_output: parsed.readable_output,
            structured_output: parsed.state,
            provider_session_id,
    }
}
```

`StreamingProviderInput` 增加：

```rust
pub structured_output_contract: Option<StructuredOutputContract>,
```

`ProviderEvent` 改为：

```rust
Completed(ProviderCompletion),
```

`run_streaming()` bridge 保持旧 `StreamChunk::Done { full_output }`，匹配时只取 `completion.full_output`。

- [ ] **Step 4: 机械迁移全部完成事件构造与匹配**

生产 Provider 暂时使用：

```rust
ProviderEvent::Completed(ProviderCompletion::plain(full_output, provider_session_id))
```

测试 fixture 同样使用 `ProviderCompletion::plain()`。消费者改为：

```rust
ProviderEvent::Completed(completion) => {
    let full_output = completion.full_output;
}
```

所有 `StreamingProviderInput` 构造点先补：

```rust
structured_output_contract: None,
```

Task 3 再只对 Review Prompt 设置 `Some(contract)`。

- [ ] **Step 5: 使用编译器确认不存在旧形态**

```bash
rg -n 'ProviderEvent::Completed \{' src tests
cargo check --locked
```

Expected:

- `rg` 无输出并返回 1。
- `cargo check --locked` PASS。

- [ ] **Step 6: 运行流式基础测试**

```bash
cargo test --locked --lib provider_completion
cargo test --locked --lib streaming_provider
```

Expected: PASS；`StreamChunk::Done` 的 `full_output` 行为不变。

- [ ] **Step 7: 提交 Task 2**

```bash
git add \
  src/cross_cutting/claude_code_provider/stream.rs \
  src/cross_cutting/claude_code_provider/tests/ask_user_question.rs \
  src/cross_cutting/claude_code_provider/tests/mod.rs \
  src/cross_cutting/claude_code_provider/tests/process.rs \
  src/cross_cutting/claude_code_provider/tests/streaming.rs \
  src/cross_cutting/codex_provider/session.rs \
  src/cross_cutting/codex_provider/tests.rs \
  src/cross_cutting/streaming_provider/fake.rs \
  src/cross_cutting/streaming_provider/mod.rs \
  src/cross_cutting/streaming_provider/tests.rs \
  src/product/coding_workspace_engine/coding.rs \
  src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/provider_stream.rs \
  src/product/coding_workspace_engine/testing_provider/execution.rs \
  src/product/coding_workspace_engine/testing_provider/plan.rs \
  src/product/coding_workspace_engine/testing_provider/report.rs \
  src/product/coding_workspace_engine/tests/provider_driven.rs \
  src/product/workspace_engine/prompts.rs \
  src/product/workspace_engine/prompts/review.rs \
  src/product/workspace_engine/prompts/revision.rs \
  src/product/workspace_engine/provider_drive.rs \
  src/product/workspace_engine/provider_drive/artifact_retry.rs \
  src/product/workspace_engine/provider_drive/work_item_plan.rs \
  src/product/workspace_engine/review/drive.rs \
  src/product/workspace_engine/tests/part_01.rs \
  src/product/workspace_engine/tests/part_03/part_03.rs \
  src/product/workspace_engine/tests/part_05.rs \
  src/product/workspace_engine/tests/part_06.rs \
  src/product/workspace_engine/tests/part_07.rs \
  src/product/workspace_engine/tests/part_09.rs \
  src/product/workspace_engine/tests/part_10.rs \
  src/web/state.rs \
  src/web/test_controls/provider.rs \
  src/web/test_controls/tests.rs \
  src/web/workspace_ws_handler/tests.rs \
  tests/it_core/workspace_ws_integration/part_04.rs \
  tests/it_core/workspace_ws_integration/part_05.rs \
  tests/it_product/product_coding_workspace_engine/part_09.rs \
  tests/it_product/product_coding_workspace_engine/part_10.rs \
  tests/it_product/product_tester_agent_loop.rs \
  tests/it_web/web_coding_ws_handler/part_06.rs \
  tests/it_web/web_work_item_generation/part_01.rs
git commit -m "refactor: carry provider completion state"
```

---

### Task 3: 让 Codex、Claude Code、Fake Provider 产出结构化完成结果

**Files:**
- Modify: `src/cross_cutting/codex_provider/session.rs:25-315`
- Modify: `src/cross_cutting/codex_provider/tests.rs:43-86`
- Create: `tests/fixtures/provider/codex_app_server_structured_output_fixture.sh`
- Modify: `src/cross_cutting/claude_code_provider/mod.rs:374-520`
- Modify: `src/cross_cutting/claude_code_provider/stream.rs:35-380`
- Modify: `src/cross_cutting/claude_code_provider/tests/streaming.rs`
- Modify: `src/cross_cutting/streaming_provider/fake.rs:18-73`
- Modify: `src/cross_cutting/streaming_provider/tests.rs`
- Modify: `src/product/workspace_engine/prompts/review.rs:3-605`
- Test: `src/product/workspace_engine/tests/part_08.rs`

**Interfaces:**
- Consumes: `ProviderCompletion::from_output()`、`StreamingProviderInput.structured_output_contract`。
- Produces: 所有 reviewer Provider 在 `Completed` 时都带 `Parsed/Failed` 状态；五种 review input 带确定的 schema name 和 nonce。

- [ ] **Step 1: 写 Codex/Claude/Fake 失败测试**

创建 `tests/fixtures/provider/codex_app_server_structured_output_fixture.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.133.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_structured_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_structured_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"message_001","text":"审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"pass\",\"summary\":\"审核通过\",\"findings\":[]}</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">","phase":"final_answer"},"threadId":"codex_structured_thread","turnId":"codex_structured_turn"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex_structured_thread","turn":{"id":"codex_structured_turn","status":"completed"}}}'
    exit 0
  fi
done
```

Codex 测试使用该 fixture，并给 input 设置 `nonce=96aca42f`：

```bash
chmod +x tests/fixtures/provider/codex_app_server_structured_output_fixture.sh
```

```rust
let completion = recv_completion(&mut session.events).await;
assert_eq!(completion.readable_output, "审核说明");
assert!(matches!(
    completion.structured_output,
    StructuredOutputState::Parsed(ref value)
        if value["verdict"] == "pass"
));
```

Claude 测试通过现有 `write_fixture()` 创建：

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"result","subtype":"success","is_error":false,"result":"审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"revise\",\"summary\":\"需要修正\",\"findings\":[]}</ARIA_STRUCTURED_OUTPUT>","session_id":"claude_structured_session"}'
    exit 0
  fi
done
```

断言：

```rust
let completion = recv_completion(&mut session.events).await;
let StructuredOutputState::Failed(error) = completion.structured_output else {
    panic!("expected structured output failure");
};
assert_eq!(error.code, StructuredOutputErrorCode::MissingEndNonce);
assert_eq!(completion.readable_output, "审核说明");
```

Fake Provider 测试给 `StreamingProviderInput` 设置同一 contract，Prompt 使用会生成 reviewer sentinel 的 fixture 输入，并断言 `Parsed`。

- [ ] **Step 2: 运行 Provider 测试确认失败**

```bash
cargo test --locked --lib codex_provider_carries_structured_completion
cargo test --locked --lib claude_provider_classifies_missing_end_nonce
cargo test --locked --lib fake_streaming_provider_parses_requested_structured_output
```

Expected: FAIL，当前三个 Provider 仍使用 `ProviderCompletion::plain()`。

- [ ] **Step 3: Codex completion 使用输入契约**

在 `run_codex_session()` 的 turn completed 分支中改为：

```rust
let completion = ProviderCompletion::from_output(
    full_output,
    input.structured_output_contract.as_ref(),
    thread_id,
);
send_provider_event(
    &event_tx,
    ProviderEvent::Completed(completion),
    &cancel,
)
.await?;
```

- [ ] **Step 4: Claude stream 透传输入契约**

`read_claude_stream()` 增加参数：

```rust
structured_output_contract: Option<StructuredOutputContract>,
```

Claude `start()` 在进入 spawn 前克隆：

```rust
let structured_output_contract = input.structured_output_contract.clone();
```

完成时构造：

```rust
let completion = ProviderCompletion::from_output(
    full_output,
    structured_output_contract.as_ref(),
    provider_session_id,
);
send_provider_event(
    &event_tx,
    ProviderEvent::Completed(completion),
    &cancel,
)
.await?;
```

- [ ] **Step 5: Fake Provider 使用相同构造器**

Fake `start()` 在 spawn 前保存 contract，并发送：

```rust
let completion = ProviderCompletion::from_output(
    output,
    input.structured_output_contract.as_ref(),
    None,
);
ProviderEvent::Completed(completion)
```

- [ ] **Step 6: Review Prompt 同时返回文本契约和输入契约**

每个 review builder 在生成 nonce 后设置：

```rust
structured_output_contract: Some(StructuredOutputContract {
    nonce,
    schema_name: "workspace_review".to_string(),
}),
```

具体映射：

- 通用 Story/Design/Work Item：`workspace_review`
- 整组旧 WorkItemPlan candidate：`work_item_plan_review`
- Outline：`work_item_plan_outline_review`
- Batch：`work_item_plan_batch_review`
- Item：`work_item_plan_item_review`

为了避免 nonce move 后 Prompt 无法使用，调用顺序固定为：

```rust
let nonce = structured_output_nonce();
let structured_output_contract = StructuredOutputContract {
    nonce: nonce.clone(),
    schema_name: "workspace_review".to_string(),
};
prompt.push_str(&reviewer_output_contract(&nonce, schema, intro));
```

- [ ] **Step 7: 增加 Prompt contract 测试**

在 `part_08.rs` 对五类输入断言：

```rust
let contract = input
    .structured_output_contract
    .expect("review input should carry structured output contract");
assert!(input.prompt.contains(&format!("nonce=\"{}\"", contract.nonce)));
assert_eq!(contract.schema_name, "work_item_plan_outline_review");
```

- [ ] **Step 8: 运行 Provider 与 Prompt 测试**

```bash
cargo test --locked --lib codex_provider_carries_structured_completion
cargo test --locked --lib claude_provider_classifies_missing_end_nonce
cargo test --locked --lib fake_streaming_provider_parses_requested_structured_output
cargo test --locked --lib build_work_item_plan_outline_review_input
cargo test --locked --lib build_work_item_plan_review_input
```

Expected: PASS。

- [ ] **Step 9: 提交 Task 3**

```bash
git add \
  src/cross_cutting/codex_provider/session.rs \
  src/cross_cutting/codex_provider/tests.rs \
  src/cross_cutting/claude_code_provider/mod.rs \
  src/cross_cutting/claude_code_provider/stream.rs \
  src/cross_cutting/claude_code_provider/tests/streaming.rs \
  src/cross_cutting/streaming_provider/fake.rs \
  src/cross_cutting/streaming_provider/tests.rs \
  src/product/workspace_engine/prompts/review.rs \
  src/product/workspace_engine/tests/part_08.rs \
  tests/fixtures/provider/codex_app_server_structured_output_fixture.sh
git commit -m "feat: parse structured output in streaming providers"
```

---

### Task 4: 将 ProviderCompletion 转换为可信 ReviewVerdict 与类型化诊断

**Files:**
- Create: `src/product/workspace_engine/review/structured_output.rs`
- Modify: `src/product/workspace_engine/review.rs`
- Modify: `src/product/workspace_engine/parsers.rs:65-430`
- Modify: `src/product/workspace_engine/review/routing.rs:4-470`
- Modify: `src/product/workspace_engine/session_state.rs:254-309`
- Modify: `src/product/workspace_engine/types.rs:135-213`
- Modify: `src/web/workspace_ws_types/review.rs:1-125`
- Modify: `src/web/workspace_ws_types/out.rs:13-90`
- Modify: `src/web/workspace_ws_types/tests.rs`
- Modify: `src/web/workspace_ws_handler/mapping.rs:288-310`
- Mechanical literal updates: files returned by `rg -l 'ReviewVerdict \{' src/product/workspace_engine src/web tests`
- Test: `src/product/workspace_engine/tests/part_10.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_01.rs`

**Interfaces:**
- Consumes: `ProviderCompletion` 和 `StructuredOutputState`。
- Produces:
  - `ReviewStructuredOutputErrorCode`
  - `ReviewCompletionError`
  - `StructuredOutputDiagnostic`
  - `parse_review_completion_for_active_node()`
  - `fallback_review_verdict()`
  - `complete_review(completion, verdict)`，不再自行解析原始文本。

- [ ] **Step 1: 写跨 Workspace Value 解析失败测试**

在 `part_10.rs` 新增表驱动测试：

```rust
#[test]
fn generic_workspace_review_parses_provider_structured_value_for_all_artifact_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let store = Arc::new(MemoryWorkspaceCheckpointStore::default());
        let (tx, _rx) = mpsc::channel(16);
        let mut session = base_session();
        session.workspace_type = workspace_type;
        let engine = WorkspaceEngine::new(store, tx, session);
        let completion = ProviderCompletion {
            full_output: "审核通过".to_string(),
            readable_output: "审核通过".to_string(),
            structured_output: StructuredOutputState::Parsed(serde_json::json!({
                "verdict": "pass",
                "summary": "审核通过",
                "findings": []
            })),
            provider_session_id: None,
        };

        let verdict = engine
            .parse_review_completion_for_active_node(&completion)
            .expect("structured review should parse");
        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
        assert_eq!(verdict.comments, "审核通过");
    }
}
```

再增加：

- missing end nonce → `ReviewCompletionError::Syntax`。
- malformed findings → `ReviewCompletionError::Schema(MalformedFindings)`。
- invalid outline reference → `InvalidOutlineReference`。

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test --locked --lib generic_workspace_review_parses_provider_structured_value
cargo test --locked --lib review_completion_reports_syntax_error
cargo test --locked --lib work_item_plan_review_reports_invalid_outline_reference
```

Expected: FAIL，尚无 completion-to-verdict API。

- [ ] **Step 3: Review parser 改为直接消费 Value**

把现有字符串入口拆为：

```rust
pub(crate) fn parse_review_value(
    value: &serde_json::Value,
    comments: &str,
) -> Result<ReviewVerdict, ReviewStructuredOutputErrorCode>;

pub(crate) fn parse_work_item_plan_review_value(
    value: &serde_json::Value,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Result<ReviewVerdict, ReviewStructuredOutputErrorCode>;
```

映射规则：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewStructuredOutputErrorCode {
    MissingVerdict,
    InvalidVerdict,
    MalformedFindings,
    InvalidOutlineReference,
    InvalidGenerationRound,
}
```

旧 `parse_review_json()` 和 `parse_work_item_plan_review_json()` 只作为测试/兼容 wrapper：先 `serde_json::from_str`，再调用 Value 入口。生产 Review 完成路径不得调用 `extract_structured_json()`。

业务校验规则固定为：

- `verdict` 缺失或非字符串 → `MissingVerdict`；不在当前 schema 枚举中 → `InvalidVerdict`。
- `findings` 可缺失，缺失等价于空数组；若存在，必须是数组，且每项都必须有合法 `severity` 和字符串 `message`，任一项不合法整个结果返回 `MalformedFindings`，不保留部分 findings。
- Work Item Plan `target_outline_id` 若存在必须属于 `valid_outline_ids`；`affects_items` 保留当前兼容阈值，无效引用超过半数时返回 `InvalidOutlineReference`，否则仅保留合法项并记录 warnings。
- Work Item Plan `generation_round_id` 必须是非空字符串，否则返回 `InvalidGenerationRound`；本任务不新增跨存储查询比对，避免改变现有 Outline 尚未建立 active index 时的行为。
- 只有完成上述校验后才调用现有 `review_gate_for()` / `work_item_plan_review_routing()` 计算路由。

- [ ] **Step 4: 增加持久化诊断字段**

在 `workspace_ws_types/review.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputDiagnostic {
    pub code: String,
    pub message: String,
    pub repair_attempted: bool,
    pub repair_succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output_preview: Option<String>,
}
```

`raw_output_preview` 必须通过 `preview()` 截断到 2048 字符，不得直接持久化 `ProviderCompletion.full_output`。

`ReviewVerdict` 增加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub structured_output_diagnostic: Option<StructuredOutputDiagnostic>,
```

所有现有 Rust struct literal 显式补 `structured_output_diagnostic: None`。

`EngineEvent::ReviewComplete` 与 `WsOutMessage::ReviewComplete` 增加同名可选字段；`review_complete_event_from_verdict()` 透传。

`src/web/workspace_ws_handler/mapping.rs` 的 `EngineEvent::ReviewComplete -> WsOutMessage::ReviewComplete` match 同步透传，避免 Engine 事件已有诊断但 WebSocket 层丢字段。

- [ ] **Step 5: 实现 completion-to-verdict API**

在 `review/structured_output.rs` 定义：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCompletionError {
    Syntax(StructuredOutputError),
    Schema(ReviewStructuredOutputErrorCode),
    NotRequested,
}

impl WorkspaceEngine {
    pub(crate) fn parse_review_completion_for_active_node(
        &self,
        completion: &ProviderCompletion,
    ) -> Result<ReviewVerdict, ReviewCompletionError> {
        let StructuredOutputState::Parsed(value) = &completion.structured_output else {
            return match &completion.structured_output {
                StructuredOutputState::Failed(error) => {
                    Err(ReviewCompletionError::Syntax(error.clone()))
                }
                StructuredOutputState::NotRequested => Err(ReviewCompletionError::NotRequested),
                StructuredOutputState::Parsed(_) => unreachable!(),
            };
        };
        if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            let scope = match self.active_node_type() {
                Some(TimelineNodeType::WorkItemPlanOutlineReview) => {
                    WorkItemPlanReviewScope::Outline
                }
                Some(TimelineNodeType::WorkItemDraftReview) => WorkItemPlanReviewScope::Item,
                Some(TimelineNodeType::WorkItemBatchReview) => WorkItemPlanReviewScope::Batch,
                _ => WorkItemPlanReviewScope::Batch,
            };
            return parse_work_item_plan_review_value(
                value,
                &completion.readable_output,
                &self.current_work_item_plan_outline_ids(),
                scope,
            )
            .map_err(ReviewCompletionError::Schema);
        }
        parse_review_value(value, &completion.readable_output)
            .map_err(ReviewCompletionError::Schema)
    }
}
```

- [ ] **Step 6: complete_review 不再解析文本**

签名改为：

```rust
pub(crate) async fn complete_review(
    &mut self,
    completion: ProviderCompletion,
    verdict: ReviewVerdict,
) {
    let node_id = self
        .active_node_id
        .clone()
        .unwrap_or_else(|| "review_unknown".to_string());
    let round = self.active_review_round().unwrap_or(1);
    let active_node_type = self.active_node_type();
    self.record_review_message(completion.readable_output);
    self.latest_review_verdict = Some(verdict.clone());
    let reviewer = self
        .active_node_agent()
        .or_else(|| self.session.reviewer_provider.clone());
    let _ = self
        .persist_review_verdict(
            &node_id,
            serde_json::to_value(&verdict).unwrap_or(serde_json::Value::Null),
        )
        .await;
    let _ = self
        .event_tx
        .send(review_complete_event_from_verdict(
            node_id.clone(),
            round,
            &verdict,
        ))
        .await;
    self.update_timeline_node(
        &node_id,
        TimelineNodeStatus::Completed,
        Some(verdict.summary.clone()),
    )
    .await;
    let artifact_verdict = match &verdict.review_gate {
        ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
        ReviewGate::UserConfirmAllowed if verdict.verdict == ReviewVerdictType::Pass => {
            ReviewVerdictType::Pass
        }
        ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
            ReviewVerdictType::NeedsHuman
        }
    };
    self.mark_latest_artifact_reviewed(reviewer, Some(artifact_verdict));

    match active_node_type {
        Some(TimelineNodeType::WorkItemPlanOutlineReview) => {
            self.route_work_item_plan_outline_review(verdict).await;
        }
        Some(TimelineNodeType::WorkItemDraftReview) => {
            self.route_work_item_draft_review(verdict).await;
        }
        Some(TimelineNodeType::WorkItemBatchReview) => {
            self.route_work_item_batch_review(verdict).await;
        }
        _ => match &verdict.review_gate {
            ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                self.enter_human_confirm(Some(verdict.summary.clone())).await;
            }
            ReviewGate::RequiresRevision => {
                self.enter_review_decision(round, verdict.summary.clone()).await;
            }
        },
    }
}
```

删除 `complete_review()` 内部 `parse_review_verdict_for_active_node(&output)` 调用。原有静态 parser wrappers 保留给兼容测试，但不得出现在 `review/drive.rs` 生产路径。

- [ ] **Step 7: 测试 Serde 与 WebSocket 向后兼容**

在 `workspace_ws_types/tests.rs` 添加：

```rust
let old: ReviewVerdict = serde_json::from_value(json!({
    "verdict": "needs_human",
    "comments": "旧数据",
    "summary": "旧数据",
    "findings": [],
    "review_gate": "user_triage_required"
}))
.unwrap();
assert!(old.structured_output_diagnostic.is_none());
```

并断言新 `review_complete` JSON 包含 `structured_output_diagnostic.code`。

- [ ] **Step 8: 运行定向测试**

```bash
cargo test --locked --lib generic_workspace_review_parses_provider_structured_value
cargo test --locked --lib review_completion_reports_syntax_error
cargo test --locked --lib work_item_plan_review_reports_invalid_outline_reference
cargo test --locked --lib workspace_ws_types
```

Expected: PASS。

- [ ] **Step 9: 提交 Task 4**

```bash
git add \
  src/product/workspace_engine/review.rs \
  src/product/workspace_engine/review/structured_output.rs \
  src/product/workspace_engine/review/routing.rs \
  src/product/workspace_engine/parsers.rs \
  src/product/workspace_engine/session_state.rs \
  src/product/workspace_engine/types.rs \
  src/product/workspace_engine/decisions.rs \
  src/product/workspace_engine/review/feedback.rs \
  src/product/workspace_engine/tests/part_02.rs \
  src/product/workspace_engine/tests/part_03/part_01.rs \
  src/product/workspace_engine/tests/part_03/part_02.rs \
  src/product/workspace_engine/tests/part_03/part_05.rs \
  src/product/workspace_engine/tests/part_04.rs \
  src/product/workspace_engine/tests/part_06.rs \
  src/product/workspace_engine/tests/part_09.rs \
  src/product/workspace_engine/tests/part_10.rs \
  src/web/workspace_ws_types/review.rs \
  src/web/workspace_ws_types/out.rs \
  src/web/workspace_ws_types/tests.rs \
  src/web/workspace_ws_handler/mapping.rs
git commit -m "feat: validate provider review completions"
```

---

### Task 5: 增加一次性 Reviewer 结构化格式修复

**Files:**
- Create: `src/product/workspace_engine/prompts/review_repair.rs`
- Modify: `src/product/workspace_engine/prompts.rs:1-85`
- Modify: `src/product/workspace_engine/review/structured_output.rs`
- Modify: `src/product/workspace_engine/review/drive.rs:3-390`
- Modify: `src/product/workspace_engine/review/routing.rs:4-70`
- Modify: `src/product/workspace_engine/types.rs:328-349`
- Test: `src/product/workspace_engine/tests/part_03/part_03.rs`
- Test: `src/product/workspace_engine/tests/part_10.rs`

**Interfaces:**
- Consumes: Task 4 的 `ReviewCompletionError`。
- Produces:
  - `build_review_repair_input()`
  - `drive_reviewer_provider_session_once()`
  - `ReviewProviderRunResult`
  - 一次 repair 编排与 `structured_output_repair` execution event。

- [ ] **Step 1: 写双调用 Provider 失败测试**

在 `part_03/part_03.rs` 增加一个按顺序返回结果的 Provider：

```rust
#[derive(Clone)]
struct QueuedReviewProvider {
    outputs: Arc<Mutex<VecDeque<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for QueuedReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(input.prompt.clone());
        let output = self.outputs.lock().unwrap().pop_front().expect("queued output");
        let completion = ProviderCompletion::from_output(
            output,
            input.structured_output_contract.as_ref(),
            Some(format!("review-session-{}", self.starts.load(Ordering::SeqCst))),
        );
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        event_tx
            .send(ProviderEvent::Completed(completion))
            .await
            .unwrap();
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

测试一：首次缺失结束 nonce，第二次合法，断言：

- Provider start 次数为 2。
- timeline 只有一个业务 Review node/round。
- 第二个 Prompt 包含 `missing_end_nonce`、原始输出和“不得改变 verdict/findings/summary”。
- 最终 verdict 是 `revise`，findings 完整。
- 同一 Review node 的 `structured_output_repair` execution event 最终状态为 `completed`。

测试二：两次都失败，断言最终 `needs_human` 且 diagnostic：

```rust
assert_eq!(diagnostic.code, "missing_end_nonce");
assert!(diagnostic.repair_attempted);
assert!(!diagnostic.repair_succeeded);
```

并断言 `structured_output_repair` execution event 最终状态为 `failed`。

测试三：第一次 error 含 `recoverable_value`，第二次返回不同 verdict，断言 diagnostic code 为 `repair_payload_changed`，不得采用第二次结果。

测试四：首次为 `invalid_json` 且无 `recoverable_value`，断言 Provider start 次数为 1，直接得到 `needs_human + invalid_json`。

测试五：首次 JSON 可解析但 findings 形状非法，断言 Provider start 次数为 1，直接得到 `needs_human + malformed_findings`。

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test --locked --lib review_structured_output_repair_succeeds_without_new_round
cargo test --locked --lib review_structured_output_repair_failure_persists_diagnostic
cargo test --locked --lib review_structured_output_repair_rejects_payload_change
cargo test --locked --lib invalid_review_json_does_not_trigger_unverifiable_repair
cargo test --locked --lib malformed_review_findings_do_not_trigger_business_rewrite
```

Expected: FAIL；当前 reviewer session 只运行一次。

- [ ] **Step 3: 实现修复 Prompt**

`review_repair.rs` 提供：

```rust
impl WorkspaceEngine {
    pub(crate) fn build_review_repair_input(
        &self,
        base_input: &StreamingProviderInput,
        completion: &ProviderCompletion,
        error: &ReviewCompletionError,
        provider_session_id: Option<String>,
    ) -> Result<StreamingProviderInput, String> {
        let nonce = structured_output_nonce();
        let recoverable_value = error
            .recoverable_value()
            .ok_or_else(|| "structured output repair requires recoverable JSON".to_string())?;
        let recoverable_json = serde_json::to_string_pretty(recoverable_value)
            .map_err(|error| format!("serialize recoverable review JSON failed: {error}"))?;
        let schema_name = base_input
            .structured_output_contract
            .as_ref()
            .map(|contract| contract.schema_name.clone())
            .unwrap_or_else(|| "workspace_review".to_string());
        let prompt = format!(
            "上一轮审核业务内容已经完成，但结构化输出格式无效。\n\
             只能修复 JSON 与 ARIA_STRUCTURED_OUTPUT 封装；不得重新审核，不得改变 verdict、summary、findings、review_scope、affects_items 或其他业务字段。\n\
             schema_name: {}\n\
             error_code: {}\n\
             已恢复的原业务 JSON（必须逐字段保持语义一致）：\n{}\n\
             原始输出：\n{}\n\
             请只返回以下 nonce block，不要输出其他说明：\n\
             <ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{{修复后的原业务 JSON}}\n</ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n",
            schema_name.as_str(),
            error.code(),
            recoverable_json,
            completion.full_output,
            nonce,
            nonce,
        );
        Ok(StreamingProviderInput {
            provider_type: base_input.provider_type.clone(),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir: base_input.working_dir.clone(),
            workspace_session_id: base_input.workspace_session_id.clone(),
            resume_provider_session_id: provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: base_input.env_vars.clone(),
            timeout_secs: base_input.timeout_secs,
            structured_output_contract: Some(StructuredOutputContract {
                nonce,
                schema_name,
            }),
        })
    }
}
```

- [ ] **Step 4: 把 reviewer session 驱动拆成单次执行**

新增结果：

```rust
pub(crate) enum ReviewProviderRunResult {
    Completed(ProviderCompletion),
    Terminal,
}

pub(crate) fn structured_output_repair_event(
    status: ProviderExecutionEventStatus,
    error_code: &str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: "structured_output_repair".to_string(),
        kind: ProviderExecutionEventKind::Output,
        status,
        title: "Structured output repair".to_string(),
        detail: Some(format!("repair reviewer structured output: {error_code}")),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}
```

将现有 `drive_reviewer_provider_session()` 改名/拆分为：

```rust
pub(crate) async fn drive_reviewer_provider_session_once(
    &mut self,
    session: Result<ProviderSession, ProviderAdapterError>,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    reviewer: &ProviderName,
) -> ReviewProviderRunResult;
```

该函数保留 permission、choice、tool、stream、abort、failure 全部处理，但收到 `Completed(completion)` 时：

- flush stream。
- record provider session。
- 空输出走原有 empty output 失败。
- 返回 `ReviewProviderRunResult::Completed(completion)`。
- 不调用 `complete_review()`。

- [ ] **Step 5: 在 drive_review_session 中编排一次 repair**

逻辑固定为：

```rust
let mut command_rx = command_rx;
let first_session = provider.start(base_input.clone(), self.cancel.clone()).await;
let ReviewProviderRunResult::Completed(first_completion) = self
    .drive_reviewer_provider_session_once(first_session, &mut command_rx, &reviewer)
    .await
else {
    return;
};
match self.parse_review_completion_for_active_node(&first_completion) {
    Ok(verdict) => self.complete_review(first_completion, verdict).await,
    Err(first_error) if first_error.is_repairable() => {
        self.emit_execution_event(
            structured_output_repair_event(
                ProviderExecutionEventStatus::Started,
                first_error.code(),
            ),
            self.active_node_id.clone(),
            Some(reviewer.clone()),
        )
        .await;
        let repair_input = match self.build_review_repair_input(
            &base_input,
            &first_completion,
            &first_error,
            first_completion.provider_session_id.clone(),
        ) {
            Ok(input) => input,
            Err(_) => {
                let verdict = fallback_review_verdict(
                    &first_completion,
                    &first_error,
                    false,
                );
                self.complete_review(first_completion, verdict).await;
                return;
            }
        };
        let repair_session = provider.start(repair_input, self.cancel.clone()).await;
        let ReviewProviderRunResult::Completed(repaired_completion) = self
            .drive_reviewer_provider_session_once(repair_session, &mut command_rx, &reviewer)
            .await
        else {
            return;
        };
        match self.parse_review_completion_for_active_node(&repaired_completion) {
            Ok(mut verdict) if repair_payload_is_compatible(&first_error, &repaired_completion) => {
                verdict.structured_output_diagnostic = Some(success_diagnostic(&first_error));
                let normalized = ProviderCompletion {
                    full_output: format!("{}\n{}", first_completion.full_output, repaired_completion.full_output),
                    readable_output: first_completion.readable_output,
                    structured_output: repaired_completion.structured_output,
                    provider_session_id: repaired_completion.provider_session_id,
                };
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Completed,
                        first_error.code(),
                    ),
                    self.active_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                self.complete_review(normalized, verdict).await;
            }
            Ok(_) => {
                let error = ReviewCompletionError::RepairPayloadChanged;
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Failed,
                        error.code(),
                    ),
                    self.active_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                let verdict = fallback_review_verdict(&first_completion, &error, true);
                self.complete_review(first_completion, verdict).await;
            }
            Err(second_error) => {
                let normalized = ProviderCompletion {
                    full_output: format!("{}\n{}", first_completion.full_output, repaired_completion.full_output),
                    readable_output: first_completion.readable_output,
                    structured_output: repaired_completion.structured_output,
                    provider_session_id: repaired_completion.provider_session_id,
                };
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Failed,
                        second_error.code(),
                    ),
                    self.active_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                let verdict = fallback_review_verdict(&normalized, &second_error, true);
                self.complete_review(normalized, verdict).await;
            }
        }
    }
    Err(error) => {
        let verdict = fallback_review_verdict(&first_completion, &error, false);
        self.complete_review(first_completion, verdict).await;
    }
}
```

Task 5 将 `ReviewCompletionError` 增加 `RepairPayloadChanged` 变体，并实现：

```rust
pub(crate) fn fallback_review_verdict(
    completion: &ProviderCompletion,
    error: &ReviewCompletionError,
    repair_attempted: bool,
) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: completion.readable_output.clone(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: error.code().to_string(),
            message: error.message(),
            repair_attempted,
            repair_succeeded: false,
            raw_output_preview: Some(preview(&completion.full_output)),
        }),
    }
}
```

`ReviewCompletionError` 实现固定映射：

```rust
impl ReviewCompletionError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Syntax(error) => error.code.as_str(),
            Self::Schema(code) => code.as_str(),
            Self::NotRequested => "structured_output_not_requested",
            Self::RepairPayloadChanged => "repair_payload_changed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Syntax(error) => error.message.clone(),
            Self::Schema(code) => code.message().to_string(),
            Self::NotRequested => "review input did not request structured output".to_string(),
            Self::RepairPayloadChanged => {
                "structured output repair changed the review payload".to_string()
            }
        }
    }

    pub(crate) fn recoverable_value(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Syntax(error) => error.recoverable_value.as_ref(),
            Self::Schema(_) | Self::NotRequested | Self::RepairPayloadChanged => None,
        }
    }

    pub(crate) fn is_repairable(&self) -> bool {
        matches!(
            self,
            Self::Syntax(error)
                if error.recoverable_value.is_some()
                    && matches!(
                        error.code,
                        StructuredOutputErrorCode::MissingEndTag
                            | StructuredOutputErrorCode::MissingEndNonce
                            | StructuredOutputErrorCode::NonceMismatch
                    )
        )
    }
}
```

`ReviewStructuredOutputErrorCode` 同样实现 `as_str()` 和 `message()`，映射到下划线错误码与稳定的中文诊断文案。

`is_repairable()` 只对“已有合法 `recoverable_value`”的以下 code 返回 true：

- `missing_end_tag`
- `missing_end_nonce`
- `nonce_mismatch`

`invalid_json`、业务 Schema 错误和 `NotRequested` 不触发 repair，直接生成 `needs_human` 诊断。固定最多一次，不使用循环。

- [ ] **Step 6: 实现 payload 等价保护**

```rust
pub(crate) fn repair_payload_is_compatible(
    first_error: &ReviewCompletionError,
    repaired: &ProviderCompletion,
) -> bool {
    let Some(expected) = first_error.recoverable_value() else {
        return false;
    };
    matches!(
        &repaired.structured_output,
        StructuredOutputState::Parsed(actual) if actual == expected
    )
}
```

缺失结束 nonce 案例必须走严格相等；没有 recoverable value 时不得进入 repair。

修复成功时的诊断不带 raw preview：

```rust
pub(crate) fn success_diagnostic(
    first_error: &ReviewCompletionError,
) -> StructuredOutputDiagnostic {
    StructuredOutputDiagnostic {
        code: first_error.code().to_string(),
        message: first_error.message(),
        repair_attempted: true,
        repair_succeeded: true,
        raw_output_preview: None,
    }
}
```

增加两个安全边界断言：

- `invalid_json` 只启动一次 Provider，diagnostic code 为 `invalid_json`。
- `MalformedFindings` 只启动一次 Provider，diagnostic code 为 `malformed_findings`。
- 失败诊断的 `raw_output_preview.chars().count() <= 2048`。

- [ ] **Step 7: 运行 repair 测试**

```bash
cargo test --locked --lib review_structured_output_repair_succeeds_without_new_round
cargo test --locked --lib review_structured_output_repair_failure_persists_diagnostic
cargo test --locked --lib review_structured_output_repair_rejects_payload_change
cargo test --locked --lib invalid_review_json_does_not_trigger_unverifiable_repair
cargo test --locked --lib malformed_review_findings_do_not_trigger_business_rewrite
cargo test --locked --lib part_03
```

Expected: PASS。

- [ ] **Step 8: 提交 Task 5**

```bash
git add \
  src/product/workspace_engine/prompts.rs \
  src/product/workspace_engine/prompts/review_repair.rs \
  src/product/workspace_engine/review/structured_output.rs \
  src/product/workspace_engine/review/drive.rs \
  src/product/workspace_engine/review/routing.rs \
  src/product/workspace_engine/types.rs \
  src/product/workspace_engine/tests/part_03/part_03.rs \
  src/product/workspace_engine/tests/part_10.rs
git commit -m "feat: repair malformed reviewer output once"
```

---

### Task 6: 修复 Work Item Plan Outline Human Confirm 专用返修并增强影响面闭合 Prompt

**Files:**
- Modify: `src/product/workspace_engine/plan_outline/revision.rs:94-330`
- Modify: `src/product/workspace_engine/decisions.rs:180-450`
- Modify: `src/product/workspace_engine/types.rs:215-231`
- Test: `src/product/workspace_engine/tests/part_03/part_01.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_05.rs`
- Test: `src/product/workspace_engine/tests/part_09.rs`
- Test: `tests/it_web/web_workspace_recovery_consistency/part_01.rs`

**Interfaces:**
- Consumes: 当前 artifact、持久化 timeline node type、existing `format_review_feedback()`。
- Produces:
  - `WorkItemPlanOutlineRevisionSource`
  - `prepare_work_item_plan_outline_revision()`
  - `human_confirm_should_revise_work_item_plan_outline()`
  - 影响面闭合 Prompt contract。

- [ ] **Step 1: 写 Human Confirm 错路由失败测试**

构造：

- Workspace type = `WorkItemPlan`。
- 当前 artifact = `WorkItemPlanOutlineCandidate`。
- timeline 中最近完成 review = `WorkItemPlanOutlineReview`。
- active node = `HumanConfirm`。
- latest verdict 为降级 `needs_human`，`work_item_plan_review=None`，diagnostic=`missing_end_nonce`。

执行：

```rust
let outcome = engine
    .handle_human_confirm(HumanConfirmDecision::RequestChange, None)
    .await
    .expect("outline human confirm change should start outline revision");

assert!(matches!(
    outcome,
    ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
));
assert_eq!(engine.session().stage, WorkspaceStage::Running);
assert!(!engine.timeline_nodes.iter().any(|node| {
    node.node_type == TimelineNodeType::Revision && node.status == TimelineNodeStatus::Active
}));
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test --locked --lib work_item_plan_outline_human_confirm_change_uses_outline_revision
```

Expected: FAIL，实际返回 `StartRevision` 并创建 generic Revision node。

- [ ] **Step 3: 抽取统一 Outline revision 入口**

定义来源：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkItemPlanOutlineRevisionSource {
    AuthorConfirm,
    ReviewDecision,
    HumanConfirm,
}
```

抽取：

```rust
pub(crate) async fn prepare_work_item_plan_outline_revision(
    &mut self,
    feedback: Option<String>,
    source: WorkItemPlanOutlineRevisionSource,
) -> Result<Option<String>, String> {
    let outline_feedback = self.work_item_plan_outline_revision_feedback(feedback.as_deref());
    self.pending_revision_context = feedback;
    self.mark_latest_artifact_rejected();
    self.mark_work_item_plan_outline_revising()?;
    let summary = match source {
        WorkItemPlanOutlineRevisionSource::AuthorConfirm => "已请求重写 WorkItemPlan Outline",
        WorkItemPlanOutlineRevisionSource::ReviewDecision => "已选择返修 WorkItemPlan Outline",
        WorkItemPlanOutlineRevisionSource::HumanConfirm => "人工确认已转 WorkItemPlan Outline 返修",
    };
    self.complete_active_node(Some(summary.to_string())).await;
    if let Some(store) = &self.lifecycle_store {
        let _ = store.update_workspace_session_status(
            &self.session.session_id,
            WorkspaceSessionStatus::Open,
        );
    }
    self.transition_stage(WorkspaceStage::Running).await;
    self.work_item_plan_author_retry_count = 0;
    self.work_item_plan_revision_retry_count = 0;
    Ok(outline_feedback)
}
```

Outline Confirm Reject、Author Confirm 中的 `request_work_item_plan_revision`、Review Decision、Human Confirm 统一调用该方法。调用方各自映射 outcome：

```rust
let feedback = self
    .prepare_work_item_plan_outline_revision(context, source)
    .await?;
Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback })
```

`handle_work_item_plan_outline_decision(Reject)` 在调用后继续执行 `begin_work_item_plan_outline_run().await`，并保持对外返回 `AuthorDecisionOutcome::HumanConfirm`。

- [ ] **Step 4: 增加基于持久化状态的判定**

```rust
pub(crate) fn human_confirm_should_revise_work_item_plan_outline(&self) -> bool {
    if self.session.workspace_type != WorkspaceType::WorkItemPlan
        || !self.current_artifact_is_work_item_plan_outline_candidate()
    {
        return false;
    }
    self.timeline_nodes
        .iter()
        .rev()
        .find(|node| {
            node.status == TimelineNodeStatus::Completed
                && matches!(
                    node.node_type,
                    TimelineNodeType::WorkItemPlanOutlineReview
                        | TimelineNodeType::WorkItemDraftReview
                        | TimelineNodeType::WorkItemBatchReview
                )
        })
        .is_some_and(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
}
```

`handle_human_confirm(RequestChange)` 在 generic revision 前判断并调用专用入口。

- [ ] **Step 5: 增加影响面闭合反馈契约**

`work_item_plan_outline_revision_feedback()` 在存在 required finding 时追加固定文本：

```text
[impact_closure_contract]
当 finding 涉及 API 契约、共享状态或测试迁移时：
1. 不得只修改 reviewer 点名的文件。
2. 必须重新检索 src/**、tests/it_web/**、tests/it_core/**、tests/it_product/**、web/src/**。
3. 必须为每个 matched file 声明 owner，或明确无需修改的原因。
4. 返修摘要必须包含 searched_scopes、matched_files、owner_mapping。
```

追加条件为：

```rust
let requires_impact_closure = verdict.findings.iter().any(is_required_finding)
    || verdict
        .structured_output_diagnostic
        .as_ref()
        .is_some_and(|diagnostic| !diagnostic.repair_succeeded);
```

因此至少一个 finding severity 为 blocking/must_fix/strong_recommend_fix，或 Outline Review 最终结构化解析失败时追加。optional-only verdict 仍不追加；diagnostic 路径只追加通用检索/owner 契约，不从 raw preview 恢复 findings。

- [ ] **Step 6: 写 Prompt 断言**

在 `part_09.rs` 断言：

```rust
assert!(feedback.contains("[impact_closure_contract]"));
assert!(feedback.contains("tests/it_core/**"));
assert!(feedback.contains("owner_mapping"));
```

optional-only verdict 断言不包含该 contract。

再构造 `findings=[] + missing_end_nonce diagnostic` 的 Outline fallback verdict，断言仍包含 `[impact_closure_contract]`，且 feedback 中不出现 raw preview 里的未校验 finding 文本。

- [ ] **Step 7: 写刷新恢复路由测试**

在 `web_workspace_recovery_consistency/part_01.rs`：

1. 持久化 Outline Review → Human Confirm timeline。
2. 重新构建 engine。
3. 发送 RequestChange。
4. 断言响应触发 `WorkItemPlanOutlineRevision`，且无 generic Revision node。

- [ ] **Step 8: 运行定向测试**

```bash
cargo test --locked --lib work_item_plan_outline_human_confirm_change_uses_outline_revision
cargo test --locked --lib work_item_plan_outline_revision_feedback
cargo test --locked --test it_web web_workspace_recovery_consistency
```

Expected: PASS。

- [ ] **Step 9: 提交 Task 6**

```bash
git add \
  src/product/workspace_engine/plan_outline/revision.rs \
  src/product/workspace_engine/decisions.rs \
  src/product/workspace_engine/types.rs \
  src/product/workspace_engine/tests/part_03/part_01.rs \
  src/product/workspace_engine/tests/part_03/part_05.rs \
  src/product/workspace_engine/tests/part_09.rs \
  tests/it_web/web_workspace_recovery_consistency/part_01.rs
git commit -m "fix: keep outline human revision on outline path"
```

---

### Task 7: 贯通 WebSocket、刷新重建与前端诊断展示

**Files:**
- Modify: `web/src/api/types/workspace.ts:240-430`
- Modify: `web/src/state/workspace-ws-store-types.ts:154-181`
- Modify: `web/src/hooks/workspace-ws-message-handler.ts:293-325`
- Modify: `web/src/state/workspace-chat-rebuild.ts:250-278`
- Create: `web/src/state/structured-output-diagnostic.ts`
- Create: `web/src/components/chat-workspace/entries/StructuredOutputDiagnostic.tsx`
- Modify: `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx:1-185`
- Test: `web/src/hooks/useWorkspaceWs.timeline.test.tsx`
- Test: `web/src/state/workspace-ws-store.test.ts`
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`

**Interfaces:**
- Consumes: Rust `ReviewComplete.structured_output_diagnostic` 与 NodeDetail verdict JSON。
- Produces: TypeScript `StructuredOutputDiagnostic`、实时和刷新一致的 chat metadata、诊断组件。

- [ ] **Step 1: 写实时消息映射失败测试**

在 `useWorkspaceWs.timeline.test.tsx` 发送：

```typescript
harness.ws.receive({
  type: "review_complete",
  node_id: "timeline_node_review",
  round: 2,
  verdict: "needs_human",
  comments: "Reviewer 已完成审核。",
  summary: "需要人工确认",
  findings: [],
  review_gate: "user_triage_required",
  structured_output_diagnostic: {
    code: "missing_end_nonce",
    message: "structured output end tag is missing nonce",
    repair_attempted: true,
    repair_succeeded: false,
    raw_output_preview: "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{invalid}</ARIA_STRUCTURED_OUTPUT>",
  },
});
```

断言 review chat entry metadata 完整保留 diagnostic。

- [ ] **Step 2: 写刷新重建失败测试**

在 `workspace-ws-store.test.ts` 构造 hydrated node detail verdict，断言重建后的 `review_verdict` entry metadata 与实时消息相同。

- [ ] **Step 3: 写 UI 失败测试**

在 `p1-entries.test.tsx` 增加：

```typescript
it("renders structured output failure without trusting raw findings", () => {
  const entry = makeEntry({
    type: "review_verdict",
    role: "reviewer",
    content: "需要人工确认",
    metadata: {
      verdict: "needs_human",
      comments: "Reviewer 发现两个真实缺口。",
      summary: "需要人工确认",
      review_gate: "user_triage_required",
      findings: [],
      structured_output_diagnostic: {
        code: "missing_end_nonce",
        message: "结束标签缺少 nonce",
        repair_attempted: true,
        repair_succeeded: false,
        raw_output_preview: "{\"findings\":[{\"message\":\"未校验内容\"}]}",
      },
    },
  });

  render(<ReviewVerdictEntry entry={entry} />);

  expect(screen.getByText("结构化审核结果解析失败")).toBeInTheDocument();
  expect(screen.getByText("结束标签缺少 nonce")).toBeInTheDocument();
  expect(screen.getByText("系统已自动修复 1 次，仍未成功。")).toBeInTheDocument();
  expect(screen.getByText("Reviewer 发现两个真实缺口。")).toBeInTheDocument();
  expect(screen.queryAllByTestId("review-finding")).toHaveLength(0);
  expect(screen.getByTestId("structured-output-raw-preview")).toHaveTextContent(
    "未校验内容",
  );
});
```

另加 repair success 测试，显示低调提示“结构化输出已自动修复”，不显示失败告警。

- [ ] **Step 4: 运行前端测试确认失败**

```bash
pnpm --dir web test -- src/hooks/useWorkspaceWs.timeline.test.tsx src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx
```

Expected: FAIL，diagnostic 尚未映射和渲染。

- [ ] **Step 5: 增加 TypeScript 类型**

在 API/store types 中统一定义：

```typescript
export type StructuredOutputDiagnostic = {
  code: string;
  message: string;
  repair_attempted: boolean;
  repair_succeeded: boolean;
  raw_output_preview?: string | null;
};
```

`ReviewVerdict` 与 `review_complete` 增加：

```typescript
structured_output_diagnostic?: StructuredOutputDiagnostic | null;
```

在 `web/src/state/structured-output-diagnostic.ts` 实现共用边界校验：

```typescript
export function structuredOutputDiagnosticFromUnknown(
  value: unknown,
): StructuredOutputDiagnostic | undefined {
  if (!value || typeof value !== "object") return undefined;
  const candidate = value as Record<string, unknown>;
  if (
    typeof candidate.code !== "string" ||
    typeof candidate.message !== "string" ||
    typeof candidate.repair_attempted !== "boolean" ||
    typeof candidate.repair_succeeded !== "boolean"
  ) {
    return undefined;
  }
  if (
    candidate.raw_output_preview !== undefined &&
    candidate.raw_output_preview !== null &&
    typeof candidate.raw_output_preview !== "string"
  ) {
    return undefined;
  }
  return candidate as StructuredOutputDiagnostic;
}
```

- [ ] **Step 6: 实时和重建路径透传 diagnostic**

`workspace-ws-message-handler.ts`：

```typescript
const diagnostic = structuredOutputDiagnosticFromUnknown(
  msg.structured_output_diagnostic,
);
```

同时写入 node verdict 和 chat metadata。

`workspace-chat-rebuild.ts` 从 `detail.verdict.structured_output_diagnostic` 读取并写入相同 metadata key。

- [ ] **Step 7: 实现诊断组件**

组件 props：

```typescript
export function StructuredOutputDiagnosticView({
  diagnostic,
  comments,
}: {
  diagnostic: StructuredOutputDiagnostic;
  comments: string | null;
})
```

行为：

- `repair_succeeded=true`：显示“结构化输出已自动修复”，不展示失败告警。
- `repair_succeeded=false`：琥珀告警、错误信息、repair 状态。
- Reviewer comments 使用 `<details>` 展开。
- raw output preview 使用第二个 `<details>`，渲染纯文本 `<pre data-testid="structured-output-raw-preview">`。
- finding 行增加 `data-testid="review-finding"`，测试明确证明未校验 JSON 只出现在原始输出预览，不进入可信 findings。
- raw output 中的 JSON 不进入 findings 组件。

`ReviewVerdictEntry` 在 findings 前渲染该组件；当 diagnostic 存在时，不再渲染原有常驻 comments `<div>`，避免 Reviewer 说明重复出现。

- [ ] **Step 8: 运行前端测试与类型检查**

```bash
pnpm --dir web test -- src/hooks/useWorkspaceWs.timeline.test.tsx src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx
pnpm --dir web tsc -b
```

Expected: PASS。

- [ ] **Step 9: 提交 Task 7**

```bash
git add \
  web/src/api/types/workspace.ts \
  web/src/state/workspace-ws-store-types.ts \
  web/src/hooks/workspace-ws-message-handler.ts \
  web/src/state/workspace-chat-rebuild.ts \
  web/src/state/structured-output-diagnostic.ts \
  web/src/components/chat-workspace/entries/StructuredOutputDiagnostic.tsx \
  web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx \
  web/src/hooks/useWorkspaceWs.timeline.test.tsx \
  web/src/state/workspace-ws-store.test.ts \
  web/src/components/chat-workspace/entries/p1-entries.test.tsx
git commit -m "feat: show structured review diagnostics"
```

---

### Task 8: 恢复一致性、全量验证与当前案例验收准备

**Files:**
- Modify: `tests/it_web/web_workspace_recovery_consistency/part_01.rs`
- Modify: `tests/it_web/web_work_item_plan_mode/part_01.rs`
- Modify: `src/product/workspace_engine/tests/part_10.rs`
- No `.aria` business data modifications.

**Interfaces:**
- Consumes: Tasks 1-7 的最终协议。
- Produces: 刷新恢复、WS contract、Outline fallback 和共享 Workspace Review 的最终验收证据。

- [ ] **Step 1: 增加实时与恢复一致性集成测试**

测试流程：

1. Fake reviewer 首次返回缺少结束 nonce。
2. repair 再次失败。
3. 捕获实时 `review_complete`。
4. 重新加载 session state。
5. 比较实时与恢复字段：

```rust
assert_eq!(
    live_review["structured_output_diagnostic"],
    restored_detail["verdict"]["structured_output_diagnostic"]
);
assert_eq!(restored_detail["verdict"]["findings"], json!([]));
```

- [ ] **Step 2: 增加 Work Item Plan Outline WS 路由集成测试**

在 Human Confirm 发送 RequestChange 后断言：

- 启动 `ProviderRunKind::WorkItemPlanOutlineRevision`。
- 下一 active node 为 `work_item_plan_outline_run`。
- 不出现 `revision` node。
- Prompt 包含 `[impact_closure_contract]`。

- [ ] **Step 3: 运行全部定向 Rust 回归**

```bash
cargo test --locked --lib structured_output
cargo test --locked --lib streaming_provider
cargo test --locked --lib part_03
cargo test --locked --lib part_08
cargo test --locked --lib part_09
cargo test --locked --lib part_10
cargo test --locked --test it_web web_workspace_recovery_consistency
cargo test --locked --test it_web web_work_item_plan_mode
```

Expected: 全部 PASS。

- [ ] **Step 4: 运行前端回归**

```bash
pnpm --dir web test -- src/hooks/useWorkspaceWs.timeline.test.tsx src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx
pnpm --dir web tsc -b
pnpm --dir web build
```

Expected: 全部 PASS；允许保留现有 Vite chunk size warning，不允许出现 TypeScript 或测试失败。

- [ ] **Step 5: 运行标准 Rust 门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 PASS，零 warning，命令不含 `-j 1`。

- [ ] **Step 6: 检查共享三模块影响**

汇报中逐项说明：

- Story Spec：通用 `workspace_review` Parsed/Failed/repair 已覆盖。
- Design Spec：通用 `workspace_review` Parsed/Failed/repair 已覆盖。
- Work Item：通用 `workspace_review` Parsed/Failed/repair 已覆盖。
- Work Item Plan：Outline/Item/Batch 业务 Schema、repair、diagnostic 与专用 revision 已覆盖。

- [ ] **Step 7: 提交 Task 8**

```bash
git add \
  src/product/workspace_engine/tests/part_10.rs \
  tests/it_web/web_workspace_recovery_consistency/part_01.rs \
  tests/it_web/web_work_item_plan_mode/part_01.rs
git commit -m "test: cover structured review recovery"
```

- [ ] **Step 8: 准备用户手工验收步骤**

实现完成并重新启动服务后，仅指导用户操作，不自动修改当前业务数据：

1. 打开 `/workbench/workspace/workspace_session_0003`。
2. 对失败会话使用 UI 提供的重试/重新生成 Outline 能力；若当前 session 无恢复入口，则创建新的 Work Item Plan Workspace 复现相同 Story/Design 输入。
3. 验证 reviewer 缺失结束 nonce 时：
   - 系统自动 repair 一次。
   - repair 成功则展示正常 findings 与“已自动修复”提示。
   - repair 失败则展示明确错误，不再只显示“需要人工确认”。
4. 选择“请求修改”，确认进入新的 `WorkItemPlan Outline 生成` 节点，而不是 `返修 Round 1` generic revision。
5. 检查下一轮 Outline 包含：
   - `tests/it_core/workspace_ws_integration/part_05.rs` owner。
   - fake runtime BootstrapGuard 的内存健康覆盖责任。
   - `searched_scopes`、`matched_files`、`owner_mapping`。

---

## Final Acceptance Checklist

- [ ] `ProviderEvent::Completed` 统一携带 `ProviderCompletion`。
- [ ] Workspace Review 生产路径不再调用 `extract_structured_json(full_output)`。
- [ ] 缺少结束 nonce 被分类为 `missing_end_nonce`。
- [ ] 存在 recoverable JSON 的封装 repair 最多执行一次，不创建额外业务 Review Round。
- [ ] recoverable JSON 与 repair JSON 不一致时转人工确认。
- [ ] diagnostic 实时消息、落盘和刷新恢复一致。
- [ ] 前端不把未校验 raw JSON 渲染为可信 findings。
- [ ] Work Item Plan Outline Human Confirm 修改请求不再进入 generic Revision。
- [ ] impact closure Prompt 覆盖 `tests/it_core/**` 和 owner mapping。
- [ ] Story、Design、Work Item、Work Item Plan 共享回归均通过。
- [ ] Rust 与前端标准验证全部通过。
