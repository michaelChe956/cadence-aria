# Subagents 使用规范（项目级）

> **适用范围**：本项目所有会话、所有需求/任务——不随单个任务或交接文档过期。
> **版本**：v1.0（2026-09-18 沉淀；规则体演化历史见 §7）。
> **启用方式**：CLAUDE.md 已引用本文件；每会话开始时按「会话入口」执行。

---

## 1. 角色指派表

| 角色 | 指派 | 通道降级链（2026-09-22 用户调整） | 全挂处置 |
|---|---|---|---|
| **实施（worker）** | **ds-task（优先）** | ✅ tydic-openai/deepseek-flash（2026-09-22 用户开放并优先） | 通知用户 |
| **实施（worker 备）** | glm53-task | ✅ my-anthropic/glm-5.3 → tydic-openai/deepseek-flash | 通知用户 |
| ter-task | 🟡 **暂缓（用户裁定太慢）** | 不派工（等用户通知恢复） | 等用户通知解除 |
| 审核（reviewer） | k3-reviewer | ✅ 可多实例扩容；my-anthropic/kimi-for-coding → dihua-openai/gpt-5.6-sol | 通知用户 |
| 勘察（scout） | glm5.3-f-scout | ✅ my-anthropic/glm-5.3-flash → bingqi/glm-5.3-flash | 通知用户 |
| 裁决 | oracle | ✅ my-anthropic/k3 → dihua-openai/gpt-6-astra → my-openai/gpt-5.6-sol | 分歧挂起等用户 |
| max-task | 🔒 **仅 oracle 推荐+用户批准** | 模型已配（dihua-openai/gpt-6-astra），不进常规派工链；oracle 认为任务过于复杂/繁琐时可建议使用，用户裁决 | 等用户批准 |

- **实施链（2026-09-22 用户再调）**：**ds-task 优先**（tydic-openai/deepseek-flash，用户开放）；glm53-task 为备选（my-anthropic/glm-5.3→tydic-openai/deepseek-flash）；**ter-task 暂缓**（用户裁定响应太慢，等通知恢复）；k3-reviewer 链=**my-anthropic/kimi-for-coding**→dihua-openai/gpt-5.6-sol 降级；oracle 链=**my-anthropic/k3**→dihua-openai/gpt-6-astra→my-openai/gpt-5.6-sol。
- **分层派工（2026-09-22 用户再调）**：**大/多步实施派 ds-task**（能力优先）；**机械小改/小任务派 glm53-task**（快）。max-task 不在常规派工链——仅 oracle 判定任务过于复杂/繁琐并建议后，由用户裁决是否使用。
- **模型切换先例**：改 `~/.omp/agent/agents/<name>.md` 的 `model:` 列表（首位优先、降级通道留尾）——2026-09-21 按用户指令调整 glm53-task/k3-reviewer 两文件降级位（此前 2026-09-19 调整三文件+ter/ds 加 SUSPENDED）。
- **通道探针**：派工前秒投一行探针（各角色一行回复即证健康）；通道挂的表现=404 model_not_found / 401 key disabled / 挂起无响应。

---

## 2. 十项铁律

