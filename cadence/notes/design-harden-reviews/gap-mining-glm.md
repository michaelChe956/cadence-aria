## M1 失败模式×清单覆盖对照表

典型弱模型（Kimi/Pi 级）走单仓 Design 全流程的正向推演：

| # | 步骤 | 具体失败形态 | 现状对策（file:line） | 清单覆盖？ |
|---|------|------------|---------------------|-----------|
| F1 | 读 Story→首生成 | heading 层级错（`###` 也过 gate）/占位 ID `[DEC-*]` 过 gate | validator 宽松（`artifact_constraints.rs:222-296`，D-07） | ✗（non-goal 4 有意留白→B） |
| F2 | 首生成 | fence 嵌套坏（artifact 内代码块撞外层三反引号） | 生成/retry/revision prompt 均含四反引号指令（`prompts.rs:439-467`） | ⑥/④部分；choice 后续路径裸奔（见 A-1） |
| F3 | 首生成后 text-fallback choice | 弱模型输出半成品+文字选项问题，用户作答后模型不把决策写入 `## 设计决策`（choice 不落章）、或不再输出完整 artifact fence | `take_pending_author_choice_prompt`（`lifecycle.rs:615-678`）只拼裸问题+答案文本，走 `AuthorPromptMode::DeltaOnly`（`provider_drive.rs:30-45`），**不注入** schema/fence 契约/skeleton/decision contract（`build_streaming_input` DeltaOnly 分支 `prompts.rs:214-216` 原样透传） | ✗ **裸奔**（→A-1） |
| F4 | review | 边界判定摆荡（抽象追踪误报 must_fix / 测试越界漏报） | 仅一句规则文本（`review.rs:42-93`） | ✓ ①三判例 |
| F5 | review | reviewer JSON 照抄示例载荷/nonce 缺失/字段漂移 | nonce+schema+repair（`review/structured_output.rs`） | ✓ ②③ |
| F6 | 用户反馈返修（AuthorConfirm） | prompt 丢 schema/fence/skeleton、当前产物三反引号被内嵌代码块击穿 | `author_revision.rs:7-27` 现状全缺 | ✓ ④⑤ |
| F7 | 反馈返修-无 resume | 上游 Story Spec（REQ/AC 来源）不在 prompt（workspace context 只在首条 system 消息，compact_history 暂缓） | 依赖 provider resume 会话残留 | ✗（D3 有意暂缓→B，建议 campaign 观测） |
| F8 | 生成 gate 失败 | Kimi/Pi 无 artifact retry（`provider_drive.rs:1001-1003`），一次结构失败即 run Failed | 有明确报错+blocking_reasons 引导（`session_state/timeline.rs:59-76`） | 部分（⑦ campaign 度量，未列入已知留白→B） |
| F9 | needs_human | 用户无引导 | verdict 映射后统一走 `route_review_report_to_author_confirm`（`review/routing.rs:86-100`），报告进对话流+提示语 | ✓ 现状够（C） |
| F10 | 断线恢复 | 返修反馈丢失 | T7 fix1 从节点 detail 重建 `pending_revision_context`（`interrupted_run_recovery.rs:124-160`）；choice 暂停态可恢复（`lifecycle.rs:334`） | ✓ 现状够（C） |

## M2 story 八类模式反查

| story 模式 | Design 侧对应物 | 清单覆盖？ |
|---|---|---|
| 协议 producer 遗漏迁移（新入口忘带 contract） | **choice followup DeltaOnly 路径无任何输出契约**（F3，同病） | ✗ →A-1 |
| legacy bypass 绕过协议 | author_revision_prompt 绕过 workspace context（已覆盖）；choice followup 绕过（未覆盖） | 半覆盖 |
| few-shot 照抄 | 判例带封装→照抄可复活；skeleton REQ/AC 提示语误导 | ✓ ①②⑤（判例去 sentinel、ID 指纹、DEC/CMP/API 文案） |
| JSON 容错越界 | `is_repairable` 示例载荷经 envelope repair 复活 | ✓ ②（四象限测试） |
| severity 双入口差异 | parser 三档 vs review_gate 判定 | ✓ ⑥/4.3 candidate→finding 断言覆盖 |
| 窗口 token 冗余 | 强 finding 双重重放（`history_compaction.rs:173-177`+`review.rs:64-68`） | ✗（non-goal 5，B） |
| 字符数≠usage | manifest 以字符数冒充 usage | ✓ ⑦/D6（usage 必填/`usage_unavailable`） |
| 真机≠单测 | grep 文案式断言≠真实行为 | ✓ ⑥⑦（真实 validator 路径+campaign baseline） |

## M3 清单外薄弱点扫描

