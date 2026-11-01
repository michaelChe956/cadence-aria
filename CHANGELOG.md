# 更新日志

本文件记录 Aria 各版本的更新内容。版本号遵循语义化版本（SemVer）：
主版本 = 不兼容变更，次版本 = 新增能力，修订号 = 缺陷修复。

---

## 0.1.0 — 2026-11-01

**主题：逻辑代码库（LC）多仓全链路的判定一致性、失败语义与可观测**

本次是一个大版本：多仓（逻辑代码库）从**聚合初始化**到**Work Item 落地**的整条链路被重新对齐了一遍——
同一份候选在链路上被**两套图**判定、失败时给不出可执行的下一步、跑完也**看不到通过率**。
本版把这三件事一次性收口。

### 一、LC 多仓全链路（本次主线）

一个多仓 issue 从授权到落地的完整流程如下（本版逐段修正）：

| 段 | 做什么 | 本版的变化 |
|---|---|---|
**① 自动化授权**（enrollment） | 授权哪个 LC、哪些成员仓、用哪个 author/reviewer、预算与审批策略 | 授权 payload **不可原地改写**（避免悄悄换 provider/换成员）；重开只能「终止 → 重开」换代 |
**② 聚合初始化**（冷启动 5 步） | `MachineSkills` → `AggregatePreflight` → `PreCheck` → `RuleAndMcpConfig` → `OpenspecAndExamples` | `AggregatePreflight` 的聚合根唯一性判定**带上自己的 LC 身份**——此前会把「自己」当「他人占用」拦掉，导致 LC 冷启动直接失败 |
**③ 计划准备**（Prepare） | 建 plan + 会话，锁定 source（story/design）与成员仓 | 候选 ref 提升与零提升语义保持不变（被拒候选绝不污染 current refs） |
**④ 作者生成**（Generate） | author 按 story/design + 成员仓上下文产出 Work Item Plan markdown | 结构门拒收改为 **typed 失败路**（此前直接硬失败）；作者阶段新增**恰一次教学重驱**（见下） |
**⑤ 候选结构门**（SG3a.1） | 把候选投影成**有效图**（canonical Inputs ∪ change_order 自动边），判定是否有环、归属是否自洽 | **候选结构评估与 Final Compile 现在必须得出同一结论**（此前一个说「无环」、一个说「成环」，owner 只能看到「写 plan 成功但 review 后报依赖环」） |
**⑥ IR 校验** | 引用与字段是否符合契约（unknown_requirement_ref、acceptance_criterion_without_reviewer_check 等） | 校验失败走**教学重驱**（把错误原文与修正指令带回作者） |
**⑦ 评审**（Evaluate） | reviewer 产出 verdict | 评审与人工门之间形成**返修闭环**：机械可修的由平台自动触发作者返修（`TriggerAggregateRepair`），改完复评 |
**⑧ 人工门**（HumanConfirm） | owner 确认 / 反馈 / 裁决 | 卡面改为**给选项而不是给断言**：依赖环不再归因「design 有问题」，而是给出环上真实链、各 Work Item 的仓序位次、逆仓序那一跳，并给 **2 个各带后果的合法修法** + 保护项 |
**⑨ Final Compile** | 归属核对、跨仓依赖落地、创建 Work Item | 与⑤共用同一套归属反查（**物理成员仓 → 逻辑成员**唯一实现，多命中绝不猜） |
**⑩ Work Item 落地** | 建 Work Item + 子 session，进入编码 | — |

### 二、失败语义：从「生成失败」到「可执行的下一步」

- **依赖环不再甩锅 design**：旧文案「根因可能在 design，需 owner 决定是否修改 design」在本类失败里是错的。
  新文案只陈述事实：环上链、各 Work Item 所在仓的 change_order 位次、逆仓序那一跳；
  并给 2 个合法修法（去掉该跨仓消费 / 把该 Work Item 连同写范围与测试迁到更后的仓），各带后果；
  附保护项：**不得为消环删除真实业务数据流、不得改动仓序本身**。