1. **尽量用 subagents 工作，controller(main) 只做编排/裁决整合/最终验证**——读代码/分析/实现/审查/写计划文档全部派出去，保主会话上下文不满。
2. **计划文档必双审后才开工**（k3 接地审查+oracle 裁决一致性，并行；发现按 ruling 注入后续 Task 派工）。
3. 每 Task 实施（TDD 红→绿）→ k3 独立审查（diff 包即视图，不重跑测试采信报告计数；全量计数逐模块对账）→ findings 进 fix loop（≤5 轮：R1-3 续派原实施者，R4-5 换更强模型全新派工；每轮修复+范围化复审）→ 终审（全分支 diff+parked 分诊）→ 一轮修复波+复审收口。
4. 续任务一律全新自含派工（带状态恢复指引/裁决注入），**禁裸 resume**。
5. 🔴 **能并行的尽量并行（2026-09-18 用户终裁）**——controller 主动寻找并行机会，**不设并行度上限**：
   - **实施并行**：计划期按文件面切好并行边界（如 Rust 段∥前端段∥测试基建段），同段内串行；派工时每组 worker 明确「独占文件清单+禁碰清单」，台账记并行组与边界。
   - **流水线并行**：T(N) 实施 ∥ T(N-1) 审查 ∥ T(N+2) brief 预备——三线常态重叠。
   - **审查并行**：k3 可开多实例并行审不同 Task（审查只读无冲突；扩实例≠换人，模型仍 k3）。
   - **已知代价协议**：单 crate 编译无法隔离并行方工作树半成品→被阻断方以「定向测试绿+commit+报告记录阻断」收尾，全量门禁与部署等对方可编译提交点由 controller 补跑；cargo 构建锁排队=正常等待，禁 kill 对方进程。
   - **冲突回退（stash 三步协议，缺一不可）**：文件撞车时 controller 可 stash 一方让另一方先走——①stash 前 `git status` 快照+涉事文件清单记台账 ②stash 后核对工作树仅剩另一方改动 ③pop 后逐文件 diff 核对无丢失再继续。
6. 🔴 **并行 worker 活跃时 controller 禁 `git add -A`**（39251070 事故）；worker 提交一律显式列文件；禁抹除性 git（reset/revert/clean；stash 按铁律 5 三步协议执行）。
7. 审查角色不可用时通知用户，不自主降级/换人（**扩实例不属换人，见铁律 5**）；504/超时≠失败，先核对文件是否已改再决定重派。
8. 大任务 3h 上限+勤 commit（controller 主动通牒先例：worker 裸奔近 3h 时 DM 催分批提交）；worker 报告写 `.superpowers/sdd/<plan>/`（gitignore 不入库；**已 track 的文件 gitignore 不生效——worker 误 add 报告时 controller `git rm --cached` 修正**）。
9. 服务器重启必须 PID/exe/md5 三对账（health ok 不算数）；**改前端必须重建二进制**（rust-embed 编译期嵌入）；长跑 bash 侧 `timeout:0`（driver 内部 timeout 兜底）。部署窗口=查用户活动会话（5min durable mtime）无活动即安全重启。
10. 真实跑绝不伪造、绝不 dry-run 冒充；UI 验证用真实浏览器+真实 provider，截图证据入库；新形态缺口停原样记录待用户裁决。

---

## 3. 并行执行细则（铁律 5 展开）

### 3.1 并行机会识别（controller 职责）

| 时机 | 并行模式 | 判定依据 |
|---|---|---|
| 计划期 | 按文件面切分实施段 | Task 的 Files 清单无交集→可分组并行 |
| Task 间 | 流水线三线重叠 | T(N) 实施完成即派 T(N-1) 审查+预备 T(N+2) brief |
| 审查期 | k3 多实例 | 不同 Task 的 diff 包互相独立 |
| 契约/计划期 | k3 接地∥oracle 裁决 | 双审并行（铁律 2 标配） |
| 测试期 | 种子 subagent 并行+controller 浏览器 | 种子只写各自 outdir；浏览器归 controller |

### 3.2 派工纪律

- 每个并行 worker 的 brief 必须含：**独占文件清单**（只许改这些）+**禁碰清单**（并行方正在改的）+「有阶段性成果立即分批 commit」。
- 并行组派工前 controller 在台账记：`并行组=[worker:文件面]×N+边界依据`。
- worker 之间不直接协调文件——归 controller 仲裁（先问归属再动工；「read 缓存漂移」先重读最新文件再判断）。

### 3.3 已验证的并行先例（可直接复用）

- Rust 实施∥前端实施（P2-T2∥IssueIdFix、P2-T3∥P1Patch）✅
- 实施∥用户浏览器验证（种子+controller 双头）✅
- 双审 k3∥oracle（每轮契约/计划标配）✅
- 借部署窗口：DM pause 并行方→stash 半成品→build→部署→pop 还原（两次先例）✅

