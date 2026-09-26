# Design: tolerate-explained-empty-open-items

## Context

见 proposal.md - Why。现状：`src/product/workspace_engine/artifact_constraints.rs` 的 `unresolved_open_item_findings` 仅对 Story 生效；`open_item_section_is_resolved` 要求整节 compact 后精确等于空标记，或逐行通过 resolved cue 白名单。实测失败样本（timeline_node_002）中模型输出「无待确认项。Issue 已明确需求……不构成本 Story 的未决问题」被误拒。

## Goals / Non-Goals

- Goals：消除"空标记 + 解释"被误杀；保持真未解决项拦截能力；输出内容要求不变。
- Non-Goals：不改重试协议、reviewer must_fix 规则、headings/ID/token 其他约束；不改 Design/WorkItem/WorkItemPlan 的待确认项行为（本启发式本就仅作用于 Story）。

## Decisions

1. **保留既有上游推导白名单分支**：`open_item_remainder_claims_upstream_derivation` → 严格推导短语录得的既有行为及其全部负例单测（part_10.rs 38 例）全部保持不变；本 change 不触碰该分支。
2. **放宽空标记行的软 cue 拒绝**：`open_item_line_is_resolved` 中以空标记开头的行，仅当包含硬未解决 cue（`[open-`/`仍待确认`/`需确认`/`需要确认`/`尚待确认`/`tbd`/`todo`）或落入推导分支时才拒绝；软 cue（`待确认`/`未决`/`未明确` 等）不再单独拒绝已声明「无」的行。实测失败样本正是被软 cue（“未决问题”）误杀。
   - 备选 A（否决）：把「无待确认项」加入 resolved cue 白名单——逐行级粒度太细，解释行仍会被软 cue 命中，失败率下降不彻底。
   - 备选 B（否决）：整节首行前缀匹配短路——会放行「无。Codex 包名仍待确认」等既有负例，破坏既有契约。
3. **空标记边界约束**：仅当空标记后紧跟句读边界（`。.,，；;：:!？?（）`、空白或行尾）才视为空标记开头，防止「无论使用哪种 runner 都需要确认」被「无」前缀误判为已声明空标记。
4. **prompt 提示同步收紧**：Story `open_item_policy_hint` 追加「若无开放问题，正文只写『无待确认项』，不得附加解释；解释性内容写入其他章节」，从源头降低进入容错分支的概率。

## Risks / Trade-offs

- [风险] 首个非空行以「无」开头的真问题被放行（如「无论使用哪种 runner 都需要确认」）→ 缓解：仅当 compact 前缀**精确等于**某空标记或空标记+分隔符（。/./，等已在 compact 中剥除）时判已解决；「无论…」compact 为 `无论…`，不以 `无` 精确前缀分词边界命中——实现时用"compact 前缀等于空标记本身"判断，即 `compact == marker || compact.starts_with(marker)` 仅对 `无待确认项` 等完整短语与单字 `无`/`暂无` 生效，并对单字 `无` 要求后续紧跟句读或结尾（`无。…` compact 后为 `无…`，需以 `无` 后无字母数字续接为界）。单测覆盖「无论使用哪种…」负例。
- [风险] 放宽后掩盖模型把真问题写成"无"的漏报 → 缓解：这是内容质量问题，reviewer 阶段与人工确认仍兜底；校验器目标只是结构判定。

## Migration Plan

单仓内小改动，无数据迁移；回滚即 revert。

## Open Questions

（无）