1. **choice followup 路径（A-1 证据链）**：
   - `src/product/workspace_engine/lifecycle.rs:615-678`：`take_pending_author_choice_prompt` 输出「用户回答了 author 的确认问题…请基于该回答继续生成完整候选产物」，无 fence/schema/skeleton/decision contract。
   - `src/product/workspace_engine/provider_drive.rs:30-45`：`handle_author_choice_followup_message` 固定 `AuthorPromptMode::DeltaOnly`。
   - `src/product/workspace_engine/prompts.rs:214-216`：DeltaOnly 分支 `prompt = user_content.to_string()`，除 aggregate 外零注入；结构化决策落章指令（`prompts.rs:499-503` Design 版「写入 ## 设计决策…author-decision-*」）只在 FullConversation/reviewer 返修路径出现。
   - 后果：完成消息走 `complete_assistant_message`（`provider_drive.rs:695-770`）→ 同样过 artifact gate；弱模型以对话体作答→gate 失败→Kimi 无 retry→run Failed；即使过了 gate，决策大概率不落章，直接违背 D5 golden「用户决策不丢失」。清单 3.2 的「三入口」不含此第四入口。
2. **linked_story_context**：`src/web/workspace_context/entity.rs:38-49` Design 注入完整 Story Spec，但仅进首条 system 消息；author 反馈返修 prompt（改造后）不含上游 Story → 依赖 resume（见 F7）。清单未列入 campaign 观测维度。
3. **provider_allows_artifact_retry**：`provider_drive.rs:1001` 排除 Pi/KimiCode——目标弱模型恰好全被排除；清单 ⑦ 度量 retry 分类但未把「弱模型零 retry 的一次失败率」列为已知留白。
4. **needs_human 用户引导**（`review/routing.rs:91-100`）：报告全文进对话流+提示语，够用。
5. **interrupted_run_recovery**（`interrupted_run_recovery.rs:124-160` + `lifecycle.rs:334`）：反馈与 choice 暂停态均可恢复，链路完整。
6. **detect_author_choice_request 对 Design**（`parsers/choice.rs:3-22`）：Design 在白名单内，行为正确；问题不在检测而在 followup prompt（见 1）。

## M4 A/B/C 分级结论

**A 级（强烈建议并入）**

- **A-1：choice followup（DeltaOnly）Design 分支注入输出契约**。代码证据见 M3.1。这是清单「协议 producer 遗漏」病根在 Design 的最后一个存活入口，且直接击穿 D5 golden 的「用户决策不丢失/不反转」验收项——campaign 语料若含 choice 形态（6 形态含「choice→DEC」），该路径失败会使 gate 误判为 prompt 改造无效。**最小改法**：在 `take_pending_author_choice_prompt` 末尾（或 `handle_author_choice_followup_message` 的 Design 分支）追加 `append_author_artifact_output_contract` + skeleton + decision contract——与 5.1 同一注入件复用，Story 分支字节不变负例同 5.2；在 3.2 滑窗 fixture 中补第四入口断言。

**B 级（记入 non-goals / residual risks）**

- B-1：author 反馈返修在无 resume 会话（provider 切换/fallback 重建）时上游 Story Spec 缺失——建议在 6.2 campaign manifest 中加 `resume_available` 维度如实记录，作为 compact_history 决策输入。
- B-2：Kimi/Pi 无 artifact retry，一次结构失败即 run Failed——建议 proposal residual risks 一句话记录（依赖失败报告引导用户重发）。
- B-3：heading 级别/占位 ID validator 宽松（non-goal 4 已列，建议在 residual risks 中显式写「占位 `[DEC-*]` 可通过 gate，靠 reviewer 判例兜底」）。
- B-4：强 finding 双重重放 token 冗余（non-goal 5 已列，无新增）。

**C 级（无需处理）**

- C-1：needs_human 引导路径——报告进对话流+AuthorConfirm 提示，行为充分。
- C-2：interrupted_run_recovery——Revision 反馈重建与 choice 暂停恢复均完整（T7 fix1）。
- C-3：review 输入中 artifact 以裸 markdown 呈现——已有显式边界说明防误判返修。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "只读完成遗漏挖掘分析，未修改任何文件、无 git 写操作、未派生子代理；输出 M1-M4 四节对照结论，全部论断带 file:line 代码证据"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "rg/sed 定向阅读 openspec 契约 + src/product/workspace_engine（choice.rs/provider_drive.rs/prompts.rs/prompts/revision.rs/prompts/author_revision.rs/lifecycle.rs/interrupted_run_recovery.rs/review/routing.rs）+ src/web/workspace_context",
      "result": "passed",
      "summary": "确认 choice followup DeltaOnly 路径零契约注入（A-1）、artifact retry 排除 Kimi/Pi、linked_story_context 仅首条 system 消息、recovery/needs_human 路径完整"
    }
  ],
  "validationOutput": [
    "A-1 证据链：lifecycle.rs:615-678 裸 followup 文本 → provider_drive.rs:30-45 DeltaOnly → prompts.rs:214-216 原样透传，decision contract 仅存在于 FullConversation 路径",
    "B 级留白 4 项、C 级 3 项均已给出代码定位与理由"
  ],
  "residualRisks": [
    "A-1 未并入前，campaign 的 choice→DEC 形态样本预期 full-chain 失败"
  ],
  "noStagedFiles": true,
  "diffSummary": "无 diff（纯只读分析任务）",
  "reviewFindings": [
    "A-1: lifecycle.rs:615 / provider_drive.rs:30 / prompts.rs:214 — choice followup Design 路径无输出契约，choice 不落章，建议与 5.1 同法最小注入"
  ],
  "manualNotes": "A 级仅 1 项且证据完整；B 级建议直接落 proposal 的 residual risks 段，不动 tasks 结构。"
}
```