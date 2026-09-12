# 测量报告 v2——3.6 全量收敛轮终版（2026-09-12）

> 基线：glm-5.3-flash 四 provider 统一弱模型（用户裁决 2026-09-07，推翻 3.5 可比性）；真实跑零伪造全入档。
> 数据源：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md`（逐跑台账）+ `convergence-36/` 各格证据。
> 前版：`measurement-report-v1.md`（2026-09-03）。

## 1. 矩阵终局（8/9 + 2 限制记录）

| 格 | 结果 | 跑量谱系 | 主暴露形态 |
|---|---|---|---|
| pi×轻 | ✅ 3/3 | 3（开关翻转后） | 兜底 commit 依赖已消（自行 commit） |
| pi×重 | ✅ 3/3 | ~12（双脚本口径后 3） | gate 竞态恢复链→修；契约网→双脚本收敛 |
| codex×轻 | ✅ 3/3 | 5 | reviewer verdict 缺失→硬化三件套后绝迹 |
| kimi×轻 | ✅ 3/3 | 5 | 不自行 commit→runner 兜底扛住；terminal cwd 绝对路径→修 |
| claude×轻 | ✅ 3/3 | 8 | 三层修复链（信任+断连+双 spawn drain）；前导语概率过 |
| claude×重 | ✅ 3/3 | 8（v15） | 误燃顶杀（产品→根修）；契约网/前导语概率过 |
| **kimi×重** | ✅ 3/3 | **37（17过/20悬）** | 契约网×13 纯方差；四次连胜 2 断；重跑收敛 |
| codex×重 | 📋 限制 | 0/10 | flash-via-codex 写不了计划（适配器逐字节等价实锤=模型行为） |
| F6 amendment | 📋 限制 | 4 | coder 判断层四逃逸形态（见 §2.2）；结构层已修 |

**C2 连跑纪律的成本分布**：轻格每格 3-8 跑；重格 pi 12/claude 8/kimi 37——kimi×重 的 37 跑是全矩阵最贵格，pure variance grinding（缺口游走实证无固定教学点）。

## 2. 弱模型基线发现全录

### 2.1 四 provider 行为差异（同 prompt 同语料同中继模型）

| 维度 | pi | codex | kimi | claude |
|---|---|---|---|---|
| 计划首稿质量（重语料） | 中（缺口可收敛） | **不可修**（逐字复制能力串都失败） | 中（37 跑 46% 过） | 中偏上（6-7 轮长链可收网） |
| 交付纪律 | 服从 commit 责任 | 部分服从 | **不自行 commit**（兜底扛） | 服从，偶发 scope 越界（门拦） |
| 结构化精度 | 路由判断错（DependencyGraphInvalid×Subgraph） | verdict 缺失（已治） | 枚举值大写/容器名（已治） | route×target 错配（已治结构） |
| 会话稳定性 | 偶发空输出 | resume stalled 高发 | 通道整段坏（外置因素）；90min 停滞×1 | 上游 idle/reset×2（不计族） |
| 握手延迟敏感度 | 低 | 中 | 低 | **高**（60-120s 慢握手放大一切竞态窗口——误燃顶杀只在 claude 格爆） |

**核心结论**：弱模型下 provider 适配器等价时，剩余差异全部来自 CLI 会话层行为（握手速度/交付纪律/通道稳定性），而非模型能力——除 codex 写计划能力为真例外。

### 2.2 结构化输出角落谱系（时间序，全部真实跑）

1. plan_defect 容器字段名当 kind 值（pi×重 v2）→ 契约 8 变体逐字枚举（d17fcb85）✅绝迹
2. reviewer 终局 JSON verdict 缺失（codex×轻）→ retry 诊断教学（7fccccc2）✅绝迹
3. InvalidRepairTarget 族 ×7（kimi×3/claude×1/pi×2+路由分歧）：route×target 矩阵错配+大写枚举值 → 矩阵逐字枚举+反例（c106a308）✅结构层绝迹（v16 两跑零违规）
4. **判断层残余**：coder 面对「需求依赖外部文件」的逃逸形态——建文件吸收/弱化测试吸收/幻觉 ops 阻塞（「文件系统只读」实测 rw）——不选 plan_repair。**结构可教，判断不可教（当前 prompt 家族内）**。
5. 前导语/unknown_structured_key/missing_section/invalid_ears：四 provider 共有，教学重驱恰一次后概率犯，重跑收敛（claude 轻格 5过1挂、kimi 语法族 4 例）

### 2.3 兜底体系有效性（产品侧）

| 机制 | 实战验证 |
|---|---|
| runner 兜底 commit（18a85d94） | kimi 全程扛住（provider 无关链路） |
| 分诊门 fail-closed（blocked_gate） | 全矩阵零错误放行；7 例 InvalidRepairTarget+幻觉全正确拦 |
| 契约前移修订循环（e929aa93） | 机械 findings 回灌+指纹闸，pi/claude/kimi 重格多轮收敛的主引擎 |
| 断连不杀 run（v11+） | rep1d run-2 存活实证；孤儿自灭 |
| 双 spawn drain（104662b0）+误燃守卫（23cdeb55） | claude×重 收官直接背书 |
| 教学重驱恰一次（ac0fe107） | 结构错偶发复发但显著降低整跑失败率 |
| 60min/90min 预算门 | rep20 8 轮不收敛正确截停；kimi coding2 90min 停滞正确截停 |

### 2.4 通道/环境观察（不修，记录）

- kimi 通道整段故障（09-11）：空输出×2+CLI 直测零回复，用户侧调整后恢复——**矩阵跑需前置通道健康探针**（建议 3.7 driver 加 CLI ping）
- 上游波动族（idle/reset/空输出）全矩阵 5 例，SOP 不计缺陷直接重跑
- 机器重启清 /tmp：部署二进制与日志随 /tmp 丢失→本报告部署谱系建议产物化（md5 清单已在 deployment-verification.md）
- claude 慢握手是竞态放大器：任何新引擎代码必须假设 60-120s 握手窗口

## 3. 修复环全景（25 commit：v13 前 22 + 本会话 3）

v13 前 22 环见交接文档 §4。本会话新增（全部 TDD+k3 Approved）：
1. **23cdeb55** SC 误燃顶杀根修（oracle 候选6：followups 循环 SC 委托守卫；claude×重 0/3→3/3 直接背书）
2. **6c236d59** F6 语料强化（REQ-003 三重栅栏，缺陷浮出 0/2→2/2 可靠）
3. **c106a308** route×target 矩阵教学（结构层 InvalidRepairTarget 绝迹）

部署谱系：v13(de12b197)→v14(0cf3aad0 重建)→v15(92c6ee9b)→**v16(4d0cd0be 现行)**。

## 4. 95-baseline 缺陷族终态

见 `convergence-36/README.md` §3（族 3/4 大幅收窄已治理，族 1/2 通道与前导语为残余，新族 5 个全录）。

## 5. Feature backlog（3.7 输入）

1. **产物自动提交可配置**（用户指定后续功能：aria 感知目标仓开关/管道级配置/冲突仲裁；连带消化 sc_advance 预置 head_commit/staged-diff 诚实性/manual_continue 失真）
2. **route×target 判断层**：结构教学已尽，判断选择需换路径（few-shot 对照案例/更强基线/产品侧引导式路由）——待「攒 1-2 个正确对照案例」（用户裁决 codex 处置同款指引）
3. 4 号修法包四件（compile 失败门/active_node 对齐+manual repair 入口/stale approval 三元组/REQ-CG-04 补句/rework 同构教学）
4. run 所有权 registry 收敛（provider_drive_in_progress 两守卫先行）
5. driver 侧：stage3_group_snapshot elapsedMs JS bug/CLI 通道健康探针/矩阵长跑 bash 超时教训（timeout:0 兜底）
6. reviewer 预算若三度触顶转 history 压缩（已两度 64→96KiB）

## 6. 结论

- 8/9 格在弱模型基线下达成 C2（含两格靠长尾重跑收敛），1 格模型能力限制+1 格判断限制——**产品侧全链（门/兜底/恢复/预算）在弱模型压力测试下零未拦事故**
- 弱模型工程结论：结构可教（三轮教学全部生效）、判断不可教、竞态必须按最慢 provider 设防、通道健康是矩阵前置条件
- 3.7 立项输入：上述 backlog + UI 层（本报告范围外）