- **结构门拒收不再硬失败**：此前会话直接进 `failed`，owner 只看到「生成失败」，
  既没有修法、也没有任何尝试记录。现在走 typed 失败路：durable 诊断 + 尝试流水 + 人工门。
- **模型有机会自己改对**：作者阶段结构门拒收给**恰一次教学重驱**——把环的事实与两个修法带回作者，
  模型改对即通过（owner 零点击）；与 compile/IR 教学重驱**分槽**，互不挤占；
  每个候选的额外 provider 驱动次数**有界**。
- **过期门卡不再误导**：计划已确认完成后，对话流里残留的旧门卡不再显示「只能终止此门后重新发起」，
  改为「这一版已确认完成，此卡是历史记录，无需处理」。

### 三、通过率可观测（新增）

- **尝试流水**（append-only，`candidate-attempts.jsonl`）：每次候选生成/编译尝试一行（成功与失败都记），
  落 **issue 级**目录 → 耐得住「终止自动化 + 重开」造成的会话目录清理（此前历史会被擦掉，读数从 1.0 掉回 0.0）。
- **通过率读数**（只读 HTTP）：
  - `GET /api/projects/{p}/issues/{i}/candidate-attempt-metrics` —— 该 issue 的尝试序列 + 读数
  - `GET /api/projects/{p}/candidate-attempt-metrics` —— 跨 issue 聚合（`issue_count = N` 时才是真率）+ per-issue 明细
  - 读数含：首次通过率、同 issue 连续失败次数 P50/P90、失败码 Top N、lowering 类终态失败计数。

### 四、缺陷修复

- **pi ask 扩展并发读半文件**（真缺陷，多会话并发起 pi 会踩）：
  原实现先建空文件再写内容，并发调用者会读到写了一半的文件并直接失败；
  改为**同目录临时文件 + rename 原子发布**（含并发回归用例，红态已验证）。
- **macOS 编译修复**：`libc::mode_t` 的宽度随平台而变（Linux `u32` / macOS `u16`），
  本仓按 Linux 宽度假设 → darwin-arm64 编译失败；现统一收敛到 `u32` 后再比较（Linux 上等价）。
- **依赖更新**：`async-trait` 0.1.89 → 0.1.92（上游修复了新工具链下触发的 lint）。

### 五、构建与 CI

- 对齐 Rust 1.99 的新 lint（`double_must_use` / `fetch_update` 弃用 / `single_element_loop`）。
- 真机 e2e 用例（pi / codex /  unix 进程组）在 CI 环境**显式跳过并打印**（CI runner 无交互式 CLI，
  本地照常真跑，不丢本地覆盖）。
- 故障注入由**进程级单槽**改为**按注入根分槽**（并行测试互相覆盖 → durable-write-failure 一族偶发的共因）。
- WS 集成用例的时序假设修正（空闲关闭的 RST 收尾、挂起 choice 投影的读竞争）。
- 证据检索用例依赖真实 `codegraph` CLI：CI 安装（钉版本）+ 缺工具时显式跳过。

### 已知未修

见 `cadence/notes/2026-11-01_待办_平台已知未修问题清单_v1.0.md`（12 条，含 2 条 P0：
自动化授权换代的 revision 竞态、旧 plan 在 CAS 之前被删）。

---

## 0.0.10 — 2026-11-01

- 多仓 work item group 全链路（判定一致 + 结构门 typed 失败 + 作者阶段教学重驱 + 通过率读数）
- 过期门卡在计划已确认完成时不再误导
- 真缺陷修复：pi ask 扩展并发读半文件 → 原子发布
- macOS 编译修复（`libc::mode_t` 宽度随平台而变）
- CI 修绿：Rust 1.99 新 lint、真机 e2e 环境门、聚合预检 rescope 本 LC、故障注入按根分槽、
  WS 时序偶发、证据检索用例的 codegraph 依赖

## 0.0.9 — 2026-08-26

- 图片创作文件存储改造 + kimi 迭代修复 + 更新动态滚动窗口

## 0.0.8 — 2026-08-17

## 0.0.7 — 2026-08-13

## 0.0.6 — 2026-08-08

## 0.0.5 — 2026-08-04