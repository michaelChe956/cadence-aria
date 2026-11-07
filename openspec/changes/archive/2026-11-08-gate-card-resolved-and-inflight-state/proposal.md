# 提案：人工门卡的「已收口 / 修订在途 / 被中断」三态不得再呈现为全新待办

> 状态：owner 已批准 D1 + D2 + D3 三项（方案 C「折叠结论卡人话块」明确不做）。
> 根因与时序证据由 controller 磁盘 + 服务端日志实测给出，本 change 不重新调查。

## 背景（Why）

owner 真机反馈：reviewer 完成后出现「审核结论待人工分诊」卡；owner 按提示操作后 author 继续编写；**author 歇一会儿后又出现「需要判断 reviewer 意图」卡 → owner 误以为还要再操作，其实那一轮已经被终止**。

实测时序：

| 时刻 | 事件 |
|---|---|
| 06:09:32 | owner 提交人工修订 |
| 06:11:00.4 | 前端终止面（owner 有意点的）→ abort `run_token=9`，`node_id=timeline_node_008` |
| 06:11:00.6 | `node_009` + turn 落 `failed / provider_err` —— **这是终止的副作用，不是 provider 故障** |
| 06:11:30.5 | 会话 `status=terminated`（abort 后等 30s 收口超时） |
| 14:15 / 14:20 | owner 截图：门卡仍显示「需要判断 reviewer 意图」+「已终止」徽章 + 完整可点选项 |

三个缺陷叠加出「已终止但看起来还要你点」：

| 记号 | 层 | 缺陷 |
|---|---|---|
| **D1** | 前端 | `isResolved` 只关掉了部分元素：命令式标题、完整选项块与「选一个，把选项名原文作为补充指令发回去即可」、「为什么找你…」meta 行全都没关；「已终止」徽章只看 `resolution`，不说明来源与后果 |
| **D2** | 前端 | 门卡靠 store 的 `humanGateTurn` 判断「正在按反馈修订」；刷新 / WS 重连后 store 把它置 null（`workspace-ws-store.ts` 的 `setSessionState`），门卡退化成「全新可操作的分诊卡」，修订进度提示消失；此时再提交会被后端判 Busy |
| **D3** | 后端 | 因取消 / 终止导致的修订失败被记成 `Failed` + 「人工修订失败，本轮反馈未采纳」，对话流里**没有任何一句**告诉 owner「你的选择没生效」 |

## 目标

| # | 目标 |
|---|---|
| 1 | **D1**：门已收口（`resolved`）时，门卡不得再以行动号召形态呈现——标题改陈述式、不渲染人话选项块与作答指引、不渲染「为什么找你…」meta 行；「已终止」徽章保留且文案逐字不变，徽章旁补一句来源与后果说明 |
| 2 | **D2**：门卡必须能表达「正在按反馈修订」与「正在复评」，且刷新 / 重连后不得退化成全新可操作卡；在途期间不渲染可点反馈框与确认按钮 |
| 3 | **D3**：因取消 / 终止导致的修订失败记为 `aborted`（节点 `Aborted`、turn `failure_class=cancelled`），不是 `failed` / `provider_err`；对话流出现一句明确的「修订被中断，你的选择未生效」 |

## 非目标（owner 明令不动）

- **不改**「结论卡 + 门卡主区都呈现」的既有设计（REQ-WSC-11 `work-item-plan-single-candidate/spec.md` 的「结论卡**与人工门卡主区** SHALL 先呈现」）。本 change 只约束**已收口**门卡，**未收口**的活门卡行为与 REQ-WSC-11 逐字不变。⇒ 方案 C（折叠结论卡人话块）**不做**。
- **不改**「标题必须不同名」的既有约定（结论卡「审核结论待人工分诊」与门卡「需要判断 reviewer 意图」/「需要人工确认」，F-50 单标题制）。
- **不引入第二套「中止」语义**：D3 复用既有 `RunAbortOutcome` 三态（`NoActiveRun` / `Completed` / `TimedOut`）与既有 `aborted` 语义链（REQ-WIGA-16/17、`TimelineNodeStatus::Aborted`、`HumanGateTurnFailureClass::Cancelled`、前端 `isTerminalTimelineStatus` 白名单含 `aborted`）。本 change 不新增枚举值、不新增状态。
- **不重排收件箱**：收件箱只在 `gate.closed === null` 时挂门条目，收口后不重复，**无此缺陷**，不动。
- **不清理留档卡**（`resolution:"superseded"`）的预算历史 meta 行——既有断言钉死「已留档卡读历史事实、显示『预算历史不可用』」。留档卡仅纳入 D1 的「标题陈述式」「不渲染人话选项块」两项。

## 范围

| 能力（capability） | 变更 |
|---|---|
| `work-item-plan-single-candidate` | ADDED REQ-WSC-26（已收口门卡不得呈现行动号召，D1）、ADDED REQ-WSC-27（门卡必须表达修订 / 复评在途态且刷新不退化，D2） |
| `work-item-plan-conversational-gate` | ADDED REQ-CG-10（取消导致的修订失败记为中止并明说选择未生效，D3） |

## 影响

- 前端：`GatePromptEntry.tsx`（标题 / 选项块 / meta 行 / 徽章说明 / 在途锁）、新增在途态选择器、`workspace-chat-rebuild.ts`（对话流中止提示条目）、`gate-prompt-copy.ts`（文案常量）。
- 后端：`conversational_gate.rs`（节点收口按失败类分流）、`sc_manual_revision.rs`（驱动错误分类）、`provider_drive/work_item_plan.rs`（中止哨兵常量）。
- 既有断言：`human_gate_revision_cancel.rs` 中「取消收口修订载体节点为 Failed」的一条断言按 REQ-WIGA-16 改为 `Aborted`（理由见 design.md 决策 4）。
- 无 wire 变更、无持久化格式变更、无迁移。
