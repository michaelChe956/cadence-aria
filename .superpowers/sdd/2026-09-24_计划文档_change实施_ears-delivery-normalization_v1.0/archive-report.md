# 归档报告：9 个已完成 change 批量归档（2026-09-26）

按实施时间线逐个执行 openspec-archive-change 流程：status 核验（isComplete+planningComplete 全过）→ tasks.md 勾选核 → delta spec 同步至主 specs（agent-driven 逐字合并）→ 逐 capability verbatim 复核 → `mv` 至 `openspec/changes/archive/2026-09-26-<name>`。

## 归档清单与 spec 同步映射

### 1. single-candidate-lowering-safety（C0）→ `archive/2026-09-26-single-candidate-lowering-safety`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `work-item-plan-single-candidate` MODIFIED REQ-WSC-02 | `specs/work-item-plan-single-candidate/spec.md` REQ-WSC-02 | 归一化段扩为两类（新增 EARS 关键字空白归一化）；新增「lowering 重复字段累积语义」段；scenario 3→10（新增 EARS 缺空格补齐、归一化不救非空白语法错误、重复能力行累积、重复需求行同构累积、相邻 contract 严格隔离、累积不改写值形态、重新编译 IR 差异仅源于重复字段语义） |

### 2. ears-delivery-normalization → `archive/2026-09-26-ears-delivery-normalization`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `work-item-plan-single-candidate` ADDED REQ-WSC-09 | 同上 spec 末尾新增 | generate 段编译语法失败的教学重驱（恰一次，invalid_ears/missing_section/invalid_work_item_id 可教学类） |
| `work-item-plan-single-candidate` MODIFIED REQ-WSC-02 | 同上 spec REQ-WSC-02 | 归一化段措辞精化（加粗 **EARS 关键字空白归一化**、「（含 CJK 标点如全角引号）」、「SHALL 照旧交给编译器以 fail-closed 拒绝」）；「固定词表」「EARS 缺空格」两 scenario 补全角引号/中文标题示例与强化 THEN。**叠加保留** C0 的 lowering 段与 6 个 lowering scenario（ears delta 未触及，语义无冲突） |

### 3. node-detail-hydration-resilience → `archive/2026-09-26-node-detail-hydration-resilience`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `node-detail-hydration-resilience` ADDED（新 capability） | 新建 `specs/node-detail-hydration-resilience/spec.md` | REQ-NDR-01 快照不清空已水合 detail / REQ-NDR-02 水合集合覆盖全部终态节点 / REQ-NDR-03 未水合 token 位占位 / REQ-NDR-04 pi usage 兜底读取加固 / REQ-NDR-05 usage 失败可观测；Purpose 逐字迁入 |

### 4. driver-lease-self-healing → `archive/2026-09-26-driver-lease-self-healing`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `driver-lease-self-healing` ADDED（新 capability） | 新建 `specs/driver-lease-self-healing/spec.md` | REQ-DLS-01 写时自愈 / REQ-DLS-02 前端单次自动重试 / REQ-DLS-03 租约转移可观测 / REQ-DLS-04 attach 对租约零效应（含 cockpit-fullcourse REQ-WCR-02 交叉引用） |

### 5. single-candidate-contract-preflight-and-loop-identity（C1）→ `archive/2026-09-26-single-candidate-contract-preflight-and-loop-identity`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `work-item-plan-single-candidate` MODIFIED REQ-WSC-02 | `specs/work-item-plan-single-candidate/spec.md` REQ-WSC-02 | 新增「候选校验携带存储 options 与基线树」段 + 3 个新 scenario（options 缺口首轮即拦 / options 三族同报 / AC 引用基线外路径被拦）；既有归一化/lowering 正文保留（C1 为压缩复述，无行为变更，按信息最全版保留） |
| 同上 MODIFIED REQ-WSC-03 | 同上 spec REQ-WSC-03 | 正文追加「机械校验 SHALL 覆盖 options×items 一致性与 AC 路径×基线树核对（REQ-WSC-02）；此类缺口 SHALL NOT 延迟至 Approval 阶段才首次出现」 |
| 同上 MODIFIED REQ-WSC-06 | 同上 spec REQ-WSC-06 | 新增 author options 镜像/AC 基线教学段、reviewer 前轮结构化 findings 注入段 + 3 个新 scenario（options 教学与校验口径一致 / AC 基线教学生效 / 复评注入前轮清单） |
| `work-item-typed-outcome-policy` MODIFIED REQ-TOP-04 | `specs/work-item-typed-outcome-policy/spec.md` REQ-TOP-04 | 整块替换：cycle 锚扩展单候选 `sc:candidate:<source_revision_hash>`、单候选返修后 Verification scope、Finding identity 改结构化 canonical key（unstable fail-safe）；scenario 2→7（新增同题异措辞稳定判重 / 异题不撞指纹 / unstable 身份 fail-safe / 单候选返修后进入 Verification / 单候选 cycle key 锚定候选） |

