# wave4-passive-quotes：malformed 输出弯引号归一化修复报告

日期：2026-09-20 · 线：malformed-markdown 修复通道（被动/延后项）

## 根因

门重测楔死链路（kimi rep4 / workspace_session_0460，codex rep2 同族；现场 nonce=“86bc0a08”）：

CJK 输入法把 sentinel 的定界引号**全部**输出为全角弯引号（U+201C/U+201D）：

```
<ARIA_STRUCTURED_OUTPUT nonce=“86bc0a08”>
{“nonce”:“86bc0a08”,“verdict”:“pass”,...}
</ARIA_STRUCTURED_OUTPUT>
```

- 起始标签：`parse_nonce` 只剥离直引号 → None → `missing_start_tag`
- JSON 体：serde 拒绝弯引号定界 → `invalid_json`

现场实际记录：`verdict.structured_output_diagnostic.code = missing_start_tag`、
`review_gate = user_triage_required`（session_0460 timeline_node_003）——reviewer 本意
verdict=pass 的完整输出被纯字形问题楔死 needs_human。证据：
`.aria/projects/project_0001/issues/issue_0286/workspace-timelines/workspace_session_0460/timeline_node_details/timeline_node_003.json`。

## 修法（对照 capability fixer 770c1f71 同款保守边界）

`src/cross_cutting/structured_output.rs`，确定性机械修，serde 仍为 JSON 合法性最终权威：

1. **`parse_nonce`**：属性值接受直/弯两种引号字形（成对出现才剥离，混搭不改）。
   引号字形本就不是信任边界——nonce **值等式**仍是权威（测试锚定：
   弯引号属性 + 值不匹配 → 照旧 `nonce_mismatch` + observed_nonce）。
2. **`normalize_curly_json_delimiters`**（`recover_json_object` 入口统一应用，
   主解析与 recoverable_value 恢复路径同时受益）：
   - 字符串外 `“` → `"`（开定界）；
   - 弯引号串内 `”` 仅当后随（跳过空白）`,` `:` `}` `]` 或结尾才视为闭定界 → `"`，
     其余内容弯引号逐字保留；
   - **直引号串内的弯引号是数据，一律不动**（合法 JSON 零回归守卫）；
   - 转义感知（`\` 后一字符原样跳过）；无弯引号输入走 `Cow::Borrowed` 零开销快路径；
   - 归一后仍非法 → 照旧 `invalid_json`，fail-closed 不静默放行。

不碰语义：`readable_output` 用原文构造，人类可读通道的弯引号逐字保留。

## TDD 证据

- **红**（修复前，与现场同码同消息）：
  `normalizes_fullwidth_curly_quoted_sentinel_into_authoritative_parse` →
  `Failed(MissingStartTag "structured output start tag must contain a valid nonce attribute")`
  （另 2 新测试同红；1 零回归守卫保持绿）
- **绿**：4 新测试全过；**现场原产物**（session_0460 streaming_content 948B 原样输入，
  expected nonce=86bc0a08）→ `PARSED verdict=pass scope=outline findings=1`

## 回归

| 面 | 结果 |
|---|---|
| `cargo test --lib -- structured_output::` | 24/24 绿（20 旧 + 4 新） |
| `cargo test --lib -- contract_autorepair`（capability 面） | 11/11 绿，零回归 |
| `cargo test --lib -- review::structured_output`（消费方） | 5/5 绿 |
| `rustfmt --check` 本文件 | 通过 |
| clippy（scratch 镜像单文件 crate，含测试） | 0 warning |

注：树内全量 clippy 期间 kimi_code_provider/client_services 处于兄弟线（F-17）编辑中，
故本文件 clippy 以隔离 crate 镜像同源文件验证。

## 影响

reviewer/author 结构化输出解析共用该 cross-cutting 解析器，全部调用方
（含 `parse_structured_output_first_line_nonce`、`parse_last_structured_output`、
review fallback 恢复路径）自动受益；弯引号输出不再楔死 needs_human，
对应 run 可继续走正常 verdict 链。
