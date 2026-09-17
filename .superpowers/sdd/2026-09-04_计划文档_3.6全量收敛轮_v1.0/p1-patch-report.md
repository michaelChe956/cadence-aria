# P1 前端补丁批报告

## 范围

仅修改 `web/`，完成门卡 UX、驾驶舱流式渲染与新建 Issue 索引延迟可见性三个前端修复；未改动 `src/`。

## 交付提交

1. `c7da7c08 feat: clarify cockpit gate actions`
   - 门卡展示当前待确认方案的标题、版本、审核结果、变更摘要及最近审核摘要。
   - 显示批量选择标记；确认操作升级为主按钮。
   - 仅当 typed gate 的命令确实与当前 session 的活跃 repair reservation 不同步时显示新命令提示。

2. `2804862d feat: render cockpit provider streams promptly`
   - 流式帧合并间隔由 80ms 收敛至 50ms。
   - 驾驶舱在正式聊天条目落盘前显示活跃 provider 的缓冲流内容；正式消息出现后临时流块自动移除，避免重复内容。

3. `a7d5ba9e fix: retain newly created issues during index lag`
   - 创建请求返回的 durable Issue 作为本次刷新 overlay 参与生命周期物化，并按更新时间降序排列。
   - REST Issue 列表索引尚未同步时，`issue_0277` 仍立刻出现在队列并可按 ID 搜索。

## TDD 证据

- 门卡：新增摘要、批量选择、主确认按钮与 reservation 条件提示测试。先因缺少“等待确认的内容”红测，后通过。
- 流式：新增 50ms 前不刷出、50ms 边界合并刷出和驾驶舱临时流块/正式消息替换测试。
- Issue：新增 POST 返回 `issue_0277`、列表 GET 故意陈旧、生命周期 GET 可用时仍在队列可见且可搜索的测试；实现前红测，修复后通过。

## 最终验证

在 `web/` 执行：

```text
pnpm tsc -b
```

退出码 0。

```text
pnpm test
```

结果：`168 passed` 测试文件，`1473 passed` 测试。

测试输出保留了既有 jsdom `navigation (except hash changes)` stderr 噪声，但 Vitest 退出码为 0，全部断言通过。
