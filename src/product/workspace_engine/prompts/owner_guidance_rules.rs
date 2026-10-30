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
    }
}
