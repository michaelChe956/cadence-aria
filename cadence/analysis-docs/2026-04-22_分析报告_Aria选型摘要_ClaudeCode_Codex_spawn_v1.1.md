# Aria 选型摘要：Claude Code / Codex 的 spawn 方案

> 日期：2026-04-22
> 文档类型：分析报告
> 版本：v1.1
> 用途：给 Aria 方案设计直接使用的短摘要
> 详细版：`cadence/analysis-docs/2026-04-22_分析报告_ClaudeCode_Codex_spawn使用方式与限制调研_v1.1.md`

---

## 一、直接结论

如果 Aria 要做一期可落地方案，不要把所有能力都压到“宿主层多开 CLI 进程”上。

更稳的理解方式是：

1. `spawn` 至少有 4 层：
   - 宿主进程 spawn CLI
   - 产品内部 spawn 子 agent
   - 会话 `resume / fork`
   - SDK / server 对以上能力的程序化封装
2. Claude Code 更适合做：
   - 对话入口
   - 规划与拆任务
   - 自动 / 半自动委派
   - 多轮会话与会话分叉控制
3. Codex 更适合做：
   - 执行器
   - review 节点
   - 长驻 thread runtime
   - 需要事件流、thread/turn/item 级可观测性的后端节点

一句话总结：

```text
Claude 做 orchestrator，Codex 做 executor；
CLI spawn 做一期基础入口，SDK / app-server 做二期正式 runtime。
```

---

## 二、最关键的事实更新

相对很多旧资料，当前必须更新的事实有：

1. **Codex 已经不是只有 CLI spawn**
   - 有 `codex exec`
   - 有 `codex resume`
   - 有 `codex fork`
   - 有 `codex app-server`
   - 有 `codex mcp-server`
   - 有官方 `@openai/codex-sdk`
2. **但 Codex TypeScript SDK 不是完全脱离 CLI 的新底座**
   - 官方 README 明确说它会 wrap `codex` CLI
   - 底层通过 JSONL 走 stdin/stdout
3. **Codex 非交互 resume 已成立**
   - `codex exec resume` 可用
4. **Codex 非交互 fork 仍缺**
   - 没有 `codex exec fork`
5. **Claude Code 的 subagents 更成熟**
   - 自动委派更强
   - `--agent`、`--agents`、`@mention` 更完整
6. **Claude agent teams 存在，但仍是实验能力**
   - 不能把它当成零风险生产底座

---

## 三、为什么不应该把“全部统一成 CLI spawn”

因为 CLI spawn 只解决一件事：

```text
拉起一个 agent 去干活
```

它解决不了下面这些系统问题：

- 对话分叉和代码状态分叉同时成立
- 长连接与中途插话
- 稳定的 thread / session 生命周期
- 多 agent 生命周期治理
- 并发写冲突
- 审批与审计
- 持久执行与调度
- 细粒度事件观测
- 凭证与最小权限隔离
- 大规模并发与背压

所以 CLI spawn 的正确定位是：

- **基础执行原语**

而不是：

- **完整 agent runtime**

---

## 四、Claude Code 与 Codex 的建议定位

## 4.1 Claude Code

适合：

- 用户对话入口
- 任务拆分
- 自动委派 subagents
- 需要会话 fork / resume 的总控
- 需要 Agent SDK 做多轮控制的场景

优先选用：

- 一次性任务：`claude -p`
- 多轮服务：Claude Agent SDK
- 一层委派：subagents

不建议只靠 `claude -p` 的场景：

- 长期多会话服务
- 复杂权限回调
- 需要更强程序化 session 管理

## 4.2 Codex

适合：

- 单次执行型任务
- review / 批处理
- 长驻 thread
- 需要 thread/turn/item 级事件流
- 作为其它 agent 的执行节点

优先选用：

- 一次性执行：`codex exec --json`
- 应用内集成：`@openai/codex-sdk`
- 正式 runtime / UI 后端：`codex app-server`
- 作为工具能力暴露给别的 agent：`codex mcp-server`

最值得记住的风险点：

- **Codex 当前最大 CLI 缺口不是 resume，而是 `codex exec fork` 缺失**

---

## 五、确认：spawn 无法单独解决的场景

结论是：**有，而且很多。**

最典型的 8 类是：

1. **要分叉对话，又要隔离代码状态**
   - `fork` 只分叉对话，不等于 worktree
2. **要长连接、中途插话、可打断**
   - 裸 CLI spawn 不优雅
3. **要稳定治理多个子 agent 的 wait / resume / close**
   - spawn 只解决启动，不解决完整生命周期
4. **要并发写代码且尽量避免冲突**
   - 必须有 worktree 和 merge 策略
5. **要可靠审批、审计、最小权限**
   - 仅靠 spawn 不够
6. **要终端关闭后继续跑**
   - 需要 scheduler / server / cloud runtime
7. **要细粒度 observability**
   - 需要 thread / turn / item 级事件，不只是 stdout
8. **要多用户、多租户、大规模并发**
   - 需要 session store、queue、resource governance

---

## 六、Aria 一期建议

最现实的落地方式是：

1. 用 CLI spawn 跑通一期
   - Claude：`claude -p`
   - Codex：`codex exec --json`
2. 默认使用 worktree 隔离并发任务
3. 明确持久化 `sessionId / threadId`
4. Claude 负责规划与分发，Codex 负责执行与 review
5. 把下面 3 项直接列入架构风险
   - `codex exec fork` 缺失
   - conversation fork 不等于 filesystem fork
   - 持久多 agent 会话需要显式生命周期治理

---

## 七、升级触发条件

出现以下任一信号，就不应继续停留在纯 CLI spawn：

1. 需要多轮会话与 resume / fork
2. 需要长驻 thread
3. 需要 turn 级 steer / interrupt
4. 需要 item 级事件观测
5. 需要正式的 UI / 后端集成
6. 需要多 agent 持久协作

升级方向：

- Claude：升级到 Agent SDK
- Codex：升级到 SDK 或 app-server

---

## 八、最终建议

如果你问“Aria 现在怎么选最稳”，建议是：

1. 不要把 Claude 和 Codex 当成两个完全同构的执行器
2. 不要把 CLI spawn 当成最终架构
3. 一期先用 CLI spawn + worktree 跑通
4. Claude 做总控，Codex 做执行
5. 二期再升级到 Claude Agent SDK 与 Codex app-server / SDK

