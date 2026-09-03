# workitem-conversational-gate-advance —— 阶段 3 验收证据（Task 8.5）

本目录是 change `workitem-conversational-gate-advance`（openspec 契约：[specs](../../../openspec/changes/workitem-conversational-gate-advance/specs)）的机器可读验收产物。全部数字聚合自真实命令输出与 durable 证据，构建器对缺行/重复 scenario/无 evidence/test_count=0/真实 provider 未声明授权/staged files 一律 fail-closed（见 [acceptance_report.test.mjs](acceptance_report.test.mjs) 与 [build_acceptance_report.mjs](build_acceptance_report.mjs)）。

## 环境 / 基线

- 被验收实现态 HEAD：`0e676fe0e43bc788a3da18cf4245d00ef016dd85`（`fix(campaign): resume amendment publication from existing journal after mid-window crash`，本 change 最后一个实现/生产修复 commit；验收产物 commit 只含本目录文件，不改变代码行为）。
- 执行环境：宿主机 worktree `feat-b-0808-add-monorepo`；`cargo --locked`、无显式 `-j`；Node `v24.17.0`（仓库现有 `node --test`）。
- 任务开始时 `git status --short` 为空；验收 commit 落盘后复核为空（`no-staged-files=true`，`git diff --cached --quiet` exit 0）。

## 命令与测试计数（2026-09-03 task 8.5 worker 亲跑；与 progress.md 中 controller 最新亲验基线一致）

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | exit 0 |
| `cargo check --locked` | exit 0 |
| `cargo test --locked --lib` | 3072 passed / 0 failed / 2 ignored（controller 基线为连续两轮同值） |
| `cargo test --locked --test it_core` | 149 passed / 0 failed |
| `cargo test --locked --test it_web` | 400 passed / 0 failed / 12 ignored |
| `node --test campaign_driver_policies.test.mjs stage3_campaign_driver.test.mjs` | 49 passed / 0 failed |
| 定向 `campaign_stage3_recovery_matrix` / `_amendment` / `_advance` / `_dependency` / `_group_projection` / `_interactive`（均先 `-- --list` 非零再跑） | 7 / 3 / 4 / 3 / 1 / 6 全绿 |
| `node --test acceptance_report.test.mjs`（本目录 fail-closed 套件） | 8 passed / 0 failed |

命令逐条记录（含 exit_code / test_count / log_ref）见 [commands.manifest.json](commands.manifest.json)；摘要日志见 [evidence/logs/](evidence/logs)。

## 14 REQ / 37 scenario 闭环

[acceptance-report.json](acceptance-report.json)：requirements 14/14 passed、scenarios 37/37 passed；[scenario-evidence.jsonl](scenario-evidence.jsonl) 每个 spec scenario 恰一行（`req_id`/`scenario`/`test_or_campaign`/`status`/`evidence_refs`/`durable_assertions`/`provider_authorization`），evidence 指向真实测试名（rust:/node: 由构建器 rg 解析）、脱敏 durable 证据（[evidence/](evidence)）与台账锚点（ledger:，人工反查 progress.md/任务报告）。六个 durable invariants（budget_exactly_once / provider_ledger_reconciled / event_prefix_immutable / advance_unique_attempt / single_active_unit / amendment_same_attempt）仅在锚定测试存在且被 scenario 证据引用时为 true。

## fake 与 real provider 分栏（如实）

- **fake/确定性证据（完整）**：37/37 scenario 全部由确定性单测/集成测试/fake campaign 锁定，`passed` 不是手填——由构建器从逐行 status 汇总，且引用测试名不可解析即构建失败。
- **真实 provider（授权已用，如实记缺）**：用户授权沿用阶段 2 常设授权（codex+pi，2026-09-02 拍板）。8 次真实跑全部 `executed_no_confirmed_run`、0 次到达人工门（human_gate_turns 全空）：codex×4（1 driver 配置错误 + 3 真实输出缺陷：source_id 缺失 / 重复契约号×2，后者为系统性）；pi×4（2 driver 真 bug 已修 [84e6a7c2]、1 硬超时 1800s 非死路、1 输出格式缺陷）。净产出：逼出并修复 2 个 driver 真 bug；如实记录 5 个真实 provider 输出质量缺陷，未伪造未绕过。**真实 Confirmed campaign 移交专项测量轮（档期 1：本 change 收官后独立任务，届时重新授权，小规模预试验先行）**——REQ-CG/WSC-interactive 的真实链完成证据缺位如实登记（见 [defer-ledger.md](defer-ledger.md) RR-1），不冒充已验证。

## 本 change 的四个生产修复（全部用户拍板 + 复审过）

1. SC 计划批准 Confirmed 终态保留人工门快照（`debb0884`+`dc133083`，用户拍板 2026-09-03）。
2. 修订候选回落至最近 Markdown artifact version（`a58070eb`，A 轮 ~10 行回落）。
3. SC 编译链落盘 draft_records，打通 confirm→advance 生产死路（`f57470cd..2072839d`）。
4. 修订出版中窗崩溃按既有 journal existing-replay 恢复（`0e676fe0`）。

另有两个真实跑逼出的 driver 真 bug 修复（`84e6a7c2`）与一个结构化 Rejected 保真修复（`07e54dbf`，7.2 Medium-1）。

## 已知语义边界（oracle 裁决 B，原文照抄）

> 已知语义边界：在当前 `validate_dependency_contract_graph` 的保守、fail-closed 约束下，现行合法 2-WI 真实 `prepare_amendment` 链路实测仅可产生 `AwaitHandoff`；`Reexecute`/`Revalidate` 的恢复机制及三模式映射已由 7.2 `group_amendment_approve_updates_binding_and_resumes_target` 锁定，因此不将两模式在本链路中的真实可达性列为本 change 的验收失败项。该边界不阻塞收官，由专项测量轮 owner 继续观察；若未来出现多级依赖链或外部修订场景，再单独评估 impact 判定与图校验关系。

引用锚：spec.md:49-66、design.md:52-54（D7）、design.md:104-107（D16）。**本报告不声称「三模式已在真实链全部验证」。**

## defer / risk

全部范围外 defer（UI、真实 provider 专项轮、SC 子 session 只读呈现形态、auto_start_coding 显式语义、多仓 coding、旧协议退役门等）与 residual risk（真实 Confirmed campaign 缺口、flaky 家族定性纪律、语义边界等）逐条带 owner/裁决引用：[defer-ledger.md](defer-ledger.md)（结构化源 [defer.manifest.json](defer.manifest.json)）。deferred 均标注「不在阶段 3 范围」，不是缺陷关闭。

## 复算

```bash
node cadence/reports/workitem-conversational-gate-advance/build_acceptance_report.mjs --check
node --test cadence/reports/workitem-conversational-gate-advance/acceptance_report.test.mjs
```
