# 复现说明：WIG 产品全流程 effort 对照（串行交替，n=2/档）

本目录保存报告 `../2026-11-09_分析报告_WIG产品流程实测与根因锁定_v1.0.md` 的全部原始证据。

## 目录内容

| 文件 | 说明 |
|---|---|
| `harvest_serial.py` | 收割器。从持久化产物还原每个 run 的节点耗时/工具调用/usage/预检码/平台 IR 判定/最终 source 的 cross-WI 合规/会话终态，并做置换检验（20,000 次，seed=23）。**只读产物，不改任何状态**。 |
| `harvest_output.txt` | 本报告全部数字的原始输出（4 个 run 的逐节点明细 + 对照表）。 |
| `sequence.log` | 串行执行序列日志（每臂起止时间、plan 终态）。 |

## 前置

- 二进制：`aria`（release）。工作区根的 `.aria/` 为产品状态目录。
- 真实案例仓库：`/home/michaelche/workspace/test-folder/test-center-java`（聚合根视图，含 busi/frontend/gateway 三个独立成员仓，各自有 main 基线）。
- 被测 issue：`project_0001 / issue_0001`（公告删除）。

## 装置

每臂一个**独立工作区副本** + **独立端口**，严格串行（同一时刻只跑一个），消除 proxy 资源竞争与顺序效应：

```
xhigh → high → high → xhigh        # 端口 4321 / 4322 / 4323 / 4324
```

- `xhigh` 臂：provider effort = `inherit`（沿用 xhigh，即默认档）
- `high` 臂：provider effort = `high`

唯一变量 = effort 档位；案例、仓库、agent 配置、provider 均相同。

## 单臂流程

1. 复制工作区 → 启动 `aria web --port <port>` → 等 `/api/health`
2. `POST /api/projects/project_0001/issues/issue_0001/work-item-plans:prepare`
   —— 新建一个空的人工 plan 并删除旧的自动 plan
3. **等待 autopilot 把「绑定 plan 已删 ⇒ enrollment terminated」落盘**
   —— **必须等**：`automation-enrollment.json` 的 `terminated_at` 非空后再继续
4. `PUT /api/projects/project_0001/issues/issue_0001/automation-enrollment`
   —— body 为 `EnrollmentWriteCommand::Enable`，**必须带 `command_id`**
   —— 仅 terminated 的取代路径消费 `command_id`；缺失 → 422；非 terminated 的常规重开不消费 ⇒ **不轮换 `prepare_intent_id`** ⇒ autopilot 随即又终止，等于没跑
5. 等待 plan 终态 `confirmed` / `failed`（上限 60 分钟），然后停 daemon

### 第 3 步是本装置最容易踩的坑

第 2 步与第 4 步之间若间隔不足 1~2 秒，autopilot 的终止尚未 durable，Enable 会被判为「非 terminated 的常规重开」：
`command_id` 不被消费、`prepare_intent_id` 不轮换、新 plan 不建立、enrollment 随即再次被终止。
表现为：prepare/enable 都返回 200，但 plan 目录始终为空。本装置用轮询 `terminated_at` + 额外 sleep 2s 规避。

## 采集

全部数据来自 durable 产物，无需埋点：

| 指标 | 来源 |
|---|---|
| 节点耗时 / 工具调用 / usage / verdict | `.aria/projects/project_0001/issues/issue_0001/workspace-timelines/<session>/timeline_node_details/*.json` |
| 平台 compiler 的 IR 判定（pass/fail） | 同目录 `../../candidate-attempts.jsonl`（按 `session_id` 过滤） |
| 最终 source + 机械报告 + 候选 IR | `.aria/.../issue_0001/work-item-plan-sources/<plan_id>/{source_revision,mechanical_report,plan_candidate_ir}/` |
| 会话终态（`confirmed` / `stopped_needs_human`） | `.aria/.../issue_0001/workspace-sessions/<session>.json` |

## 收割

```bash
python3 harvest_serial.py
```

- 收割器默认从 `/tmp/aria-wig/run-*` 读取；改路径请编辑脚本顶部 `BASE`。
- `ir_predicate` 复刻 `src/product/work_item_contract/validation.rs:245-312` 的 plan 域成员校验谓词（跨 WorkItem 的 `done_when_refs` 是否属于本 WI 自己的 criterion 集合），用于量化最终 plan 的 cross-WI 越界。
- 置换检验对两臂数值做 20,000 次随机重排，取「观测差至少与实测一样极端」的比例。

## 已知未覆盖

- run 3（high-4323）在 `stopped_needs_human` 后**未继续**：该 session 被 enrollment 精确绑定，`POST /api/workspace-sessions/{id}/takeover` 被服务端拒绝（`workspace_session_takeover_not_allowed`）；`POST .../automation-enrollment/continue-plan` 需要完整 `expected_binding` 与 `expected_stop_key`，本次未构造 ⇒ 「人工批准后能否完成」未测。
- n=2/档 ⇒ 性能维度 p=0.339，不可据此判定档位优劣。
