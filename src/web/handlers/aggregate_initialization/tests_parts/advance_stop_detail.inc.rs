// 冷启动停止解释的**诊断串**口径：界面文案不得摊开原始 JSON。
//
// 旧实现把 provider 选择门的整个 `details`（候选表 + 单家核验入口 + copy）
// 序列化进诊断串，界面直接摊开一坨机器报文 —— 用户要的是「哪几家、什么
// 状态」，不是原始 JSON。机器字段仍留在 HTTP 响应体里给 agent 用。

/// 诊断串里有候选的紧凑中文行，且不含任何原始 JSON 结构。
#[test]
fn prepare_rejection_detail_renders_candidates_without_raw_json() {
    let error = crate::web::error::ApiError::runtime(
        "aggregate_initialization_provider_needs_verification",
        "已安装的 provider 都还没有核验",
        serde_json::json!({
            "copy": {"title_zh": "已安装的 provider 都还没有核验"},
            "candidates": [
                {
                    "provider": "pi",
                    "display_name": "Pi",
                    "cli_available": true,
                    "capability_state": "unverified",
                    "skipped_reason": "needs_verification",
                    "action_hint": "先核验 Pi 在本逻辑代码库的能力(一次真实探测),通过后重试本步"
                },
                {
                    "provider": "claude_code",
                    "display_name": "Claude Code",
                    "cli_available": true,
                    "capability_state": "unverified",
                    "skipped_reason": "needs_verification",
                    "action_hint": "先核验 Claude Code 在本逻辑代码库的能力(一次真实探测),通过后重试本步"
                }
            ],
            "revalidate": {"url": "/api/provider-capabilities/pi/revalidate"}
        }),
    );

    let detail = prepare_rejection_detail(&error).expect("诊断串");

    assert!(
        detail.contains("候选：Pi（未核验）、Claude Code（未核验）"),
        "诊断串必须给出逐家状态：{detail}"
    );
    assert!(
        !detail.contains('{') && !detail.contains('"'),
        "诊断串不得摊开原始 JSON：{detail}"
    );
    assert!(
        !detail.contains("| details:"),
        "诊断串不得保留旧的 JSON 拼接形态：{detail}"
    );
}

/// 没有候选表时：诊断串只保留错误 message（不追加任何机器字段）。
#[test]
fn prepare_rejection_detail_without_candidates_keeps_message_only() {
    let error = crate::web::error::ApiError::runtime(
        "aggregate_initialization_failed",
        "根配方准备工作失败",
        serde_json::json!({}),
    );

    let detail = prepare_rejection_detail(&error).expect("诊断串");

    assert_eq!(detail, "根配方准备工作失败");
}
