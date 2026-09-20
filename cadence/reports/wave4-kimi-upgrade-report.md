# Wave 4：kimi 升级重跑报告（F-17 解锁后首次真实验证）

- 日期：2026-09-20（执行窗口 20:10–21:25+ CST）
- 服务端：v31（PID 3411905，git_sha `85af89644f12`，controller 部署并三对账）
- 执行者：KimiFinal
- 目标：kimi coding attempt 走完三角色全链（usage_by_role 三键 + code_review_complete + internal_pr_review_complete）→ ledger 受限登记升全量转正
- 终态（**如实**）：attempt `coding_attempt_9298fe0bd8644ceabb2625d4539a1d9f`（issue_0300）**waiting_for_human@final_confirm**——3 units 全 completed、4 轮 code_review（最后一轮 approve）、review_request_0001 已建（branch `aria/issues/issue_0300`，commit 2597ad9）、group readiness complete/diagnostics 空；final_confirm 被 **F-25（顿号 scope 方言缺口，新产品缺陷登记）** fail-closed 拒绝，attempt 留存（未 abort，现场保全）。usage_by_role 两键（author+reviewer），internal_reviewer 键未产生——见判据结构性发现。

## 一、本轮核心实证

### F-17 修复生产验证：✅ 通过（本轮关键验证点）

- coder（kimi）在 v31 bwrap 沙箱内**一次成功** `git add server/ data/levels.json tests/backend/` + `git commit`（13:03 CST，worktree `/home/michaelche/workspace/github/naruto/.worktrees/aria-issues/issue_0300`，HEAD=`410f4f5`，宿主侧 git log/status 复核干净）。
- 对照上轮（v27）：同场景两掷两中 `index.lock: Read-only file system` → blocked 门人工分诊。F-17（85af8964）后通路一次打通，kimi coder 承包契约（TDD 写路径 + commit 责任）在沙箱内可履行。
- 证据：`kimi-coding/coding-kimi_code-coding_attempt_9298fe0b…/ws.jsonl`（13:03:06/13:03:21 两条 git 命令 execution event）+ 宿主 worktree git log。

### 弯引号归一化：✅ 实证（rep10 计划腿直通 Confirmed）

- v31 下 kimi reviewer verdict=pass 被正常解析（rep10：session_0490 Confirmed，无 needs_human 降级、无 takeover 介入）——上轮 rep4 同输入死于 U+201C/U+201D nonce 属性 `missing_start_tag`。
- reviewer usage：input 14327 / output 5939 / cache_read 25408（真实消耗）。

### 三角色链推进（coding 腿，attempt 9298fe0b）

| WI | coder | code_review | 结果 |
|---|---|---|---|
| WI-001 | TDD 实跑（Write×N+沙箱 git commit 410f4f5） | round1 request_changes（CT-002 契约违规）→ 返修 → round2 **approve** | 过 |
| WI-002 | 实跑 | round3 **approve** | 过 |
| WI-003 | 进行中 | — | — |

- usage_by_role 已采两键：author（645/1030 最新轮）、reviewer（2981/865）；internal_reviewer 待 internal_pr_review 阶段触发。
- 真实返修循环在案：request_changes → coder 修复 → approve（非一路绿灯）。

## 二、环境故障与处置（如实登记，controller 批准在案）

1. **kimi CLI 模型端点故障**（20:14–20:20 CST 诊断）：campaign 三连死 `provider_empty_output`（Kimi ACP exit code 0）。根因链：`~/.kimi-code/config.toml` 20:10 起 `default_model=bingqi/glm-5.3-flash`，该模型在 103.236.76.196:9527 稳定返回 `server_error`（直测×3）；tydic-openai key 已 `INVALID_API_KEY`（上午成功轮用的是 tydic/glm-5.3-flash）；kimi 官方 oauth 文件为空。手动 ACP probe 复现：`end_turn` 返回但零 assistant_message。
2. **处置**（Main 批准）：config.toml 切 `default_model="bingqi/glm-5.3"` + 新增 `[models."bingqi/glm-5.3"]` 定义（glm-5.3 主模型直测 3×OK，延迟 ~5.5s）。切换后 ACP probe 恢复流式输出（105 msgs，usage 21872）。属 kimi CLI 基础设施配置，非 aria 代码。

