# Subagents 使用规范（项目级）

> **适用范围**：本项目所有会话、所有需求/任务——不随单个任务或交接文档过期。
> **版本**：v2.0（2026-09-22 拆分；v1.x 通用内容已提为用户级规则，见下）。
> **启用方式**：CLAUDE.md / AGENTS.md 已引用本文件。

---

## 0. 与用户级规则的分工（先读这段）

v1.x 的十项铁律中约七成是跨项目恒真内容，已提为**用户级规则**，由
`Cadence-skills/agent-rules/` 经 `install-agents.sh` 分发到 `~/.omp/agent/rules/`，
在所有项目的所有 agent 的 `<domain-rules>` 中按需读取：

| 用户级规则 | 内容 | 可见范围 |
|---|---|---|
| `rule://subagent-grounding` | 规则与工具的渐进式发现协议 | 所有 agent |
| `rule://subagent-protocol` | 编排铁律、并行细则、派工纪律、git 安全、报告契约 | 所有 agent |
| `rule://subagent-roster` | 角色指派表、模型链、通道运维 | **仅主会话**（`agents: main`） |

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

## 7. 演化历史

| 日期 | 变更 | 触发 |
|---|---|---|
| 2026-09-15 | 初版十项铁律（禁并行实现 worker） | 39251070 并行事故后用户钉死 |
| 2026-09-17 | 铁律 5 条件豁免（文件零交集可并行） | 用户裁决「不能并行吗」 |
| 2026-09-18 | 铁律 5 升级默认并行（不设上限 + 审查扩实例 + stash 三步协议）+ 沉淀为项目规则 | 用户裁决 |
| 2026-09-19 ~ 09-22 | 多轮模型通道与指派调整（详情已随指派表迁入 `rule://subagent-roster`） | 用户配置指令 |
| 2026-09-22 | **v2.0 拆分**：通用铁律、并行细则、报告契约提为用户级 `subagent-protocol`；指派表提为 `subagent-roster`（仅主会话）；新增 `subagent-grounding` 接地协议；本文件瘦身为项目专属。实测依据：子代理不接收 `AGENTS.md`/`CLAUDE.md` 正文，`<domain-rules>` 清单 + `rule://` 按需读取是唯一可靠通道 | 用户指令「优化 subagent prompt，规则渐进式加载」 |

> 本节只记变更不重复正文——正文始终为现行版。
