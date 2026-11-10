//! SC markdown author 的**判例 few-shot 与依赖语法教学面**。
//!
//! 从 `prompts.rs` 拆出，独立承载：`[real_finding_few_shot]` 判例渲染
//! （真实 provider 原始 finding，勿照抄业务名/ID/路径/命令）与 `dependencies`
//! 键的语法教学。判例 fixture 路径与渲染逻辑逐字节不变。

use crate::product::work_item_plan_compiler::grammar;

pub(crate) const WORK_ITEM_PLAN_FEW_SHOT_FINDINGS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_policy/fixtures/golden_findings.json"
));

pub(crate) const WORK_ITEM_PLAN_FEW_SHOT_IDS: [&str; 11] = [
    "rep1-f1", "rep1-f2", "rep2-f1", "rep2-f2", "rep2-f3", "rep2-f4", "rep2-f5", "rep2-f6",
    "rep3-f1", "rep4-f1", "rep4-f2",
];

pub(crate) fn work_item_plan_dependency_syntax_rules() -> String {
    let dependencies_key = grammar::DEPENDENCIES_KEY;
    let item_id_prefix = grammar::ITEM_ID_PREFIX;
    let item_id_suffix = grammar::ITEM_ID_SUFFIX;
    format!(
        "`{dependencies_key}`：`- {dependencies_key}: []`；`- {dependencies_key}: {item_id_prefix}001`（裸值）；多依赖每行：`- {dependencies_key}: {item_id_prefix}001`\n`- {dependencies_key}: {item_id_prefix}002`。禁止括号列表、空格或逗号分隔多值；值仅 `[]` 或 `{item_id_prefix}{item_id_suffix}`。"
    )
}

pub(crate) fn work_item_plan_real_few_shot() -> Result<String, String> {
    let findings: Vec<serde_json::Value> =
        serde_json::from_str(WORK_ITEM_PLAN_FEW_SHOT_FINDINGS)
            .map_err(|error| format!("无法读取 Work Item Plan 判例 fixture：{error}"))?;
    let mut cases = String::from(
        "[real_finding_few_shot]\n以下为真实 provider 原始 finding；rep1 是 Advisory。按错误模式→修正原则学习，勿照抄业务名、ID、路径或命令。\n",
    );

    for id in WORK_ITEM_PLAN_FEW_SHOT_IDS {
        let entry = findings
            .iter()
            .find(|entry| entry.get("id").and_then(serde_json::Value::as_str) == Some(id))
            .ok_or_else(|| format!("判例 fixture 缺少 {id}"))?;
        if entry.get("source_kind").and_then(serde_json::Value::as_str) != Some("provider_raw") {
            return Err(format!("判例 {id} 不是 provider 原始 finding"));
        }
        let finding = entry
            .get("finding")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("判例 {id} 缺少 finding"))?;
        let field = |name: &str| {
            finding
                .get(name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("判例 {id} 缺少 {name}"))
        };
        cases.push_str(&format!(
            "\n#### {id}\n错误模式：{}\n证据：{}\n修正原则：{}\n",
            field("message")?,
            field("evidence")?,
            field("required_action")?,
        ));
    }

    cases.push_str(
        "\n标题正反例：❌ `# 工作项计划` / `### 身份` → ✅ `# Work Item Plan` / `### Identity`（内容中文，标题英文逐字）。\n",
    );

    Ok(cases)
}
