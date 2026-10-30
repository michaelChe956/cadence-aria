# 交付说明：审查 finding 范围约束与 owner 排除闭环

> 2026-10-31 · 分支 `feat-b-0808-add-monorepo` · 提交 `e18d02a2` + `e248aa16`（归档）· 已部署 4317

## owner 2026-10-30 真机反馈（本 change 的起因，原文）

> 1. 我的整个流程不应该存在这个情况【前端规范文档里还留着「中文 locale 尚未配置」这条缺口，这次代码补上中文语言包后，由谁来把这条记录改成已完成——是让做这次前端改动的人顺手一起改，还是交给流程那边统一回写？】，**我的文档都是 aria 产品的文档，不存在让 coder 实施的时候去帮我回写的概念**，这块你需要优化
> 2. 刚刚这样卡住了，**用户不知道如何往下继续了**
> 3. 这个【前端规范文档里还留着「中文 locale 尚未配置」这条缺口】这个问题，**用户其实也操作过两次了，但是每次 author 修完之后，reviewer 还是会审核出来这个问题**

**真机现场**：session `workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e`，reviewer **连报三轮**（第 3 轮每条开头都写「重复上一轮未收敛的问题：」）；owner 两次回复：12:58:21「这次不回写，文档先保持原样」→ completed；**13:04:01「计划里不写文档更新任务，本次只交付代码,交给流程那边统一回写」→ `failure_class = validation_reject`** → 自动化停在原地。

**根因（三条同一病根）**：① reviewer 的 finding 范围**无约束**，且 findings **无归属字段** → 机器无法判别「这条能否通过改 WI 收敛」；② owner 的反馈**只有一种语义**（修订指令），**没有「这不归本 WI」的表达位**；③ **下一轮 reviewer 拿不到上一轮处置**（输入只有 `plan_candidate_ir_ref` + `mechanical_report_ref`）。

## 改造清单

| # | 改造 | 落点 |
|---|---|---|
1 | findings 增 `disposition`（`actionable`\|`advisory`，**缺失＝actionable** fail-safe）+ `target_work_item_id` + `finding_key`（**平台算**：`sha256(severity+\x1+message)` 前 16 位 hex，模型不得生成）| `web/workspace_ws_types/review.rs` |
2 | **7 处**审查 prompt 经 `FINDING_SCOPE_RULES` 统一加「范围约束 + 收敛约束」（范围外事实只能作 advisory；已处置事实不得重复报为 actionable）| `prompts/review.rs` + `review_parts/*` |
3 | **平台硬规则**：只有 actionable 计入阻断/返修/预算/必关闭清单/门卡计数（**判定落在平台侧，不依赖模型标对**）；`advisory` + `category ∈ {contract_gap, self_contradiction}` → 解析期升为 actionable（**堵住漏检后门**）| `parsers.rs`、`review/feedback.rs`、`work_item_plan_policy/classify.rs`、`prompts/history_compaction.rs` |
4 | **作者返修输入面**只列 actionable（`feedback.rs` 新增 actionable 变体 + 三处注入点接入 + `sc_revision.rs` 的 `[review_findings]` 只迭代 actionable）；advisory 单列 `[review_advisory_findings]`（**只给条数、不含正文**）+「仅供提示，本轮不要修改」| `review/feedback.rs`、`review/routing.rs`、`plan_outline/revision.rs`、`work_item_split_engine/prompts/sc_revision.rs` |
5 | **owner 排除项落 durable**（`workspace-sessions/{session_id}/human-dispositions.json`，按 key 幂等零写、按 session 隔离）；**排除-only 帧先落盘再受理**（不建修订 turn、不判反馈为空、不发 `INVALID_HUMAN_GATE_FEEDBACK`）；WS 回 `human_gate_exclusions_accepted`、REST 回 200 `accepted`；**落盘失败回 422 不冒充受理** | `web/workspace_ws_handler/decisions/inbound.rs`、`handlers/workspace_human_action.rs`、`lifecycle_store/human_dispositions.rs` |
6 | **下一轮评审回喂** `[human_dispositions]`；**平台侧按清单强制降级**（`disposition_override.rs`：`finding_key ∈ 清单` → 强制 advisory，接线 4 处：complete_review / 恢复派生 / 指纹闸门两侧 / classify 前置）→ **不依赖模型听话** | `review/disposition_override.rs`、`prompts/review_context.rs` |
7 | **可撤销**：`revoke_human_dispositions`（按 key 删、幂等零写）+ REST `revoke_finding_exclusions` + 前端「撤销排除」按钮（成功后本地集合**整体对齐**回执 `remaining_finding_keys`）| `lifecycle_store/human_dispositions.rs`、`web/src/state/workspace-finding-exclusions.ts` |
8 | **门卡可操作化**：被拒时给「被拒反馈 / 可读原因 / 下一步 / 编译诊断」（**诊断落盘可查**）；顶部「**现在请做 X**」8 态指引；机器词（「预判」）片段级人话化；确认按钮禁用态给原因、可用态给后果；**排除失败撤销「已排除」并给可读原因**（WS 与 REST 两通道同源归因，15s 窗口 + kind 判据防误归因）；**durable 拒收重建加「门内在途 turn 不覆盖」** | `web/src/components/chat-workspace/entries/**`、`web/src/state/workspace-ws-store-human-gate.ts`、`web/src/hooks/workspace-ws-message-handler.ts` |
9 | **字节兼容**：`ReviewFinding` 的 **9 个 Option 字段全部保留** `skip_serializing_if`；durable 读历史记录**零改写**（两侧对称剔除 `finding_key`，不会造成假相等）| `web/workspace_ws_types/review.rs`、`session_state_parts/historical_review.inc.rs` |

