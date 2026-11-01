# Optimized sanity（SDD Task 7.2 选项 A：真机验证 + 优化后 sanity）

- 运行日期：2026-08-22
- API：`http://127.0.0.1:4317`（worktree `feat-b-0808-add-monorepo` @ `7ff2cf43`，含 sentinel 协议 / sliding window / kimi client_services，二进制 2026-08-22 03:01 编译，cargo watch 热重载）
- 样本策略：与基线一致——每组合 1 个独立 Issue，冻结语料 `corpus/05-simple-pure-function.md`，author/reviewer 同 provider，`review_rounds=1`，superpowers+openspec 开启。
- 等待上限：单样本 300s（基线为 120s）；choice 交互自动回传首选项。
- 驱动方式：Node WS driver（`/tmp/story-run.mjs`），原始日志在 `/tmp/story-runs/*.jsonl`。

## 步骤 1：真机 kimi bash/grep 恢复验证（P2a）—— **失败（bug 仍复现）**

样本：`issue_0007` / `workspace_session_0007`（kimi_code/kimi_code）。

author 首轮（timeline_node_002）中 kimi 请求的工具调用结果：

| 工具 | 次数 | 结果 |
|---|---:|---|
| Read | 14 | 全部成功（真实文件内容输出，如 `naruto/package.json`） |
| Glob | 2 | **全部 failed，exit_code=1，output 含精确文本 `ACP terminal capability is unavailable`** |
| Bash | 1 | **failed，exit_code=1，output 含精确文本 `ACP terminal capability is unavailable`** |
| Grep | 0 | 本样本未调用 |

失败事件原文（Aria execution event，来自 `/tmp/story-runs/kimi_code-*.jsonl`）：

```text
{"title":"Glob","status":"failed","output":"{\"pattern\": \"openspec/**/*.md\"}{\"pattern\":\"openspec/**/*.md\"}ACP terminal capability is unavailable","exit_code":1}
{"title":"Glob","status":"failed","output":"{\"pattern\": \"cadence/designs/**\"}...ACP terminal capability is unavailable","exit_code":1}
{"title":"Bash","status":"failed","output":"{\"command\": \"ls -la /home/michaelche/workspace/github/naruto/ && ... find ... | head -50\", ...}ACP terminal capability is unavailable","exit_code":1}
```

交叉证据：kimi CLI 侧 wire log `~/.kimi-code/sessions/wd_naruto_4d73fb6dca57/session_40217633-*/agents/main/wire.jsonl`（本次运行 09:52 CST）中，同样 3 条 `tool.result` 带 `"isError":true`、output 即该错误文本——错误由 kimi CLI（v0.38.0）在 ACP 模式下生成。

关键事实链：

1. Aria 侧 src 已包含 4b53b08e（ACP 客户端服务），`kimi_code_provider/session.rs` 的 `initialize` params 已带 `clientCapabilities: {"fs":{...}, "terminal": true}`，错误字符串不存在于 Aria 源码。
2. 运行中的二进制（03:01）确认包含该提交。
3. 尽管如此，kimi CLI 对 Bash/Glob 仍返回 "ACP terminal capability is unavailable"。

结论：**P2a 判定失败**——优化后（sentinel/滑动窗口/client_services 已合入、真机、当次会话）kimi 的 Bash/Glob 仍不可用。推测为 kimi CLI 0.38.0 对 terminal 能力的方言/握手判定与 Aria 广播的 boolean dialect 不匹配（kimi 在收到 `terminal:true` 后仍认为能力不可用），需要针对 0.38.0 实测其期望的 capability 结构，属于 P2a 修复未达验收，而非"未生效的旧二进制"。

## 步骤 2：三组合优化后 sanity（每组合 1 样本）

| 组合 | Issue / Session | choice 交互（自动首选项） | author 首 artifact | reviewer | retry | 300s 内判定 |
|---|---|---|---:|---|---:|---|
| kimi_code/kimi_code | issue_0007 / ws_0007 | 1 次（56.7s，非有限输入处理→选"原样返回"） | **87.5s** | review pass @121.9s；随后 accept_finalize 发出 | 0 | author+review 完成；finalize 后 ws 关闭，300s 上限截停记录 |
| claude_code/claude_code | issue_0008 / ws_0008 | 1 次（146.9s，"请确认 2 个问题"→首选项） | **217.7s** | reviewer 217.8s 启动，300s 窗口内未出结论 | 0 | author 完成；**reviewer 未完成（超时）** |
| pi/pi | issue_0009 / ws_0009 | 1 次（101.5s，非有限输入→"超出范围不定义"） | **183.7s** | review verdict=needs_human @284.5s；finalize 发出 | 0 | author+review 完成；needs_human 转人工 |

补充：pi 样本中 bash 工具 3/3 成功执行（read 9/9 成功）；kimi 样本 Read 14/14 成功、Bash/Glob 0/3 成功；claude 样本 Bash 2/2、Read 4/4、Skill/codegraph 全部成功。

## 与基线对比结论

- 基线（120s 窗口）：三组合 author 首轮 3/3 全部未完成，无 artifact、无 provider session ID。
- 优化后（300s 窗口）：三组合 author 首轮 3/3 全部产出 artifact（87.5s / 217.7s / 183.7s），choice 交互自动应答后均可推进；kimi、pi 走完 review（pass / needs_human），claude reviewer 在窗口内未完成。
- **结论：优化对首轮 author 完成率有决定性改善（0/3 → 3/3）**；但 P2a 的 kimi Bash/Glob 恢复未通过，story 链路在 kimi 下仍以"降级只读"方式工作（本次靠 Read+AskUserQuestion 仍完成了 story）。
- 口径注意：本文件仍是每组合 1 样本 sanity，非 5 样本 campaign，不能作为 release-gate 统计。
