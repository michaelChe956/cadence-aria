# Wave4-Passive F-24：choice 卡主动触发验证报告（真实 pi + 真实浏览器第二连接）

- 日期：2026-09-20
- 执行：F24Trigger（wave4 被动/延后项主动解决线）
- 验证对象：F-24 修复（`a1a479a1`，choice 卡可靠重投面）**实况复验**——修复上线后首次真实 provider 全链验证
- 结论速览：**通过。pi 真实 ask_user → choice 卡经 attach 重投面送达浏览器 cockpit 页（0484 形态：卡 pending 后才 attach）→ 用户浏览器应答 → pi 续跑 → 评审 → Confirmed（计划内容按用户裁定落实），全链无断点。**

## §0 环境

| 项 | 值 | 证据 |
|---|---|---|
| 服务器 | v30，PID 3305925（全程未重启） | `ps -p 3305925` |
| build | `git_sha=82ad301784b2`（HEAD，含 F-24 修复 `a1a479a1`；`git merge-base --is-ancestor a1a479a1 82ad3017` 成立） | `GET /api/runtime-info` |
| 二进制实证 | 内嵌 F-23 恢复日志串 `stale zombie run recovered to prepare_context`（同提交产物）；built_at 1789897033（17:42:13） | `strings target/release/aria` |
| 健康检查 | `{"status":"ok"}` | `GET /api/health` |
| 场景 | single_candidate work-item-plan，author=reviewer=pi，run_policy=auto_if_valid，minimal fixture 套 + **自定义描述注入强制 ask_user 裁定项**（「问候文案选哪个：hello / 你好 / こんにちは，未获答复不得编写计划内容」） | 驱动器 `f24-choice/f24-choice-drive.mjs` |
| 会话 | issue_0295 / workspace_session_0486 | `run2/ws.jsonl` |

## §1 验证链五环逐项判定

| # | 环节 | 结果 | 证据 |
|---|---|---|---|
| 1 | **pi 发 ChoiceRequest** | ✅ 作者 run 启动 8.6s 后（10:13:04.285Z）pi 调 `ask_user`（aria-ask 扩展）→ 服务端广播 `choice_request`（id=`893aac75…`，source=`provider_choice`，3 选项） | run2/ws.jsonl；marker `/tmp/f24-choice-pending.json` |
| 2 | **v30 重投面送达 cockpit 页** | ✅ 驱动器（连接 A）**显式拒绝应答**；浏览器（连接 B）在卡已 pending 后 attach，DOM 呈现 `data-testid=choice-request-entry`（问题「问候文案选哪个：hello / 你好 / こんにちは」，source 徽标 provider choice，提交按钮 disabled→选后 enabled） | 截图 `evidence/01-choice-card-pending.png`；DOM 探针（labels=[hello,你好,こんにちは,补充内容]） |
| 3 | **choice 卡可应答（用户操作）** | ✅ 勾选 `hello` → 「提交选择」→ 卡转「已选择：hello」 | 截图 `evidence/02-choice-answered.png`；DOM 探针 `resolved=true` |
| 4 | **provider 继续** | ✅ 应答后 ≤3s（10:14:07.585Z）pi 流恢复；后续 1064×stream_chunk 写全计划 + artifact_update + cross_review（reviewer pi）+ compile | run2/ws.jsonl（`provider_stream_resumed_after_choice`） |
| 5 | **裁定落到产物** | ✅ Confirmed 终态（node_005「WorkItemPlan 已确认」，phase=completed）；计划 source 全文按用户裁定 `hello` 落实：WI-001 标题「（问候文案 hello）」、capability `GET /api/greet 返回 200 且 body 恰为 {"message":"hello"}` | durable timeline_nodes.json + `work-item-plan-sources/…/source-3700018515f5affa.json`；截图 `evidence/03-final-confirmed.png` |

## §2 为什么第 2 环节证的是 F-24 修复面（而非既有广播）

广播空窗证明：ws.jsonl 中 `10:13:04.285(choice_request 广播) → 10:14:07.585(execution_event)` 之间**连接 A 收到 0 帧**（广播扇出对所有连接可见 ⇒ 该窗口无任何重广播帧）；浏览器恰在该窗口 attach 且随即呈现卡。卡只能来自 **attach 初帧对挂起 choice 帧的补发**——正是 `a1a479a1` 新增的 `ActiveRun.pending_choices` 重投面（0484 现场「引擎 pending、用户全程看不到」的直接反面）。

（degraded 恢复补发面与 cursor 重订阅面属同修复另两投面，本场景未构造对应故障形态，由单测 `degraded_attachment_recovery_redelivers_pending_provider_choice` / `attach_and_cursor_resubscribe_redeliver_pending_provider_choices` 覆盖，不在本实况复验范围。）

## §3 过程事实（如实登记）

1. **run1（issue_0293/session_0485）失败**：驱动器 v1 只在 `stage_change` 帧上触发 `start_generation`，而初始 `session_state` 即携带 `stage=prepare_context` → 未启动 → 90s 空闲被服务端断开。修复驱动器（stage 信号不分消息来源，与 campaign 驱动器同语义）后 run2 一次通过。run1 产物保留（`run1/`）。
2. **run2 驱动器进程被外层 bash 300s 后台超时杀死**（cross_review 进行中）——只影响驱动器侧收尾（result.json 缺席），**服务端 run 不受影响**（引擎侧持久推进至 Confirmed），终态以 durable 落盘为准（`run2/run2-analysis.json` 为事后分析，原始依据 ws.jsonl+durable）。
3. choice 悬置 900s 看门狗（F-22）未触发：卡送达+应答在广播后 ~63s 内完成。

## §4 产物索引

- 驱动器：`cadence/reports/provider-validation-round/f24-choice/f24-choice-drive.mjs`
- 原始帧流：`…/f24-choice/run2/ws.jsonl`（含 choice_request 全帧/应答后事件分布）；run1（失败首跑留档）
- 截图：`…/f24-choice/evidence/01-choice-card-pending.png`（卡渲染·第二连接）、`02-choice-answered.png`（已选择：hello）、`03-final-confirmed.png`（Confirmed 终态）
- durable：`.aria/projects/project_0001/issues/issue_0295/`（timeline_nodes / work-item-plan-sources / workspace-sessions 0486+0487）

## §5 结论

F-24 修复在真实 pi 全链上成立：**choice 卡不再依赖「恰好在场」的广播**——挂起帧随 run 登记并向后 attach 的用户连接补发，用户应答后 provider 无缝续跑。0484 形态（引擎 pending、用户全程看不到）在本证据下不可复现。遗留登记：无新增缺陷；03485（issue_0293/session_0485）run1 会话停留 prepare_context（无活跃 run，非僵尸，无需恢复动作）。