---

## 4. 会话入口（每会话开始执行）

1. 读本文件（本文件为 subagent 使用唯一权威）。
2. 通道探针（§1）确认各角色健康。
3. 当前任务的计划文件（cadence/plans/）确定 Task 分组→按 §3 切并行边界。
4. 台账（.superpowers/sdd/<当前 plan>/progress.md）记并行组再派工。

---

## 5. Worker 报告契约

- 路径：`.superpowers/sdd/<plan>/<task>-report.md`（只写盘不 git add）。
- 内容：做了什么/红灯绿灯证据（命令+输出摘要）/commit 列表/自我审查发现/concerns。
- 返回 controller：状态（DONE/DONE_WITH_CONCERNS/NEEDS_CONTEXT/BLOCKED）+commits+一行测试摘要+concerns。
- 实施者自发现缺陷可先修后报（T9 先例：先于审查落修被追认可）——报告必须如实记录。

---

## 6. 审查者工具限制与处置

- k3/oracle 偶发只读无法写报告文件→yield 全文由 controller 代存（两例先例）。
- oracle 传输中断（遗言含发现）→重派同 brief 并注入遗言发现（两次先例，重派后均成功）。
- 审查 job 偶发异常消失（无报告退出）→直接重派同 brief。

---

## 7. 演化历史

| 日期 | 变更 | 触发 |
|---|---|---|
| 2026-09-15 | 初版十项铁律（禁并行实现 worker） | 39251070 并行事故后用户钉死 |
| 2026-09-17 | 铁律 5 条件豁免（文件零交集可并行） | 用户裁决「不能并行吗」 |
| 2026-09-18 | **铁律 5 升级默认并行（不设上限+k3 扩实例+stash 三步协议）+沉淀为项目规则** | 用户裁决「能并行的尽量并行多 agents」+「作为项目规范沉淀」 |

> 本节只记变更不重复正文——正文始终为现行版。
| 2026-09-19 | **用户指令调整通道链**：worker 链 my-anthropic/glm-5.3→bingqi；k3 降级 ds-flash；scout 改 my-anthropic flash 优先；ter/ds 暂停等通知 | 用户配置指令 |
| 2026-09-21 | **用户指令再调降级链**：worker 降级位 bingqi/glm-5.3→**tydic-openai/deepseek-flash**；k3 降级位 tydic-openai/deepseek-flash→**my-openai/gpt-5.6-sol**（初版 my-openai/deepseek-flash / dihua-openai/gpt-5.6-sol 实测不可用，用户当场纠正） | 用户配置指令 |
| 2026-09-22 | **ter-task 恢复并设为优先 worker**（dihua-openai/gpt-5.6-terra；探针 4m18s 通过）；glm53-task 降为备选；ds-task 仍暂停 | 用户指令「尝试一下 ter-task 能不能用?能用的话优先使用」 |
| 2026-09-22 | **四项再调**：①ds-task 开放并设为优先 worker（tydic-openai/deepseek-flash）；②ter-task 暂缓（用户裁定太慢）；③oracle 首位改 dihua-openai/gpt-6-astra→my-anthropic/k3 降级；④k3-reviewer 首位改 dihua-openai/gpt-5.6-sol→my-anthropic/kimi-for-coding 降级；⑤max-task 模型改 dihua-openai/gpt-6-astra | 用户配置指令 |
| 2026-09-22 | **用户纠正**：k3-reviewer 链改回 my-anthropic/kimi-for-coding 首位→dihua-openai/gpt-5.6-sol 降级（controller 首调时顺序写反）；max-task 从常规派工链移除，改为仅 oracle 判定任务过于复杂/繁琐时建议+用户裁决 | 用户纠正指令 |
| 2026-09-22 | **oracle 模型链再调**：my-anthropic/k3 → dihua-openai/gpt-6-astra → my-openai/gpt-5.6-sol（k3 回首位） | 用户配置指令 |
