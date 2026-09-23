# F-43 前端面复验报告（coding attempt 驾驶舱四问题）

- 四态：**DONE**
- 分支/worktree：`.worktrees/feat-b-0808-add-monorepo`（基线 417ee950）
- 提交：`68f5875d fix(coding-ui): 选择卡 clamp 常显+到达通知+设置 slot 补挂+日志叠印（复验 F-43）`
- 改动面：**仅 web/**（14 文件，10 改 4 新）；未触碰 src/（后端 F-43 ④ 由并行 BE worker 负责）
- 验收证据：`cd web && npm test` → **181 文件 / 1705 测试全绿**；`pnpm exec tsc --noEmit` → **exit 0**；另附真实浏览器量测（见 §5）

---

## 1. ① 选择卡可操作性（ChoiceRequestEntry）

**根因**：prompt 原样 `{prompt}` 平铺为普通文本。活库实测危险命令 prompt 长度 **~3400 字符 / 数十行**（`coding_choice_gate_0001/0002.json` 原文），卡片被撑到视口之外，选项 radio 与「提交选择」落在卡尾折叠区外；同时命令正文没有等宽呈现。

**改动**（`web/src/components/chat-workspace/entries/ChoiceRequestEntry.tsx`）：
- `promptSegments()` 按 provider（pi）实测形态切段：`⚠️ Dangerous command:` 标记行 / 命令正文 / `Allow?` 尾行（正则见源码；无标记行时整段为文本，行为与旧版一致；命令为空时安全回退文本）。
- 文本段 → `<p data-testid="choice-prompt-text">`：`whitespace-pre-wrap break-words` + **`line-clamp-6`**（仅在需要时），配 `展开全文` / `收起全文` 切换（判定：>6 行或 >240 字符，jsdom/首帧无量测故用行数+列宽估算）。
- 命令正文 → `<pre data-testid="choice-command-block" class="max-h-40 max-w-full overflow-auto whitespace-pre-wrap break-words font-mono text-[11px]">`：等宽 + **内滚**（max-h-40 封顶），不再把卡撑长。
- 选项区与提交钮顺序未动，仍在 prompt 区之后 —— clamp 后即「紧跟常显」。

## 2. ② 选择到达通知（选定的实现）

**选定**：**对话流内联常驻横幅 + 一键定位**（`web/src/components/chat-workspace/PendingChoiceNotice.tsx`），未沿用 CockpitShell 的 5s toast。

**理由（最小且有效）**：coding attempt 的 choice 不走 workspace observer（controller 实测 attach 无选择帧），`countedInbox` 不含它 → shell toast 对本案**根本不触发**；要复用需给 shell 新增对外触发 API（更大改动面）。横幅方案：`pendingChoiceEntries()` 过滤未 resolved 的 `choice_request`，显示「有 N 个选择请求待处理：<首行摘要>」+`定位选择卡`；应答/失效后自动消失。

**接线**：
- `CodingWorkspacePage`：横幅挂在**页级**（顶栏之下、状态横幅之前），不随「运行对话/运行结果」页签切换消失——卡只存在于对话页签，用户停在结果页签时同样必须看到提示（已有断言）；`定位` → `ChatListRef.scrollToEntry(entryId)`（顺带回到卡所在行）。
- `ChatCockpitPage`：横幅置于「下钻对话流」页头与列表之间（artifact/plan 视图中该 section 仍在，横幅不被遮），`定位` → 既有 `handleJumpToEntry`（顺带把视图切回对话流）。
- 两页的网格行模板按横幅存在与否切换**静态字面量**，避免 Tailwind JIT 丢类 / 网格错行（无横幅时模板与旧版逐字一致）。
- `ChatEntryList`：`choice_request` 走分组 `interruptEntries`（分组行只带分组/主条目 id），原 `scrollToEntry` 对 choice id **静默失败** → 索引表补入组内成员 id，并在 `data-entry-id` 查不到时按 `data-index` 回退定位。

## 3. ③ 设置按钮 slot 补挂

| 页面 | 结论 |
|---|---|
| coding attempt 页（`/workbench/projects/:pid/issues/:iid/coding/:aid`） | **补挂**：`CodingWorkspacePage` 顶栏（新增 `data-testid="coding-workspace-top-bar"`）注册 `cockpit-settings-slot` |
| workspace 页（work_item 会话） | **补挂**：`ChatWorkspacePageLegacy` 顶栏（新增 `data-testid="workspace-top-bar"`）注册 slot |
| cockpit 会话页（story/design/work_item_plan） | **无需改**：`ChatCockpitPage` → `CockpitPageHeader` 已含 slot（F-37 先例，代码在 36/83-87 行） |

**归属判定证据**：`readChatCockpitMode()` 白名单仅 story/design/work_item_plan；`.aria` 中 `workspace_session_0004/0005/0006` 的 `workspace_type = "work_item"` → 默认 legacy 形态。真实浏览器实测该 URL 渲染 `legacy-chat-workspace-page`（`cockpit-page` 不出现）——即 controller 观测到 `slotExists=false` 的「workspace cockpit 页」是 legacy 顶栏。两页均按 F-37 先例（`useCockpitSettingsSlotRef()` + 宿主 div）挂载。

## 4. ④ 实时日志叠印 + 横向溢出（CodingLogConsole）

**根因（行容器样式）**：日志行是 `absolute` + **内联写死 `height: 20px`**（`LINE_HEIGHT`），消息 span 同时带 `truncate`（nowrap+overflow hidden）与 `whitespace-pre-wrap break-all`（Tailwind 产出顺序：whitespace 组在 textOverflow 之后 → **pre-wrap 生效**）。流式分片含换行时，span 在 20px 定高行内折成多行、以自身内容高度溢出到相邻行 → **文字叠印**；节点名 span `shrink-0` 无上限 → 长串把行撑出容器 → **横向溢出**。

**改动**：行高交给虚拟化**按内容测量**（`measureElement` 选项：优先 ResizeObserver `borderBoxSize`，量到 0 时回退 `LINE_HEIGHT`，jsdom 下保持既有窗口化行为），移除内联 `height`；消息改为 `whitespace-pre-wrap break-words`（去掉与 pre-wrap 冲突的 truncate）；节点名 `max-w-[8rem] shrink truncate`；行容器加 `overflow-hidden` 兜底。
`LINE_HEIGHT`(20) 仍作 `estimateSize` 估算值。

## 5. 验证

### 5.1 自动化（验收口径）
- `cd web && npm test`：**181 文件 / 1705 测试全绿**（含新增 **16** 例：ChoiceRequestEntry 3、PendingChoiceNotice 4、CodingLogConsole 1、ChatEntryList 1、CodingWorkspacePage 3、ChatCockpitPage 3、ChatWorkspacePage 1 —— 均先红后绿）
- `cd web && pnpm exec tsc --noEmit`：exit 0

### 5.2 真实浏览器（vite dev 5199 → 活 aria 4317，HEAD 源码；验后已停服/关页）
- **③ coding 页** `/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_e4a4aa9d6cb245e9844774392ab820c5`：`cockpit-settings-slot`×1、`cockpit-settings-trigger`×1、`cockpit-settings-fallback`×**0**；齿轮 44×44 @ 顶栏矩形内(top 0–44, right 1556)、无 `position:fixed` 祖先；点击打开「驾驶舱设置」对话框。
- **③ workspace 页** `/workbench/workspace/workspace_session_0004`：渲染 legacy 页，slot×1 在 `workspace-top-bar` 内，fallback×**0**。
- **④ 日志几何**：21 行、**相邻行重叠 0**、内联固定行高 0 处、行高实测 `[280,140,220,20,20,240]`（11 行为多行、按内容测量）、行/滚动容器横向溢出均为 0。
- **① 布局机制（真引擎 + 应用 CSS）**：30 行长文 clamp 后高 **144px = 6×24px**（未 clamp 720px）；命令块高 **160px = max-h-40** 且 `scrollHeight>clientHeight`（内滚）、无横向溢出（未 clamp 672px）；选项行 32px 紧随其后。
- **CSS 产出核对**：`line-clamp-6 / break-words / max-h-40 / max-w-[8rem] / pre-wrap / overflow-auto / min-h-9 / amber-50|200|900` 均在 dev CSS 中实际生成（防 JIT 丢类）。

## 6. Concerns / 未覆盖

1. **①② 缺 live 端到端复验（无可用样本）**：活库当前 **0 个 pending choice**（attempt e4a4aa9d 的两道 choice gate 均 `resolved`，末次应答 04:02:34，其后无新帧；新连接补发 pending choice 正是 BE 面 F-43 ④，未落地的运行进程不会补发）。故 ①② 的证明强度为：用**从活库 choice-gates/resolved 抓到的真实危险命令 prompt**（非构造样本）写红→绿单测 + 页面级集成断言（横幅出现/定位/应答后消失）+ ① 的 clamp 机制真浏览器量测。端到端复验建议：BE 补发落地后由 controller 在真实 pending choice 上确认「横幅出现 + 一键定位 + 选项可点」。
2. `line-clamp-6` 为固定 6 行阈值；判定走行数/字符估算（无布局量测的降级），超长单行（>240 字符）也会出现「展开全文」——产品语义如此，非缺陷。
3. `ChatEntryList` 的 `Map` 索引为运行时动态键集（规则允许的情形），`ChoiceRequestEntry` 内既有局部 `isRecord` 守卫为本次改动前的既有结构（未新增同类守卫，也未改其语义）。
4. 未改 src/：F-43 ④ 后端面（新连接补发 pending choice）不在本单范围；F-35~42 面未回退（全量测试绿）。

---

## 7. fix1（k3 复审 P2 回执）— 横幅定位在「运行结果」页签下静默失效

- 提交：`4c89f494 fix(coding-ui): 横幅定位在结果页签下切回对话页签再滚动（F-43 fix1）`（文件：`web/src/pages/CodingWorkspacePage.tsx`、`web/src/pages/CodingWorkspacePage.pending-choice.test.tsx`；基线 HEAD 6db84c48，含 F-44 改动）
- **问题（P2）**：`CodingWorkspacePage.tsx` 的 `onJump` 只调 `chatListRef.current?.scrollToEntry`，而 `ChatEntryList` 仅在 `activePanel !== "results"` 且无 `activePlanRepair` 的分支挂载——用户停在「运行结果」页签时列表卸载、`ref=null`，点「定位选择卡」既无滚动也不切回，属静默无操作。
- **修法**：新增 `pendingJumpEntryId` 状态；`onJump` 置目标 + `setActivePanel("chat")`（同步调用时列表尚未重挂，不能只切页签）；新增 effect 在 `activePanel === "chat"`、无 `activePlanRepair`、且 `chatListRef.current` 非空时执行 `scrollToEntry` 并清空目标（ChatCockpitPage `jumpEntryId` 先例，:557-562）。Plan Repair 期间列表被整块替换，effect 挂起至修复会话结束/退出后再滚动；切换 attempt（addressKey 变化）时一并清空挂起目标。
- **测试（先红后绿）**：新增 `switches back to the conversation panel before scrolling when the results tab is open`——results 页签下断言对话列表不存在 → 点「定位选择卡」→ 断言切回 chat 页签、选择卡重新渲染、`scrollIntoView` 确实被调（修复前红：列表仍未挂载）。
- **验证**：`cd web && npm test` → **183 文件 / 1711 测试全绿**；`pnpm exec tsc --noEmit` → exit 0。
- **残余**：同为「未挂载即静默 no-op」的相邻入口 `handleSelectTimelineNode`（时间轴选节点后 `scrollToEntry`）在 results 页签/Plan Repair 下同样不滚动——本次未纳入（不在 k3 回执范围），如需一致行为可复用同一 `pendingJumpEntryId` 通道。
