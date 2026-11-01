# 问题记录：`stopped_needs_human` 停机态的说明入口与文案错配

日期：2026-10-28
状态：**仅记录，未实施**（owner 裁定：先做完「悬空引用治理」再动前端）

## 一、现场（2026-10-28，session `workspace_session_auto_6bfd55b3-6ea0-4da0-9f51-af26142addc2`）

界面提示：

```
[人工说明次数已用尽
评审轮的人工说明预算已耗尽。请在下方写一两句补充说明后「继续自动化」——只开启一个新的说明回合，历史不清零。
技术详情 stage=stopped_needs_human · stop_key=plan_gate:...:human_confirm
]
```

### 真实成因与提示语**不符**

| 实测事实 | 值 |
|---|---|
`human_gate_snapshot.manual_repairs_used` | **0**（人工说明一次没用） |
`human_gate_snapshot.manual_repairs_remaining` | **3**（满的） |
`run_history.repairs_used` / `max_repairs` | **1 / 1**（自动返修额度用完） |
`human_gate_snapshot.trigger` | `verification_new_findings` |
`single_candidate_phase` | `evaluate` |

### 四节点真实经过

| 节点 | 角色 | 结果 |
|---|---|---|
`timeline_node_002` | 评审1（pi） | **4 个 Error 级缺口**（`intent_unexecutable`：intent target 逻辑代码库 `lc_aggregate_root_view_57b9b575…` 不在授权集合 `57b9b575…` 内） |
`timeline_node_006` | author（codex） | 自动返修 **1 次**（`max_repairs=1` 用完） |
`timeline_node_004` | 评审2 | **2 条 suggestion**：`contract_gap`（CT-001/CT-002 错误码语义：公告引用分类不存在应 400，依 DEC-013 / API-002 / API-004）+ `verification_unattributable`（TASK-010 的 `frontend/tests/integration` Node 20 联调测试没有任何验证检查执行） |
`timeline_node_007` | 评审3 | **failed** —— reviewer 一条 bash 写错（`ls -la .../policy/.../3/` 路径末尾多斜杠 → `Not a directory`）→ 整轮失败、**无任何 verdict**（`messages=0`、`streaming_content` 空、`artifact_ref=None`） |

**即：最后卡住的是「评审运行自己挂了」，不是「评审不通过」，更不是「人工说明预算用尽」。**

## 二、三个缺口（建议按此优先级）

### ③ provider 运行失败被当成"评审不通过"（优先级最高）

`timeline_node_007` 是 provider 侧运行失败（自己命令写错），系统却归入"评审轮 + 人工门"叙事，**让用户背锅**。
建议：provider 运行失败 → **自动重试有限次**（`review/drive.rs` 已有重试路径，本次未走到）→ 仍失败才停机；停机文案明确写"评审运行失败"而非"评审不通过"。

### ① 说明不预填 reviewer 结果

产品手里有 findings（门快照 `findings` / 时间线 reviewer_run 节点的 `verdict.findings`），却要求用户**人工转写**进输入框。
建议：停机态下输入框**初值 = 上一轮 reviewer findings 的可编辑摘要**（可改/可删/可加），「继续」时随说明一并送出。

### ② 文案一律说"人工说明次数已用尽"

`web/src/components/chat-workspace/ChatInputBar.tsx:511` 的 `PLAN_CONTINUE_NARRATIVE` 对 `stopped_needs_human` 只有**一段写死文案**，不区分成因。
建议按真实成因分四类：自动返修超限 / 人工说明预算用尽 / 复验持续挑出新问题 / **复验运行失败**。

## 三、设计意图（现状，供理解操作方式）

`stopped_needs_human` = "AI 卡住了，需要人递句话"：

1. 输入框写 1–2 句说明（**必填**，`ChatInputBar.tsx:431` 会拦空提交：「请先写一两句话说明情况」）
2. 点「**继续自动化**」（`ChatInputBar.tsx:424`）
3. 说明**随继续动作带给 AI**（唯一能影响 AI 的通道）→ 开启新回合；人工预算耗尽时只 +1 回合，**历史不清零**（`workspace_single_candidate.rs:258` `reopen_stopped_needs_human_for_bound_continuation`，owner A-04 裁决）

「代入 reviewer 结果」的输入框**只存在于呈现态**（`human_confirm` / `author_confirm`，组件 `HumanPresentationEditor`，aria-label「编辑人工说明」）；停机态只有普通聊天框，**无 findings 预填桥**（缺口①）。

## 四、本次的临时操作办法

provider **看不到评审原话**，只看得到输入框里的说明，因此需人工转写**最新一轮**（`timeline_node_004`）的 2 条要求即可；**不要**转写第一轮那 4 个 Error（已被自动返修处理，见 `timeline_node_004` 只剩 2 条 suggestion）。