### 6. human-gate-convergence（C2）→ `archive/2026-09-26-human-gate-convergence`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `human-gate-convergence` ADDED（新 capability） | 新建 `specs/human-gate-convergence/spec.md` | REQ-HGC-01 门预算权威与真实递减 / REQ-HGC-02 收敛投影与门区分（批次门 vs 修订门）/ REQ-HGC-03 有效分类透明与解析韧性 |
| `work-item-plan-conversational-gate` MODIFIED REQ-CG-02 | `specs/work-item-plan-conversational-gate/spec.md` REQ-CG-02 | 整块替换：预算重置边界反转为「新 logical gate 才取默认、同 gate 内 carry-forward」（显式修订原「重置为默认值」条款）；新增 accepted_feedback_turns 段；scenario 7→9（新增 同 logical gate 修订不重置预算 / accepted 计数不污染 policy 计数） |

### 7. human-gate-termination-reliability（C3）→ `archive/2026-09-26-human-gate-termination-reliability`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `human-gate-termination-reliability` ADDED（新 capability） | 新建 `specs/human-gate-termination-reliability/spec.md` | REQ-HTR-01 abort 语义与回执诚实 / REQ-HTR-02 门态动作呈现分层 / REQ-HTR-03 degraded 连接关键帧投递 |

注：tasks.md 5 项归档时原为未勾状态；按本会话实施事实（已部署+用户冒烟通过）全量勾选后归档。

### 8. per-issue-base-branch（PIB）→ `archive/2026-09-26-per-issue-base-branch`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `per-issue-base-branch` ADDED（新 capability） | 新建 `specs/per-issue-base-branch/spec.md` | REQ-PIB-01 基准分支设置与锁定 / REQ-PIB-02 author 上下文基线限定（host-served 硬边界 vs provider 原生软限制）/ REQ-PIB-03 coding fork 与核对同源 |

### 9. artifact-candidate-selection（ACS，最后归档）→ `archive/2026-09-26-artifact-candidate-selection`

| Delta | 主 spec 落点 | 说明 |
|---|---|---|
| `artifact-candidate-selection` ADDED（新 capability，含 v1.1 delta 注记） | 新建 `specs/artifact-candidate-selection/spec.md` | REQ-ACS-01 顶层候选枚举与唯一选择 / REQ-ACS-02 候选选择诊断落盘 / REQ-ACS-03 弱模型输出负面清单教学（含 F-60 P0 `StreamingProviderInput` 出口统一装配升级句——v1.1 注记已并入正文） |
| `story-pipeline-weak-model-hardening` MODIFIED「Story author artifact 一次成功（SHALL）」 | `specs/story-pipeline-weak-model-hardening/spec.md` | 「一次成功」定义改为唯一 gate-passing 候选（锚定 REQ-ACS-01）；scenario 1→3（新增 前置示意 block 不破坏一次成功 / 多个有效候选不算一次成功） |

## 合并原则说明

- 同一 requirement 被多个 change 修改时按实施时间线叠加：后 delta 的行为变更（如 C2 的预算 carry-forward 反转、C1 的 REQ-TOP-04 重写）以 delta 为准整块替换；delta 仅对前序内容做压缩复述而无行为差异时（如 C1 对 WSC-02 归一化/lowering 段、对 3 个 scenario 的示例删减），保留信息最全的前序版本，新增段与 scenario 逐字合入。
- 新 capability 主 spec 采用主格式（无 Delta 头），Purpose 逐字迁入，ADDED requirements 原样落位。

## 验证结果

- 逐 change 复核：每个 delta 的全部 ADDED/MODIFIED 正文与 scenario 以脚本 verbatim 校验存在于主 spec（除上述声明的压缩复述差异）。
- `openspec validate --specs`（worktree `.worktrees/feat-b-0808-add-monorepo`）：**53 passed, 0 failed**（合并前 47 项 → 合并后 53 项，新增 6 个 capability）。
- 剩余 9 个 active change（cockpit-fullcourse-connection-resilience 等）未动。
