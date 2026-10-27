//! REQ-PRB-03：自动续写重试（整体替换 prompt）同样携带边界块，且至多一次。
//! Review Focus #1（resume 场景）：resume 会话的新一轮 prompt 仍须带边界块。

use super::*;

const RUN_BOUNDARY_HEADING: &str =
    "## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）";

#[test]
fn artifact_retry_prompt_carries_run_boundary_block_once() {
    let (_tmp, _store, engine) = persistent_test_engine();
    let base = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("author prompt");
    assert_eq!(
        base.prompt.matches(RUN_BOUNDARY_HEADING).count(),
        1,
        "漏斗已注入一次"
    );

    let retry = engine.build_artifact_retry_input(
        &base,
        "上一轮输出（缺章节）",
        &["缺少验收标准章节".to_string()],
        None,
    );
    assert_eq!(
        retry.prompt.matches(RUN_BOUNDARY_HEADING).count(),
        1,
        "整体替换后仍恰一次"
    );
}

#[test]
fn artifact_retry_resume_prompt_still_carries_run_boundary() {
    let (_tmp, _store, engine) = persistent_test_engine();
    let base = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("author prompt");
    let previous_output = "辅助草稿曾写入 /tmp/work-item-plan.md，可直接读取复用。";
    let retry = engine.build_artifact_retry_input(
        &base,
        previous_output,
        &["缺少验收标准章节".to_string()],
        Some("thread_resume_1".to_string()),
    );
    assert_eq!(
        retry.resume_provider_session_id.as_deref(),
        Some("thread_resume_1")
    );
    assert_eq!(
        retry.prompt.matches(RUN_BOUNDARY_HEADING).count(),
        1,
        "resume 轮仍须带边界块"
    );
}
