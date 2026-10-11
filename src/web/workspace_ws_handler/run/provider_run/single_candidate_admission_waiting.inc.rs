/// C-1：LC admission waiting 的上浮消息——明确「等待而非终态失败」语义，
/// 携带预检判别码、缺失材料与允许动作，用户补齐材料后可重新发起生成。
/// 6.1（方案 A 附带）：`manual_guidance` 非空时（trust 缺失）附同源只读
/// 人工登记指导（provider 特定工件路径/trust 键/人工形状＋生成场景
/// retry_action）；其它 waiting 不携带，原投影不变。
fn format_single_candidate_admission_waiting(
    reason_code: &str,
    detail: &str,
    missing_materials: &[String],
    allowed_actions: &[crate::product::logical_codebase::BootstrapActionKind],
    manual_guidance: Option<&str>,
) -> String {
    let mut message = format!(
        "SingleCandidate LC 准入预检未通过（reason_code={reason_code}）：{detail}。\
         会话保持 waiting/Prepare 面（未终态失败），补齐材料后可重新发起生成"
    );
    if !missing_materials.is_empty() {
        message.push_str(&format!("；缺失材料：{}", missing_materials.join("；")));
    }
    if !allowed_actions.is_empty() {
        let actions: Vec<&str> = allowed_actions
            .iter()
            .map(|action| action.as_str())
            .collect();
        message.push_str(&format!("；允许动作：{}", actions.join("/")));
    }
    if let Some(guidance) = manual_guidance {
        message.push_str(&format!("；人工登记指导：{guidance}"));
    }
    message
}
