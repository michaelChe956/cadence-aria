# Subagents 使用规范（项目级）

> **适用范围**：本项目所有会话、所有需求/任务——不随单个任务或交接文档过期。
> **版本**：v2.0（2026-09-22 拆分；v1.x 通用内容已提为用户级规则，见下）。
> **启用方式**：CLAUDE.md / AGENTS.md 已引用本文件。

---

## 0. 与用户级规则的分工（先读这段）

v1.x 的十项铁律中约七成是跨项目恒真内容，已提为**用户级规则**，由
[`cadence-subagents-config`](https://github.com/michaelChe956/cadence-subagents-config)
的 `agent-rules/` 经 `install-agents.sh` 分发到 `~/.omp/agent/rules/`，
在所有项目的所有 agent 的 `<domain-rules>` 中按需读取：

| 用户级规则 | 内容 | 可见范围 |
|---|---|---|
| `rule://subagent-grounding` | 规则与工具的渐进式发现协议 | 所有 agent |
| `rule://subagent-protocol` | 编排铁律、并行细则、派工纪律、git 安全、报告契约 | 所有 agent |
| `rule://subagent-roster` | 角色指派与派工规则、通道运维 | **仅主会话**（`agents: main`） |
| `rule://subagent-routing-bridge` | 内置角色名映射与设计派工硬规则（alwaysApply 常驻） | **仅主会话**（`agents: main`） |

> 🔴 **前置依赖：上表四条规则不随本仓库分发。** 未安装时 `rule://` 引用无法解析，
> 本文件第 0 节以外的协作纪律将全部缺失。首次在本仓库工作前执行一次：
>
> ```bash
> git clone https://github.com/michaelChe956/cadence-subagents-config.git
> cd cadence-subagents-config && ./install-agents.sh install
> # 装完需重启 omp 或 /clear——规则快照在会话启动时固定
> ```
>
> 校验：`./install-agents.sh status` 应列出 4 条 `[本脚本管理]` 规则与 11 个 agent 定义。
> 该仓库同时分发 11 个 agent 定义（anthropic-task、glm53-task、ds-task、fe-fix-task、max-task、
> design-task、design-task-max、k3-reviewer、scout、oracle、researcher）——未安装则派工时找不到这些角色。

**本文件只保留本项目专属内容。** 通用纪律不在此重复——需要时读上表对应规则。
项目专属约束的活动索引见 `cadence/project-rules/subagent-project-index.md`。

---

## 1. 本项目专属的并行代价协议

- **单 crate 编译无法隔离并行方工作树半成品** → 被阻断方以「定向测试绿 + commit + 报告记录阻断」收尾；
  全量门禁与部署等对方可编译提交点后由 controller 补跑。
- **cargo 构建锁排队 = 正常等待**，禁 kill 对方进程。
- 并行边界按本项目文件面切分的既有分法：Rust 段 ∥ 前端段 ∥ 测试基建段，同段内串行。

---

## 2. 本项目专属的验证与部署纪律

- **服务器重启必须 PID / exe / md5 三对账**，health ok 不算数。
- **改前端必须重建二进制**（前端资源经 rust-embed 编译期嵌入）。
- 长跑命令 bash 侧用 `timeout:0`（driver 内部 timeout 兜底）。
- **部署窗口**：查用户活动会话（5min durable mtime），无活动即安全重启。
- UI 验证用真实浏览器 + 真实 provider，截图证据入库；新形态缺口停下原样记录，等用户裁决。

---

## 3. 本项目的路径约定

- 计划文件：`cadence/plans/`
- 台账：`.superpowers/sdd/<当前 plan>/progress.md`
- Worker 报告：`.superpowers/sdd/<plan>/<task>-report.md`（只写盘不 `git add`；
  **已 track 的文件 `.gitignore` 不生效**——worker 误 add 报告时 controller 用 `git rm --cached` 修正）

---

## 4. 会话入口（每会话开始执行）

1. 读 `rule://subagent-roster` 确认当前角色指派与模型链。
2. 读本文件 + `cadence/project-rules/subagent-project-index.md` 确认项目专属约束。
3. 通道探针确认各角色健康。
4. 由 `cadence/plans/` 的计划文件确定 Task 分组 → 按 `rule://subagent-protocol` §2 切并行边界。
5. 台账记并行组再派工。

---

## 5. 本项目已验证的并行先例（可直接复用）

- Rust 实施 ∥ 前端实施（P2-T2 ∥ IssueIdFix、P2-T3 ∥ P1Patch）✅
- 实施 ∥ 用户浏览器验证（种子 + controller 双头）✅
- 双审 k3 ∥ oracle（每轮契约/计划标配）✅
- 借部署窗口：DM pause 并行方 → stash 半成品 → build → 部署 → pop 还原（两次先例）✅

---

## 6. 审查者工具限制与处置（本项目观测）

- 审查/裁决角色现已获准**写自己的报告文件**（2026-09-22 调整）：brief 指定路径即可写，禁 `git add`；
  未指定路径则在 yield 正文返回，由 controller 代存。
- oracle 传输中断（遗言含发现）→ 重派同 brief 并注入遗言发现（两次先例，重派后均成功）。
- 审查 job 偶发异常消失（无报告退出）→ 直接重派同 brief。

---

## 7. 派工防卡死（2026-11-05 实测，本项目强制）

**现象**：实施 worker 连续 3 次只读不写（`OrderAndRedText` 58 次调用全为 read/grep，
零 write；转录里「let me check the last details before writing tests」同一句说 9 遍），
48 分钟零产出。同批 `glm53-task` 26 分钟做完 9 条红 ⇒ **差异在 brief 形状，不在模型**。

### 7.1 根因（4 条叠加，全在 brief 上）

| # | 根因 | 后果 |
|---|---|---|
| 1 | brief 给了**无界活**（「逐条同步各 fixture」） | worker 必须先穷举全部命中点才敢动手 ⇒ 探索无底洞 |
| 2 | brief **已含完整答案**（精确常量/文案/class） | 无决策可做 ⇒ 只剩「验证整个爆炸半径」⇒ 它就去验证 |
| 3 | 无**机械步数上限** | 「读完就写」全靠模型自觉 ⇒ 每次都说「最后一批」 |
| 4 | controller **未按铁律 8 主动 DM 催分批提交** | 卡住时产出为零，无中间态可捞 |

### 7.2 强制条款（brief 必写，缺项即视为违规派工）

1. **闭集清单**：brief 列出**文件 + 行号 + 精确 old→new 字符串**。worker 只改清单内；
   清单外命中点 = **记录不改**，不得扩范围搜索。
2. **禁止「先读全仓库」**：brief 第一动作必须是「写红测试」。已给答案的项**不许再查**。
3. **机械上限**：读文件 ≤6 次、grep ≤8 次；超限即按清单盲写。禁止「再检查最后一批」。
4. **切片 ≤3 文件**；**每写完一个红测试就 commit 一次**（显式列文件，禁 `git add -A`）——
   卡死时至少有可捞的中间产出。
5. **controller 看门狗**：worker 启动后 **15 分钟无 commit ⇒ 立刻 DM**
   「提交你已完成的全部内容，然后停」；30 分钟无 commit ⇒ kill，由 controller 接手或按新 brief 重派
   （重派 brief 必须带 git status/diff + 已完成 commit 清单 + 剩余切片，禁裸 resume）。
6. **状态自检**：worker 每完成一个可验证步骤须在回复里给出「已写文件 / 已跑命令 / 结果」；
   controller 见到连续 2 次「再看最后一处」而无 write ⇒ 立即 DM 收束。

### 7.3 派工模板（小切片）

```
【目标】一句话，可验证。
【闭集】文件A:行号 —— 精确 old → new（逐字）；文件B:行号 —— 精确 old → new。
【第一步】先写红测试（文件C），跑它看到失败，把失败输出贴出来。
【上限】读文件≤6、grep≤8；清单外的命中点记录不改。
【提交】每写完一个红测试 commit 一次，显式列文件，禁 git add -A。
【验证】贴真实命令输出。禁 dry-run 冒充执行。
```

---

## 8. 演化历史

| 日期 | 变更 | 触发 |
|---|---|---|
| 2026-09-15 | 初版十项铁律（禁并行实现 worker） | 39251070 并行事故后用户钉死 |
| 2026-09-17 | 铁律 5 条件豁免（文件零交集可并行） | 用户裁决「不能并行吗」 |
| 2026-09-18 | 铁律 5 升级默认并行（不设上限 + 审查扩实例 + stash 三步协议）+ 沉淀为项目规则 | 用户裁决 |
| 2026-09-19 ~ 09-22 | 多轮模型通道与指派调整（详情已随指派表迁入 `rule://subagent-roster`） | 用户配置指令 |
| 2026-09-22 | **v2.0 拆分**：通用铁律、并行细则、报告契约提为用户级 `subagent-protocol`；指派表提为 `subagent-roster`（仅主会话）；新增 `subagent-grounding` 接地协议；本文件瘦身为项目专属。实测依据：子代理不接收 `AGENTS.md`/`CLAUDE.md` 正文，`<domain-rules>` 清单 + `rule://` 按需读取是唯一可靠通道 | 用户指令「优化 subagent prompt，规则渐进式加载」 |

> 本节只记变更不重复正文——正文始终为现行版。