## 三、路径教训（复证在案）

1. **POST 直建回落 codex 复证**（受限面 2 重演）：对 Confirmed plan 直接 POST `/work-item-plans/{plan}/coding-attempts` → attempt 62ae9aa admission_kind=legacy_group、provider=codex/codex，且 codex 侧 glm-5.3-flash 已 404 → blocked 门。abort 后 group initialization journal 仍绑定 legacy_group，typed advance 被拒（`existing group initialization is bound to another advance identity`，advance.rs:481；HTTP 无 delete_attempt 面）。**正确次序：Confirmed 后先 typed advance（冻结 kimi），再 campaign 幂等收养**——issue_0299 因先 POST 报废（62ae9aa aborted 留档）。
2. **repeated_fingerprint 分诊面正常工作**：rep11 计划腿 reviewer 两轮 suggestion 级 findings 指纹重复 → stopped_needs_human（trigger=repeated_fingerprint，resumable）→ takeover（0494）→ takeover-drive.cjs bare confirm → Confirmed → typed advance → attempt 9298fe0b（admission_kind=sc_advance，provider=kimi_code 三角色）全链 102s 打通。
3. rep10（issue_0299）直通 Confirmed 但被教训 1 报废；rep11（issue_0300）经 takeover 面建链成功——两条路径均产品内操作，零 durable 文件手工干预。

## 四、台账处置

- provider-status-ledger.md：受限面 1 已终态化（9298fe0b final_confirm 留存 + 判据结构性发现）；**新增 F-25 登记段**（顿号 scope 方言缺口）；结论维持**受限登记**（三键判据未齐且 group scope 下结构性不齐）。
- **判据结构性发现（controller 裁定面）**：runner.rs:383-410——provider-backed InternalPrReview 流程仅保留给 single-work-item attempt（旧 group shard/reduction 管线已退役，group 从 readiness 直进人工 final_confirm）；「internal_pr_review_complete + usage 三键」在 group scope 下结构性不可达（544a 冻结等待同因）。转正路径二选一：①走 single-WI attempt 采 internal_reviewer 键；②按 group 语义改判据为 completed+readiness 等价终态。

## 五、交付物索引

- coding 腿证据：`kimi-coding/coding-kimi_code-coding_attempt_9298fe0bd8644ceabb2625d4539a1d9f/`（ws.jsonl 全程 + result.json/coding-result.json 终态）；F-25 durable 现场：`issue_0300/work-item-revisions/…/work-item-projection-bundles/*.json`
- 计划腿：`kimi-coding/kimi_code/rep10/`（弯引号修复后直通 Confirmed+handoff，issue_0299）、`rep11/`（repeated_fingerprint 现场，issue_0300）、`rep11-takeover/`（takeover+advance-result.json+handoff.json）
- 报废现场：`kimi-coding/coding-kimi_code-coding_attempt_62ae9aa…/`（POST 直建回落 codex，已 abort）
- 环境诊断：本轮报告 §二；config.toml 变更（Main 批准）

## 六、移交（campaign 已终态）

- attempt 9298fe0b 留存 waiting_for_human@final_confirm（未 abort）：F-25 现场保全，controller 可复核或裁定 abort；campaign 已退出（failureClass=protocol_error, coding_final_confirm_failed）。
- F-25 修复后（顿号归一化），对同 attempt 重发 final_confirm 即可走完 completed（readiness 已 complete/diagnostics 空；F-25 是唯一拦截项——前提是该修复不改 SCO 校验语义）。
- 服务器 v31（PID 3411905）勿重启；kimi CLI config 已切 bingqi/glm-5.3（Main 批准）。