## 契约

`openspec/changes/archive/2026-10-31-bound-review-findings-to-work-item-scope/` → 主 spec `work-item-plan-single-candidate` **+6 条需求**：

- `REQ-WSC-12` 审查 finding 必须落在本 Work Item 范围内，范围外事实降级为 advisory
- `REQ-WSC-13` owner 排除项落 durable 并回喂下一轮评审
- `REQ-WSC-14` 重复报告必须说明未收敛原因
- `REQ-WSC-15` 人工门在拒绝与 advisory 场景下必须可操作
- `REQ-WSC-16` owner 可以撤销已提交的排除
- `REQ-WSC-17` 返修触发路径只认 actionable

## 独立审查（3 轮 k3，每轮都抓到真东西）

| 轮 | 结论 |
|---|---|
第 1 轮 | **不可归档**：P0-2（排除-only 帧必被引擎拒 + 前端却显示已排除）、P0-3（durable 被回写 finding_key）、P0-1（advisory 漏检后门）、P0-2B（已处置事实只在 prompt 生效）|
第 2 轮 | **仍不可归档**：P0-2A/P0-3 已修 ✅，但**裁决面漏接「作者返修输入面」** → owner 第 3 点在输入面残留；另 P2（前端排除失败界面说谎）、P3×7（文档与实现不一致）|
第 3 轮 | P0-2A / P0-3 / 裁决面 **已修**；剩余 P1（作者返修 prompt 仍把已排除 finding 当必改项）+ P2（REST 通道同问题）+ P3（拒收重建在途 turn 判据）→ 已全部修完 |

## 验证（controller 实测）

```
cargo test --locked --lib          → 5743 passed / 0 failed（另见下方「已知偶发」）
cargo test --locked --test it_core → 191 passed / 0 failed
cargo test --locked --test it_web  → 591 passed / 0 failed
cargo clippy --all-targets --all-features --locked -- -D warnings → 零告警
cargo fmt --all -- --check         → 干净
cargo test --locked --test it_core large_file_guard → 2 passed（超限 0）
pnpm -C web test                   → 251 文件 / 2562 用例
pnpm -C web build                  → ✓
openspec validate … --strict       → is valid（exit 0）
```

**已知偶发**：`cargo test --locked --lib` 偶发 1 个失败（约 1/6 轮），**名字尚未捕获** → 见 `cadence/notes/2026-10-31_待办_lib全量偶发失败未定位_v1.0.md`（含定位方法与基线对照结论：**暂无证据**与本批相关）。

## 部署

| 项 | 值 |
|---|---|
二进制 | `/tmp/deploy-target/release/aria`（本次构建）|
回滚件 | `/tmp/aria-rollback-<sha16>`（构建前已备份）|
启动 | `bash /tmp/aria-deploy/start.sh`（**唯一权威入口**，含 `ARIA_API_TRUSTED_ORIGINS` 与 TTY）|
自检 | 端口 / `GET /` 200 / `GET /api/health` 200 / `governance_complete` / origin 白名单在位 / TTY / **新代码指纹（新件在、回滚件不在）** —— 7 项全过 |

## 遗留（已登记，不阻塞）

- `finding_key` 是内容指纹 → 模型改写措辞则 key 变 → 排除不再命中（**「按内容排除」的固有边界**，换算法属新契约）
- `revise_batch` / `plan_reopen_required` **不参与**归一（专用 legacy 机制，既有 3 条契约测试依赖）
- observer 镜像未加拒收重建（主 store 已覆盖刷新路径）
- 上面那条「已知偶发」
