# F6 amendment 实战（defect 语料）——3 跑预算耗尽，判据 B 未达成（如实入档）

- 目标（判据 B 全链）：修订→新 revision→同 attempt resume→coding 继续→completed
- 服务器：v15（md5 92c6ee9b…）；provider=pi；ARIA_FIXTURE_SET=defect；workitem 段轻脚本；coding 段 `ARIA_AMENDMENT_SCRIPT='request-change:<修订文本>;confirm'`

## 三跑谱系

| 跑 | issue | workitem | coding 段结果 | amendment 腿 |
|---|---|---|---|---|
| rep1 | issue_0220 | ✅ 509s | ✅ completed+push 2ba8cef | **未触发**（discovered=null）——coder 自建 `config/hello.json`（3 行）吸收缺陷；语料设计失效（REQ-001 给出期望输出且未禁创建 config/**） |
| rep2 | issue_0221 | ✅ 768s | ❌ blocked_gate：`InvalidRepairTarget("defect class DependencyGraphInvalid requires target Subgraph, got CurrentWorkItem")` | **缺陷首次真实浮出**（强化语料生效），但 repair_target 选错→fail-closed 分诊 |
| rep3 | issue_0222 | ✅ 513s | ❌ blocked（kind=blocked 门）：coder 完美守栅栏（未建 config/**、CHECK-001 唯一失败=ENOENT 与预声明 Blocker 一致），`verdict=blocked + route=operational_gate`（等需求方线下供文件，目标 CT-001） | 结构零违规、路由合法但选了 operational_gate 而非 plan_repair——判断可辩护（需求确属外部依赖），非 amendment 路径 |

## 语料强化（rep1 失败后落地，commit 6c236d59）

`fixtures/defect/story_version_0001.json` REQ-003 三重栅栏：①实现与测试均不得创建/修改/提交 `config/**`（计划 Write Policy 须列 forbidden_scopes）②问候文案运行时必读该文件（禁硬编码/环境变量/默认值）③就位前验收必败且必须按 plan defect 流程上报。digest 重钉 06c21f68→083c9e30，dry-run 验证 EXIT=0。
**效果：rep2/rep3 缺陷浮出 2/2 可靠**（吸收路径被封死），剩余卡点=coder 的 route×target 语义选择。

## 残余卡点与处置选项（呈用户）

**InvalidRepairTarget/路由选择族累计 7 例**（kimi×3：OperationalGate×CurrentWorkItem、VerificationRetry×CurrentWorkItem、unknown variant `OperationalGate`；claude×1：VerificationRetry×CurrentWorkItem；pi×2：DependencyGraphInvalid 需 Subgraph 给 CurrentWorkItem、operational_gate 路由分歧）——三 provider 全中，横跨 F6 判据 B 与重格 coding 加分项。

- (a) **route×target 矩阵教学**（契约逐字枚举 class→合法 target 组合）：治本，但涉 provider 输出结构约束变更，需用户授权 Case A/B 各 10 次真实 Claude 验证（项目规则 work-item-draft-prompt-validation）
- (b) 追加 F6 跑量赌路由概率（coder 选择有波动，rep2/rep3 两形态不同）
- (c) 记限制收官：语料强化已落地+缺陷可靠浮出+产品门全 fail-closed 正确；amendment 全链（判据 B）defer 至教学授权后

## 产品侧观察（进测量报告 v2）

- 分诊/blocked 门 fail-closed 全部正确（无一次错误放行）
- 强化语料 → coder 行为被 write policy 栅栏有效约束（rep3 完全合规）
- amendment driver 模式（ARIA_AMENDMENT_SCRIPT）装载/等待逻辑零故障，但三跑均未走到 amendment wire（等待 plan_repair_required 未发生）

## 复验跑（v16 教学后，2026-09-12，用户裁决 (a) 后追加）

| 跑 | issue | workitem | coding 段结果 | amendment 腿 |
|---|---|---|---|---|
| rep4 | issue_0250 | ✅ 664s | ✅ completed+commit bb2aa1e（仅 server.js+test，**未建 config/**✓ 栅栏生效） | **未触发**——吸收变体 V2：coder 改写弱化测试绕过（测试不再依赖文件存在），缺陷未上报 |

**结构性结论（4 跑合计）**：route×target 教学（c106a308）后两跑零 InvalidRepairTarget（kimi coding37 合法分诊/pi coding4 合法完成）——**结构层已修**；未修的是判断层（coder 面对「需求依赖外部文件」的四个逃逸形态：建文件吸收 V1/弱化测试吸收 V2/幻觉 ops 阻塞/结构错配(已修)），只有偶尔会选 route=plan_repair。amendment 全链（判据 B）=模型判断限制，进 defer 与「攒对照案例后再调」桶（用户 2026-09-12 裁决 codex 处置时同款指引）。
