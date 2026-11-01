# Story weak-model baseline sanity（阶段 0）

- 运行日期：2026-08-21
- API：`http://127.0.0.1:4317`
- 样本策略：每组合 1 个独立 Issue，使用冻结的 `corpus/05-simple-pure-function.md`；author 与 reviewer 均设为同一 provider，`review_rounds=1`。
- 限制：按照任务约束，每个真实 provider 运行最多等待 120 秒；若未进入 author 确认或 reviewer 阶段即停止并记录，绝不无限等待。

## API 发现

创建 Issue：

```text
POST /api/projects/project_0001/issues
{
  "title": "...",
  "description": "<corpus text>",
  "change_id": "...",
  "repository_id": "repository_0001",
  "logical_codebase_id": null
}
```

创建 Story Workspace：

```text
POST /api/projects/project_0001/issues/{issue_id}/story-specs:generate
{
  "title": "... Story Spec",
  "author_provider": "claude_code|kimi_code|pi",
  "reviewer_provider": "claude_code|kimi_code|pi",
  "review_rounds": 1,
  "superpowers_enabled": true,
  "openspec_enabled": true
}
```

`GenerateStorySpecsRequest` 的 Rust 字段为 `title`、可选的
`involved_repository_ids`、`author_provider`、`reviewer_provider`、`review_rounds`、
`superpowers_enabled`、`openspec_enabled`。生成响应中的会话字段名为
`workspace_session.workspace_session_id`。真实执行通过
`/api/workspace-sessions/{session_id}/ws`：先发 `hello`，收到 `prepare_context`
会话状态后发 `start_generation`；author 到 `author_confirm` 时自动发送
`author_decision=accept_with_review` 以尝试进入 reviewer。

## 运行记录

|组合（author/reviewer）|Issue / Workspace session|author/reviewer provider session|prompt 字符数|retry|结果|
|---|---|---|---:|---:|---|
|`claude_code` / `claude_code`|`issue_0004` / `workspace_session_0004`|未持久化（author 未完成 turn）|4,337|0|120.100 秒后仍为 `running`；仅见 provider/turn started，未到 author 确认，reviewer 未启动。|
|`kimi_code` / `kimi_code`|`issue_0005` / `workspace_session_0005`|未持久化（author 未完成 turn）|4,446|0|120.037 秒后仍为 `running`；未到 author 确认，reviewer 未启动。见下方已知 bug 复现。|
|`pi` / `pi`|`issue_0006` / `workspace_session_0006`|未持久化（author 未完成 turn）|4,243|0|120.018 秒后仍为 `running`；未到 author 确认，reviewer 未启动。Pi 发出 3 个用户 choice（模块落点、测试设施、非有限输入），runner 自动回传每题首选项，随后仍未完成。|

说明：prompt 字符数从每个持久化的 `timeline_node_002.json` 的 `prompt` 字段读取；本次 provider 事件没有暴露 input-token usage，故该列不能替代 token usage。每一运行均未产生 author/reviewer 原生 session ID；因此 retry 都是 0（无完成轮次/无 revision 节点），不是“一次成功”。

## Kimi bash/grep 已知 bug 复现证据

`issue_0005` 的真实 Kimi author 运行确实请求过 `Bash`；Aria 自动批准后，执行事件中有 5 次失败，并出现精确文本：

```text
ACP terminal capability is unavailable
```

Kimi 在这次受限样本中尝试 Bash/Glob 来检查 `openspec`/`cadence`，随后 Bash 与 Glob 标记 failed；120 秒上限到达时 author 尚未输出 artifact。这是 P2a 前的基线复现证据，不能被解读为模型产物质量失败。

## 1.3 resume usage 结论

未能进行 resume usage 实测：三组 author 首轮都没有在 120 秒窗口内完成，因此未得到可续接的 provider session ID，也没有 provider 返回 input-token usage。当前结论为 **未判定**；在 P1 实施前必须针对完成了首轮的 session 分别补测 fresh/resume usage，不能据此决定 fresh-session 窗口策略。

## 基线口径结论

本文件是每组合 1 样本的 sanity，而非 tasks.md 1.2 要求的 5 样本/组合统计 campaign。author 首次成功率、reviewer 首次 syntax+schema 通过率、full-chain 一次成功率、retry 分布与 input-token 指标均因 3/3 在 author 首轮超时而暂不可计算；不得将这三次视为 release-gate 基线。
