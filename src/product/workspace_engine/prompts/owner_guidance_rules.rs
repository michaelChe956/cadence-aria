/// REQ-WSC-11（owner 可读的人工裁决）：所有 reviewer 输出契约共享的唯一
/// 口径——`needs_human` 结论会被原样呈给用户，用户必须能读懂并在不写技术
/// prompt 的前提下作答。措辞只此一份，避免各 builder 漂出「一类有人话、
/// 一类没有」的分叉。
pub(crate) const OWNER_GUIDANCE_RULES: &str = "\
 面向用户的人工裁决（`needs_human`）呈现硬约束：\n\
 - 只要输出 `verdict` 为 `needs_human`，必须至少有一条 finding 同时填写 \
 `owner_question` 与 `owner_options`；缺失即被判为结构化输出不合规。\n\
 - `owner_question`：一句话说清「现在卡在哪 + 为什么需要你来定」，用日常中文。\
 禁止出现内部字段名、枚举值、指纹、追踪 ID 这类术语（例如 \
 `write_policy`、`exclusive_scopes`、`done_when_refs`、`AC-001`、`REQ-*`）。\n\
 - `owner_options`：2–3 个互相排斥的选项，每项写 `label`（选项名，用日常中文）与 \
 `consequence`（选它之后会发生什么）；不要用技术术语充当选项名。\n\
 - 输出 `pass` 或 `revise` 时，不要填写这两个字段（留空即可）。\n";

/// REQ-WSC-12/14（owner 2026-10-30 裁定，取舍 1=B / 取舍 2=A）：所有 reviewer
/// 输出契约共享的唯一 finding 范围与收敛口径——finding 只能落在本 Work Item
/// 的写面内，范围外事实降级 advisory；已被 owner 处置的事实不得重复报为
/// actionable。措辞只此一份，避免七个 builder 漂出分叉。
pub(crate) const FINDING_SCOPE_RULES: &str = "\
 finding 范围与收敛硬约束：\n\
 - 每条 finding 必须显式声明 `disposition`：`actionable`（本 Work Item 可以通过修订自己收敛）\
 或 `advisory`（只作提示）。平台只把 `actionable` 计入阻断与返修，标错也救不了流程——\
 把范围外的事实标成 actionable 只会让 owner 无路可走。\n\
 - finding MUST 落在本 Work Item 的写入范围 / 验收标准 / 能力契约之内。「仓库内其它文档的\
 当前状态」「产品侧规范或流程待办」「与本 Work Item 无关的外部事实」不是本 Work Item 的\
 缺口——若仍要提示，MUST 标 `disposition: \"advisory\"`，且 `required_action` 不得要求\
 作者执行（不得写「需在计划中安排…」「必须补…」这类祈使要求，只能写成「提示：…」）。\n\
 - 能指到具体 Work Item 时填写 `target_work_item_id`（如 `WI-001`）；指不到就留空。\n\
 - 若你报告的事实在上一轮已被 owner 处置或排除（见输入里的 `[human_dispositions]`），\
 MUST NOT 把它报为 `actionable`；如仍有必要提及，只能 `disposition: \"advisory\"` 且\
 `required_action` 不得要求作者执行。\n\
 - 重复报告同一事实时 MUST 在 `message` 里说明「上一轮的处置为何没有解决它」；\
 已处置且事实未变 → MUST NOT 报为 actionable。\n";

#[cfg(test)]
mod tests {
    use super::super::*;

    /// REQ-WSC-11（观察 2）：任一处 reviewer prompt 都必须携带人话裁决条款与
    /// 两个字段名——新增第 8 个 builder 若漏走 `reviewer_output_contract` 或漏
    /// 写 schema 字段，本测试即报红。
    #[test]
    fn every_reviewer_prompt_contract_carries_owner_guidance_rules() {
        let prompt =
            reviewer_output_contract("nonce", "{}", "intro", &RoutingReferenceContext::Legacy);
        assert!(prompt.contains(OWNER_GUIDANCE_RULES), "{prompt}");
        for field in ["owner_question", "owner_options", "consequence", "label"] {
            assert!(prompt.contains(field), "missing {field} in contract rules");
        }

        // 7 处 builder 的 schema 与契约注入点：源码级钉（新增 builder 必须同名进入）。
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sources = [
            "src/product/workspace_engine/prompts/review.rs",
            "src/product/workspace_engine/prompts/review_parts/draft_input.inc.rs",
            "src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs",
        ];
        let mut contract_calls = 0;
        let mut schema_fields = 0;
        for relative in sources {
            let text = std::fs::read_to_string(manifest.join(relative))
                .unwrap_or_else(|error| panic!("read {relative}: {error}"));
            contract_calls += text.matches("reviewer_output_contract(").count();
            schema_fields += text.matches("\"owner_question\"").count();
        }
        assert!(
            contract_calls >= 7,
            "reviewer prompt builders must inject the shared contract: {contract_calls}"
        );
        assert!(
            schema_fields >= 7,
            "every reviewer schema must show owner_question: {schema_fields}"
        );

        // REQ-WSC-12：同一批 7 处 schema 必须给出 disposition 与
        // target_work_item_id 字段形状，且共享契约携带范围/收敛硬约束。
        assert!(
            prompt.contains(FINDING_SCOPE_RULES),
            "reviewer contract must carry FINDING_SCOPE_RULES"
        );
        let mut disposition_fields = 0;
        let mut target_fields = 0;
        for relative in sources {
            let text = std::fs::read_to_string(manifest.join(relative))
                .unwrap_or_else(|error| panic!("read {relative}: {error}"));
            disposition_fields += text.matches("\"disposition\"").count();
            target_fields += text.matches("\"target_work_item_id\"").count();
        }
        assert!(
            disposition_fields >= 7,
            "every reviewer schema must show disposition: {disposition_fields}"
        );
        assert!(
            target_fields >= 7,
            "every reviewer schema must show target_work_item_id: {target_fields}"
        );
    }
}
