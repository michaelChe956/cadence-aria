# Proposal

> 权威来源：owner 已批准的决策报告
> `cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §5（决策五：WIC 命名域隔离 + 指纹代际）。
> 本 proposal 只记录 Why / 范围 / 非目标 / 受影响 capability；架构边界在 `design.md`，验收条款在 `specs/`，工作包在 `tasks.md`。

## Why

plan 域的验收标准 ID（当前统一写成 `AC-001`）与 spec 域的验收标准 ID（story/design 正文里的 `AC-*`，由 `[design_requirements]` 清单登记）共用同一字面命名空间：同一个字符串 `AC-001` 在两侧各指一个实体。引入新的 plan 命名域（`WIC-`）如果不同时处理「读」与「身份冻结」，存量 lineage 会在修订时被整体重编号，历史 finding 的身份随之漂移。

更关键的是指纹侧：finding identity 与 review scope digest 都建立在受限 ID 抽取之上（现抽取无 token 边界，会从 `WAC-001` 里抽出 `AC-001`）。若为了修边界而无版本地换算法，历史 `seen_fingerprints` / `original_fingerprints` 与新算出的指纹不再可比，旧 lineage 会把「同一个老问题」判成新 finding，并可能触发重复的自动返修。

因此本 change 的主题不是「加一个新前缀」，而是**命名域隔离 + 指纹代际隔离**：读的兼容、生的强制、身份的冻结、算法的按会话固定，四件事一起做。

## What Changes

- **双前缀只读兼容**：编译器 parser 仅在 criterion 分支接受 `WIC-` 与 `AC-` 两种前缀；`task_id` / `check_id` / work item ID 仍为单前缀；标识符词法（非空 ASCII 数字）不放宽；全 plan criterion 唯一性检查不变（两前缀共享同一次去重）。
- **spec 域 / plan 域职责分界**：spec 域 `REQ-*` / `NFR-*` / `AC-*` 只服务 Traceability 与 `requirement_refs`；plan 域 ID 只服务本 Work Item 的 criterion、`done_when_refs` 与 `reviewer_check_refs`。
- **服务端命名策略元数据**：新增 server-only 的命名策略（`WicV1` / `LegacyPreserved`），在真实 SC 首次生成准入时以 CAS 冻结；存量 lineage 冻结为 `LegacyPreserved`，未知历史不得伪装成新生；客户端不得以任何入参绕过。
- **存量保 ID、新生强制 WIC**：`LegacyPreserved` lineage 的修订保留基线既有 AC ID，新增 criterion 必须用 WIC；不提供「仅前缀升级」替换整组身份的操作，也不批量 AC→WIC，不改旧 immutable record 与其 hash。
- **指纹代际隔离**：新代际使用带 `WIC-` 的稳定 ID 前缀集合，抽取与删除走同一套 token 扫描（最长前缀先、token 前后不得紧连字母/数字/下划线）；原 v2 身份配方逐字保留，仅用于读冻结的旧历史。
- **代际随会话固定**：`RunHistory` 增可缺省的指纹代际字段（缺省为 v2，旧 v2 记录序列化时可省略）；新建受控会话显式写入新代际；review scope 增可缺省版本，v2 的 scope 摘要配方逐字不变，新代际 scope 绑定版本与 original 集合；版本不一致按既有结构性协议错误处理，不降级为人工门。
- **compiler version 本轮不 bump**：双前缀是兼容接受新字面形式，旧输入的 lower 结果与内容寻址不漂移；命名纪律由独立命名策略承担、算法代际由独立版本承担，都不借编译器版本表达。

## Non-Goals（本 change 明确不做）

- 不做存量 plan 域 `AC-*` 实体的批量数据迁移，不改旧 source / IR / report / 引用 / 指纹 / 审批记录（不可变记录不能原地改）。
- 不做全历史指纹重算，不移除旧算法实现（它是冻结历史的兼容读路径）。
- 不提供 WIC→AC 的自动反写，不做跨命名域的同号实体映射。
- 不改 `compiler_version` 语义，不借命名域改造夹带 compiler IR 语义版本升级。
- 不改动 legacy JSON outline / draft / batch 流程的命名域。

## Capabilities

### New Capabilities

- 无。本 change 不引入新能力路径，全部落在既有 capability 的 delta 上。

### Modified Capabilities

- `work-item-plan-single-candidate`：**ADDED** `REQ-WSC-30`（plan 域 criterion 命名域与双前缀只读兼容、spec/plan 域职责分界）、**ADDED** `REQ-WSC-31`（服务端 lineage 命名策略、存量保 ID、新生强制 WIC、旧实体 ID 保持）。
- `work-item-typed-outcome-policy`：**MODIFIED** `REQ-TOP-04`（指纹算法代际按会话固定、scope 版本绑定、新旧命名域不合并）、**MODIFIED** `REQ-TOP-05`（`RunHistory` 代际字段与缺省规则、接管/恢复不得改代际、缺证据不得清 history 或充值预算）。

> 需求编号说明：`REQ-WSC-26` / `REQ-WSC-27` 已被归档 change
> `2026-11-08-gate-card-resolved-and-inflight-state` 占用；`REQ-WSC-28` 归
> `bound-sc-generation-teaching`；`REQ-WSC-29` 归 `explicit-sc-author-effort`。
> 本 change 取 `REQ-WSC-30` / `REQ-WSC-31`，与其他 change 无重号。

## Impact

- **编译与解析面**：plan 编译器 parser 的 criterion 前缀分支与标识符词法；修复示例与教学材料中的 ID 纪律。
- **策略面（指纹/评审）**：稳定 ID 抽取与剥离的 token 扫描规则、finding identity 配方分代、`RunHistory` 与 review scope 的可缺省版本字段、scope 摘要配方分代。
- **持久化面**：工作项计划实体上的 server-only 命名策略元数据、SC 首次生成准入的 CAS 冻结点、会话创建与接管路径的代际写入。
- **守卫面**：新候选编译与提升边界上的命名域守卫（新模块），以及人工修订路径上的同一规则复用。
- **教学与 fixture**：SC author 教学材料与提示中的 ID 纪律、编译器 fixture 与 prompt 契约测试的部分样例（真实历史 AC fixture 保留为兼容用例，不全仓替换）。
- **兼容影响**：不 bump compiler version；旧 session、旧 lineage、旧 immutable record 与其 hash 全部保持原样可读可修订。
- **不在本 change 范围内**：其余 WIG 工作包（教学窗口、机械处置、评审闭合、effort、计量）由各自 change 承担；与 `mechanicalize-plan-compile-repair` 的共享文件归属见 `design.md` 的「共享面与排序」。
