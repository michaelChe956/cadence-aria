# spec delta 同源落地：REQ-BOOT-08 与 REQ-LAYOUT-01/02（含两份 tasks 同步）

- 日期：2026-11-03；版本：v1.0。
- 状态：**DONE_WITH_CONCERNS** —— 四个文件已按锁定文本逐字落地；非 strict 校验绿；`--strict` 被 OpenSpec 的「ADDED requirement 正文 >500 字符」体例告警阻断，该冲突由锁定文本本身带来，需 controller 裁定（§5-A）。
- 接地 worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`（已 `cd` 后 `pwd` 确认）。
- 依据：只读 `cadence/notes/2026-11-03_专项_布局文案同源与跨块顺序_v1.0.md` §2.2 / §2.3 / §6.1 表末两行。
- 纪律：只改四个文件；未改 Plan、技术方案、源码、`openspec/specs/`；未 `git add`；未改任何设计结论（§2.2/§2.3 文字逐字照抄）。

## 0. 结论摘要

| 项 | 结果 |
|---|---|
| 四文件按锁定文本落地 | ✅ 完成 |
| `openspec validate fix-aggregate-cold-start-and-audit-budget`（非 strict） | ✅ `is valid`，exit 0 |
| `openspec validate remove-single-repository-path`（非 strict） | ✅ `is valid`，exit 0 |
| `... --strict`（块2） | ❌ exit 1：1 条 >500 字符 WARNING |
| `... --strict`（块3） | ❌ exit 1：2 条 >500 字符 WARNING |
| 七码列表一致性 | ✅ 三处列表逐字节相同（§3.1） |
| FailureCopy 五字段名/类型一致性 | ✅（§3.2） |
| 块3 组7 无「自建文案表/布局端点」 | ✅（§4） |
| 主 specs 同名 requirement 冲突 | ✅ 无（§3.4） |

## 1. 逐文件改动（行号 → 改前 → 改后要点）

### 1.1 `openspec/changes/fix-aggregate-cold-start-and-audit-budget/specs/non-interrupt-repository-bootstrap/spec.md`

该条仍在 `## ADDED Requirements`（:49 起）内，**修改该 ADDED 条目自身**，未新增同名 MODIFIED 条目。

| 行（改前 → 改后） | 改前 | 改后要点 |
|---|---|---|
| :109 → :109 | `### Requirement: 聚合初始化失败面的字段化与文案单一来源（REQ-BOOT-08）` | `### Requirement: 聚合初始化、聚合索引及登记布局失败面的字段化与文案单一来源（REQ-BOOT-08）` |
| :111 → :111-:117 | 首句「聚合初始化的失败面 SHALL 由**单一来源**产出，MUST NOT 在投影层 / DTO / 通知三处各拼一份。」+ 原「每条失败面 SHALL 至少包含…」 | 首句被 §2.2 四段替换，逐字：**P1**（:111，157 字，范围与同源：初始化/索引/登记布局失败面 + 只读事实适配 + 四侧禁止同义文案）；**P2**（:113，364 字，登记六条布局规则 + 七个**对外稳定码**逐字列表 + 内部码仅作反查）；**P3**（:115，349 字，`FailureCopy` 五字段必填非 Option + 码表每条三个中文字段 + 七码须给规则依据与处置动作 + 无 `action` 不构成例外 + DTO/消费链完整透传、`reason_zh` 不得改可选）；**P4**（:117，114 字，未知码中性兜底、不得编造、已知码不得被当未知码） |
| — → :119 | 原「每条失败面 SHALL 至少包含：**中文主文案**…、**可执行动作**…、**分诊标签**…与**机器码**…。机器码 MUST NOT 作为主文案呈现。」 | **原样保留**，按 §2.2「随后保留」置于四段之后（独立段落） |
| :113 → :121 | 脱敏/截断/诚实 `unknown` 段 | **未改**（§2.2 明确「不降低」） |
| :115-118 → :123-126 | WHEN「聚合初始化产生一条失败记录」；THEN 三段 + 机器码降级 | WHEN → 「聚合初始化、聚合索引或上述登记布局检查产生失败事实」；THEN 原要求保留并在句末追加「；布局失败同时呈现同源原因段」 |
| 新增 → :128 / :133 / :138 / :143 | — | 4 个新场景逐字取自 §2.2：`无 action 的布局失败同源`、`单点修改覆盖两出口`、`字段完整且跨链同构`、`未知码中性兜底不伪装布局` |
| 保留 → :153 / :158 / :163 / :168 / :173 | — | `原始输出被截断时如实标注`、`凭据与身份路径被脱敏`、`最早期的失败记录也有兜底文案`、`成因不可判定时诚实标注`、`确定性成因被正确归因` 全部原样保留 |
| 顺延 | REQ-BOOT-09 原 :150 | 现 :178（场景随之顺延，内容零改动） |

REQ-BOOT-08 场景数：**7 → 11**。

### 1.2 `openspec/changes/remove-single-repository-path/specs/logical-codebase-layout-guidance/spec.md`

| 行 | 改前 | 改后要点 |
|---|---|---|
| :13（未移行） | 「规则表每一项 MUST 绑定其稳定错误码与一句「用户该怎么摆」。任何布局错误码 MUST 能映射到表中一行；系统 MUST NOT 呈现只有错误码、没有动作说明的布局失败。」 | 整句替换为 §2.3 的 REQ-LAYOUT-01 文本（358 字）：六规则与稳定码映射 MUST 登记在 `REQ-BOOT-08` 定义的唯一失败面文案来源；本能力 MUST NOT 自建第二份规则表或文案表；每项绑定稳定码并引用非空 `title_zh`/`reason_zh`/`action_zh` + `triage`/`machine_line` 五字段合同；**reason_zh MUST 为必填 String 而非 Option**；内部码反查对外稳定码；不得呈现只有码/无动作/缺原因段的布局失败 |
| :54（未移行） | 末句「该文案 MUST 与失败面文案的单一来源同源（`non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08`），MUST NOT 另抄一份。」 | 末句替换为 §2.3 的 REQ-LAYOUT-02 文本（358 字）：**取自** `REQ-BOOT-08` 登记的规则表 + 三段分别由必填非空 `title_zh: String`/`reason_zh: String`/`action_zh: String` 承载并消费同源 `triage`/`machine_line` + 布局引导侧只渲染成品、不自补中文模板 + 七码即使无 `action` 仍须完整三段 + **缺 `reason_zh` 或缺 copy 不得作为正常缺省降级验收通过**。该段首句「失败面 SHALL 按错误码呈现「问题 / 原因 / 怎么做」三段…次要行。」原样保留（未重复），故整段 = §2.3 blockquote 逐字 |
| :75（未移行） | THEN「该修改 SHALL 落在失败面文案的单一来源（`REQ-BOOT-08`）里；MUST NOT 在布局引导侧维护第二份同义文案」 | 替换为 §2.3 的「文案单一来源」THEN：落在 `REQ-BOOT-08` 登记的六规则/七个对外码唯一来源 + 两条消费链直接获得修改后的 `title_zh`/`reason_zh`/`action_zh` + 不得维护第二份同义文案 + 不得因原因字段缺失静默省略原因段 |
| :11 | 「系统 SHALL 维护单一布局规则表，覆盖六条规则：(1)…(6)…」 | **未改**。§2.3 只点名 `:13`；保留六规则枚举＝本能力对布局判定的规范陈述（"系统"层单表即 `REQ-BOOT-08` 来源，与「本能力 MUST NOT 自建第二份」不冲突）。是否删除见 §5-B |
| :77-:96（REQ-LAYOUT-03） | — | **零改动**（codegraph 文案继续引用块2 条目、安装继续用块2 机制），符合「不改义」 |

### 1.3 `openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md` 组 6（原 :63-:67 → 现 :63-:70，5 条 → 8 条）

| 编号 | 改前 | 改后要点 |
|---|---|---|
| 6.1 | `FailureCopy { title_zh, triage, action_zh, machine_line }`（4 字段）+ `describe(&AggregateInitializationErrorRecord)` | `failure_copy.rs` **一次写全**六个布局规则 + 七个对外稳定码（逐字列表）+ 内部反查码；`FailureCopy { title_zh: String, reason_zh: String, action_zh: String, triage: FailureTriage, machine_line: String }` 五字段全部必填、非 Option、非空；`describe(FailureInput) -> FailureCopy`（纯函数）；**未知码中性三段兜底**；**块3 MUST NOT 追加写本文件** |
| 6.2 | 仅「`:334-347` 与 `:575-589` 共用同一函数渲染 detail（禁止第二份拼接）」 | 追加 **链A**：经 `BootstrapFailure.copy` 下发同源成品（初始化两处 + 索引失败调用点；无 `action` 的布局事实仍命中已知模板，不降级为通用重试） |
| 6.3 | 仅 `#[serde(default)] pub triage: Option<String>;` | 保留 triage（值只由 `copy.triage` 序列化派生，不另查表）+ 新增 `#[serde(default)] pub copy: Option<FailureCopy>;`；新产生的已知失败（尤其七码）必须 `Some(copy)` |
| 6.4 | 原为「前端 `LogicalCodebaseBootstrapCard.tsx` 主行渲染…」 | 改为 **链B**：`AggregateRootPreflightError::copy()` 只做输入适配并委派同一 `describe`（不自持表）+ `product_store_error.inc.rs:9-31` 的 `aggregate_root_api_error` 保留既有 code/message/details.reason 并追加 `error.copy()` 到 `details.copy` |
| 6.5（新增） | — | **Rust DTO 逐字段搬运**：`src/web/types/logical_codebase_bootstrap.rs` 新增 `FailureCopyDto` + `BootstrapFailureDto.copy` + `From<&Projection>`（product 加字段不会自动透出，必须手工镜像五字段）—— 即专项 §1.3 第 4 点的 B2-Plan `:410-437` 漏项 |
| 6.6（新增） | — | **TS 同构**：`web/src/api/types/logical-codebase-bootstrap.ts` 增 `FailureCopyDto` 与 `BootstrapFailureDto.copy`；五字段必填，`reason_zh` MUST NOT 改成可选。块2 tasks 改前**完全没有**该行（已 grep 确认），故按 brief ④ 补 |
| 6.7 | 原 6.4 | 前端卡片主行渲染三段成品与 `failure.detail`，`原因 {reason_code}` 降级折叠区，`triage` 渲染中文标签 |
| 6.8 | 原 6.5 | 验收：`no stderr summary` → 0；早期记录兜底；**七码逐条命中具体规则依据与可执行动作**；**Rust DTO / `details.copy` / TS 五字段同构，缺 `reason_zh` 或缺 copy 不算通过** |

编号说明：6.3 有意保持「triage」任务编号不变 —— 既有材料引用 `tasks.md:65（6.3）`（专项 §1.3 第 3 点、块2 侧改造依据），改名会断引用。

### 1.4 `openspec/changes/remove-single-repository-path/tasks.md` 组 7（:93-:97，条数不变）

| 编号 | 改前 | 改后要点 |
|---|---|---|
| 7.1 | 「**落规则表**：六条规则 → 稳定错误码 → 「用户该怎么摆」，作为实现与文案的共同来源」 | 「**引用**块2 `failure_copy.rs` 的共享规则与文案结果（六条规则 → 七个对外稳定码 → 「用户该怎么摆」）作为本能力实现与验收的共同来源；**MUST NOT 在块3 自建规则表或文案表**」 |
| 7.2 | — | 原样保留 |
| 7.3 | 「**六个错误码**（`aggregate_root_is_git` / `member_path_outside_root` / `aggregate_index_layout_unsupported` / `member_symlink_escape` / `aggregate_root_overlap` / `nested_worktree` / `aggregate_index_member_coverage_failed`）…」（标注六、实际 7 项且混用内部码） | 「**七个对外稳定码**（七码逐字列表）…；内部码 `member_path_outside_root` / `member_symlink_escape` / `nested_worktree` 只作反查」 |
| 7.4 | 「布局动作文案接入失败面单一来源（块2 `REQ-BOOT-08`），**不另抄一份**；块2 未落地时先落映射，文案接线标注依赖」 | 「只渲染块2 `REQ-BOOT-08` 登记的 `failure_copy.rs` 成品（五字段必填），**不另抄一份**；本片**显式依赖块2 片2** 交付，块2 未落地时先落映射并标注依赖，MUST NOT 在块3 另建文案来源」 |
| 7.5 | — | 原样保留 |

## 2. validate 逐字输出

### 2.1 `--strict`（块2）—— exit 1

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' has issues
⚠ [WARNING] non-interrupt-repository-bootstrap/spec.md: ADDED "聚合初始化、聚合索引及登记布局失败面的字段化与文案单一来源（REQ-BOOT-08）": Requirement text is very long (>500 characters). Move examples and edge cases into scenarios, or split it into separate requirements that each state one behavior.
Next steps:
  - Ensure change has deltas in specs/: use headers ## ADDED/MODIFIED/REMOVED/RENAMED Requirements
  - Each requirement MUST include at least one #### Scenario: block
  - Debug parsed deltas: openspec show fix-aggregate-cold-start-and-audit-budget --json --deltas-only
```

### 2.2 `--strict`（块3）—— exit 1

```
$ npx --no-install openspec validate remove-single-repository-path --strict
Change 'remove-single-repository-path' has issues
⚠ [WARNING] logical-codebase-layout-guidance/spec.md: ADDED "布局规则表是唯一映射来源（REQ-LAYOUT-01）": Requirement text is very long (>500 characters). Move examples and edge cases into scenarios, or split it into separate requirements that each state one behavior.
⚠ [WARNING] logical-codebase-layout-guidance/spec.md: ADDED "入口目录树示意与失败面动作（REQ-LAYOUT-02）": Requirement text is very long (>500 characters). Move examples and edge cases into scenarios, or split it into separate requirements that each state one behavior.
Next steps:
  - Ensure change has deltas in specs/: use headers ## ADDED/MODIFIED/REMOVED/RENAMED Requirements
  - Each requirement MUST include at least one #### Scenario: block
  - Debug parsed deltas: openspec show remove-single-repository-path --json --deltas-only
```

### 2.3 非 strict（各 exit 0）

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
⚠ [WARNING] non-interrupt-repository-bootstrap/spec.md: ADDED "聚合初始化、聚合索引及登记布局失败面的字段化与文案单一来源（REQ-BOOT-08）": Requirement text is very long (>500 characters). …

$ npx --no-install openspec validate remove-single-repository-path
Change 'remove-single-repository-path' is valid
⚠ [WARNING] logical-codebase-layout-guidance/spec.md: ADDED "布局规则表是唯一映射来源（REQ-LAYOUT-01）": Requirement text is very long (>500 characters). …
⚠ [WARNING] logical-codebase-layout-guidance/spec.md: ADDED "入口目录树示意与失败面动作（REQ-LAYOUT-02）": Requirement text is very long (>500 characters). …
```

即：**除该 3 条体例告警外无任何 error / warning**（requirement 缺 SHALL/MUST、缺场景、重复条目、场景丢失等一律没有）。

### 2.4 基线复现（证明该告警由本次锁定文本引入）

把 `openspec/` 全量拷到 `/tmp/osbase2`，按 §1 反向还原四个文件到改前状态后执行（`openspec` 为全局 CLI v1.15.0）：

```
$ cd /tmp/osbase2 && openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0
$ cd /tmp/osbase2 && openspec validate remove-single-repository-path --strict
Change 'remove-single-repository-path' is valid
exit=0
```

requirement 正文长度（复刻 CLI 的 `extractRequirementBody` 口径：首个 header 前的非空行 trim 后以 `\n` 连接）：

| requirement | 改前 | 改后 | 限值 | 备注 |
|---|---|---|---|---|
| REQ-BOOT-08 | **493** | **1422** | 500 | 改前 493 距限值仅 7 字符 → 说明该正文原被刻意压在 500 以下 |
| REQ-LAYOUT-01 | **287** | **559** | 500 | |
| REQ-LAYOUT-02 | **289** | **504** | 500 | 仅超 4 字符 |

REQ-BOOT-08 六段长度：P1 157 / P2 364 / P3 349 / P4 114 / P5 194 / P6 239（合计 1422，含 5 个换行）。**单段均 <500**，所以「拆成多条 requirement、文字逐字不动」在技术上可行（§5-A1）。

## 3. 一致性核对

### 3.1 七处七码列表（逐字节）

核对脚本（Python，按含反引号与 ` / ` 的字面串匹配）：

```
七码串 = "`aggregate_root_is_git` / `aggregate_root_member_outside_root` /
`aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` /
`aggregate_root_overlap` / `aggregate_index_layout_unsupported` /
`aggregate_index_member_coverage_failed`"
命中数：块2 spec = 1（:113）；块2 tasks = 1（6.1）；块3 tasks = 1（7.3）；块3 spec = 0
```

结论：三处列表**逐字节相同**。块3 spec 不作列表 —— §2.3 在 REQ-LAYOUT-01/02 只写「七个布局码 / 七个对外码」并引用 `REQ-BOOT-08`，未给列表，故不存在第四处列表可核。各码名单独计数显示：块2 spec / 块2 tasks / 块3 tasks 均为七个各 1 次；块3 spec 仅 scenario 内既有的 `aggregate_root_is_git` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed` 与内部码（既有内容，本次未在改动范围）。

### 3.2 `FailureCopy` 五字段名与类型

- 块2 spec `:115`（P3）与块2 tasks `6.1`：`title_zh: String` / `reason_zh: String` / `action_zh: String` / `triage: FailureTriage` / `machine_line: String`，各命中 1 次，且均明写「必填、非 Option」（DTO 侧「不把 reason_zh 改为可选」）。
- 块3 spec `:54` / `:75` 与块3 tasks `7.4`：按字段名列举（`title_zh`/`reason_zh`/`action_zh`/`triage`/`machine_line`），与 §2.3 原文一致（§2.3 在此处不重复 Rust 类型，属预期）。
- 源码接地：`src/product/logical_codebase/failure_copy.rs:59-64` 的 `FailureCopy` 已是上述五字段同序、`:28` 为 `FailureTriage` —— 契约名与实现名一致。
- 关键断言落点：块2 spec P3「不把 reason_zh 改为可选」、块3 spec `:54`「缺 reason_zh 或缺 copy MUST NOT 作为正常缺省降级验收通过」、块2 tasks 6.8「缺 reason_zh 或缺 copy 不算通过」—— 三处同向，无互相削弱的表述。

### 3.3 场景解析

REQ-BOOT-08 = **11** 个场景（改前 7 + 新增 4，新增的四个均被正确解析）；REQ-LAYOUT-01 = 7、REQ-LAYOUT-02 = 3、REQ-LAYOUT-03 = 3（未动）。

### 3.4 主 specs 是否已有同名 requirement（冲突/重复检查）

```
$ grep -rn "REQ-BOOT-08\|REQ-LAYOUT-0" openspec/specs/
(none)
$ ls -d openspec/specs/logical-codebase-layout-guidance
(no such main spec)
$ grep -n "### Requirement:" openspec/specs/non-interrupt-repository-bootstrap/spec.md
9:  REQ-BOOT-01 / 42: 生产执行 / 62: REQ-BOOT-05 / 106: REQ-BOOT-06 / 145: 聚合重新初始化入口 /
159: REQ-BOOT-03 / 202: REQ-BOOT-04
```

结论：主 specs 内**没有** `REQ-BOOT-08`、`REQ-LAYOUT-01/02`，也**没有** `logical-codebase-layout-guidance` 主 spec；`non-interrupt-repository-bootstrap` 主 spec 无同标题 requirement ⇒ 不冲突、不重复，未改主 specs。

## 4. 「块3 不建文案表 / 布局端点 / LayoutFailureAction」残留检查

```
$ cd .../bugfix-b-11-02
$ grep -n "layout_guidance\.rs\|LayoutFailureAction" openspec/changes/remove-single-repository-path/tasks.md
(全文件无命中)

$ awk 'NR>=89 && NR<=98' openspec/changes/remove-single-repository-path/tasks.md | grep -nE "layout_guidance|LayoutFailureAction|布局端点|/layout"
(no match)
```

改后组 7（:93-:97）中与「自建」相关的全部字样只有**禁止句**本身，且已不含「布局端点」：

```
$ sed -n '89,98p' openspec/changes/remove-single-repository-path/tasks.md | grep -n "自建\|文案表"
5:- [ ] 7.1 … MUST NOT 在块3 自建规则表或文案表
(7.4 现为「MUST NOT 在块3 另建文案来源」)
```

块2 组 6 亦无「建第二来源」的表述：6.1 明写「一次写全…块3 MUST NOT 追加写本文件」，6.5/6.6 只是 DTO/TS 逐字段搬运。

## 5. 需 controller 裁定

### A（主要，阻断 `--strict`）500 字符体例门禁 vs 锁定文本

事实：三条 ADDED requirement 正文 493→1422、287→559、289→504，而 CLI 的 `MAX_REQUIREMENT_TEXT_LENGTH = 500`（`@fission-ai/openspec@1.15.0` `dist/core/validation/constants.js:9`，仅对 **ADDED** 生效；注释明确「Same limit the main spec enforces after archive (#1976)」）。基线（改前）严格校验为绿，故红灯完全由锁定文本的字数引入。旁证：仓库主 specs 561 条 requirement **全部 ≤500**（最大 497），改前 BOOT-08 正是 493 —— 该项目事实上把 500 当硬线。

三种出路（我未擅自选，均需授权）：

- **A1（推荐，零措辞变更）**：授权把这三条过长的 ADDED 正文按「一条 behavior 一 requirement」拆成多条 ADDED requirement，**文字逐字保留**（单段均 <500，例如 BOOT-08 可拆为「范围与同源」「七码登记」「五字段合同与未知码兜底」三条）。需要 controller 给出新 REQ id 与拆分点；授权后我可执行并复跑 `--strict`。
  - 可用的现成切点（都已实测 <500）：BOOT-08 的 P1 157 / P2 364 / P3 349 / P4 114 / P5 194 / P6 239，任选分段即可，例如 `{P1+P5（352）, P2（364）, P3+P4（464）, P6（239）}`；
  - LAYOUT-01 只需把既有 `:11`（200）与新增 `:13`（358）拆到不同 requirement；
  - LAYOUT-02 只需把既有 `:52`（145）与新增 `:54`（358）拆到不同 requirement。
  - 注意：拆分会新增契约条目标识（REQ id），因此必须有 controller/owner 明确授权；在获授权前我保持单条现状。
- **A2（接受偏差）**：保留现状，把 3 条 >500 告警作为**已知偏差**登记在 change 内，以「非 strict valid + delta 结构正确 + 零 error」作为门禁替代。代价：archive 时同一限值会在主 spec 上再次触发（`openspec validate --specs` 会红），届时仍要改。
- **A3（不推荐、且被本任务禁止）**：删改锁定措辞压到 500 以内。

补充：REQ-LAYOUT-02 仅超 4 字符，但**不存在措辞无损的合规路径** —— 把 `:52` 与 `:54` 合并成一段（少 1 个换行）仍为 503；`:52` 是既有规范内容不可删。故只能走 A1 或 A2。

### B. REQ-LAYOUT-01 是否保留 `:11` 的六规则枚举

现状：保留（§2.3 只点名 `:13`）。保留的读法：`系统 SHALL 维护单一布局规则表` 指平台层唯一表（即 `REQ-BOOT-08` 来源），与 `本能力 MUST NOT 自建第二份` 不冲突；且删除会移除本能力对「六条规则是什么」的规范陈述（虽 Purpose 与 6 个 scenario 仍覆盖）。

代价数据：若删除 `:11`，`LAYOUT-01` 正文 559 → **358**，块3 的该条 strict 告警即消失。**需 controller 明确「删除枚举算不算改契约内容」**；我不擅自删。

### C. 块3 tasks `11.4` 的「六类布局错误各给动作」

该行（组 7 之外、`11.1-11.5` 验收组）写的是「六类布局错误各给动作」。两种读法：①「六类」=六条规则（第二条规则对应两个码）⇒ 无需改；② 指对外码 ⇒ 应为「七类」。本次**未改**（§6.1 只要求组 7 同步），请裁定是否一并改为「七个对外稳定码」。

## 6. 接地事实（供实施与后续核对）

- `src/product/logical_codebase/failure_copy.rs` **已存在**（58653 B，2026-11-03 08:38），含 `FailureCopy` 五字段（:59-64）、`FailureTriage`（:28）、`LAYOUT_RULES` 六规则（:160，测试 `LAYOUT_RULES.len() == 6`）、七码（含 `aggregate_index_layout_unsupported`）、`describe(FailureInput)`（:1003）。与 tasks 6.1 的措辞吻合；tasks 仍按计划口径写「新建」（全文件任务未勾选）。
- `src/web/types/logical_codebase_bootstrap.rs` 与 `web/src/api/types/logical-codebase-bootstrap.ts` 当前**均无** `FailureCopyDto` / `copy` 字段 ⇒ tasks 6.5 / 6.6 是**有效的新增项**（非重复）。
- `src/web/handlers/support_parts/product_store_error.inc.rs:9-31` 的 `aggregate_root_api_error(error: AggregateRootPreflightError)` 存在 ⇒ tasks 6.4 的接线点接地。
- 两个 change 目录在 git 中仍为 untracked（`?? openspec/changes/fix-aggregate-cold-start-and-audit-budget/`、`?? openspec/changes/remove-single-repository-path/`），故无法用 `git diff` 展示；§1 的「改前」内容取自改动前对四文件的完整读取。**未执行 `git add`**。
- 未触碰：任何 `cadence/plans/`（含 B2-Plan `:410-437` 漏项本身）、`cadence/designs/`、源码、`openspec/specs/`。

---

# 附：A1 拆分落地（controller 2026-11-03 裁定后执行）

- 状态：**DONE**。三个 change 的 `--strict` 全绿（exit 0）。
- 执行范围：`openspec/changes/fix-aggregate-cold-start-and-audit-budget/{specs/non-interrupt-repository-bootstrap/spec.md, tasks.md}`、`openspec/changes/remove-single-repository-path/{specs/logical-codebase-layout-guidance/spec.md, tasks.md, design.md}`。
- 文字纪律：**逐字搬运**，唯一改动是「拆段 + 改标题里的编号 + 引用同步」。未精简、未改写任何句子。

## 7.1 旧编号 → 新编号完整映射（块2 `non-interrupt-repository-bootstrap`）

拆分前该条为 `REQ-BOOT-08`（正文 P1~P6 + 11 个场景）。拆分后：

| 新编号 | 标题 | 装载内容（逐字） | 场景 |
|---|---|---|---|
| **REQ-BOOT-08** | 聚合初始化与聚合索引失败面的字段化与文案单一来源 | **P1**（范围与同源）+ 原 `:119` 保留段 **P5**（每条失败面四要素：中文主文案 / 动作 / 分诊 / 机器码） | 失败面给出中文主文案与可执行动作；三类分诊给出不同下一步（2） |
| **REQ-BOOT-10** | 布局规则与七个对外稳定码的唯一登记 | **P2**（六条布局规则 + 七个对外稳定码列表 + MUST NOT 省略登记 + 内部码只作反查） | 无 action 的布局失败同源；单点修改覆盖两出口（2） |
| **REQ-BOOT-11** | FailureCopy 五字段合同 | **P3**（五字段必填非 Option + 码表每条非空三段 + 七码须给规则依据与处置动作 + 无 action 不构成例外 + DTO 完整透传） | 字段完整且跨链同构（1） |
| **REQ-BOOT-12** | 未知码的中性兜底 | **P4**（未知码中性问题/诚实未知原因/可执行下一步 + MUST NOT 编造 + 已知码不得被当未知码 + 兜底不替代七码验收） | 未知码中性兜底不伪装布局（1） |
| **REQ-BOOT-13** | 失败面原始输出的脱敏、截断与成因标注 | **P6**（原 `:113` 脱敏/截断/诚实 `unknown` 段） | 原始输出被截断；凭据与身份路径被脱敏；最早期兜底；成因不可判定；确定性成因（5） |

- `REQ-BOOT-09`（codegraph 前置探测）**未改**，现位于 `REQ-BOOT-13` 之后。
- 场景守恒：2+2+1+1+5 = **11**（拆分前同为 11，零丢失、零新增）。
- **P6 为什么单独立条**：Main 指定 `REQ-BOOT-08 = P1 + P5 + P6`，实测 157+194+239+2 = **592 > 500**；按 ⑤「若拆完还有 >500 的继续拆到 ≤500」，把 P6 独立为 `REQ-BOOT-13`，`REQ-BOOT-08` 落为 352。编号 13 由我取（Main 指定的 08/10/11/12 已用满）。

## 7.2 旧编号 → 新编号完整映射（块3 `logical-codebase-layout-guidance`）

| 新编号 | 标题 | 装载内容（逐字） | 场景 |
|---|---|---|---|
| **REQ-LAYOUT-01** | 布局规则与稳定错误码 | 原 `:11` 六规则枚举（保留原文，**未删规范内容**） | 既有 7 个（6 条规则 + 每个布局错误码都有动作） |
| **REQ-LAYOUT-02** | 入口目录树示意 | 原 `:52` 目录树与可索引范围句 | 入口展示目录树（1） |
| **REQ-LAYOUT-04** | 失败面三段式渲染（问题 / 原因 / 怎么做） | 原 `:54` 三段式段（首句保留 + 新增整句） | 失败面给出具体动作（1） |
| **REQ-LAYOUT-05** | 布局规则表是唯一映射来源 | 原 `:13` 整句（块2 唯一来源 + MUST NOT 自建 + 五字段绑定 + reason_zh 必填 String 非 Option + 内部码反查 + 不得只有码无动作） | 文案单一来源（1） |
| **REQ-LAYOUT-03** | 入口的依赖前置提示 | **未动** | 3 个（未动） |

- 场景守恒：7+1+1+1+3 = **13**（拆分前同为 13）。
- 原 `:75` 的「文案单一来源」THEN 随场景归入 `REQ-LAYOUT-05`；原 `:52`/`:54` 已各自成段。

## 7.3 每条 requirement 的最终字符数（口径同 CLI `extractRequirementBody`，限值 500）

块2 `non-interrupt-repository-bootstrap`：

| requirement | 字符数 | 状态 |
|---|---|---|
| REQ-BOOT-03（未动） | 415 | ✓ |
| REQ-BOOT-07（未动） | 483 | ✓ |
| **REQ-BOOT-08** | **352** | ✓（拆分前 1422） |
| **REQ-BOOT-10** | **364** | ✓ |
| **REQ-BOOT-11** | **349** | ✓ |
| **REQ-BOOT-12** | **114** | ✓ |
| **REQ-BOOT-13** | **239** | ✓ |
| REQ-BOOT-09（未动） | 396 | ✓ |

块3 `logical-codebase-layout-guidance`：

| requirement | 字符数 | 状态 |
|---|---|---|
| **REQ-LAYOUT-01** | **200** | ✓（拆分前 559） |
| **REQ-LAYOUT-02** | **145** | ✓ |
| **REQ-LAYOUT-04** | **358** | ✓（拆分前 504） |
| **REQ-LAYOUT-05** | **430** | ✓ |
| REQ-LAYOUT-03（未动） | 325 | ✓ |

全仓复查：`openspec/changes/**/specs/*/spec.md` 中 ADDED requirement 正文 **>500 的条数 = 0**。

## 7.4 三个 validate 逐字输出（**全绿 exit 0**）

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0

$ npx --no-install openspec validate remove-single-repository-path --strict
Change 'remove-single-repository-path' is valid
exit=0

$ npx --no-install openspec validate fix-automation-enrollment-provider-swap --strict
Change 'fix-automation-enrollment-provider-swap' is valid
exit=0
```

（无任何 WARNING / ERROR 输出——拆分前的 3 条 >500 告警已全部消失，且未引入新告警。）

## 7.5 跨文件引用同步逐处清单

| # | 文件:行 | 改前 | 改后 |
|---|---|---|---|
| 1 | 块3 spec `:13` → 现 `:74`（REQ-LAYOUT-05 正文） | 「MUST 登记在 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08` 定义的唯一失败面文案来源中」 | 「…的 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 共同定义的唯一失败面文案来源中」 |
| 2 | 同上（同段落） | 「…以及 `triage`、`machine_line` 五字段合同；其中reason_zh…」 | 「…五字段合同（五字段合同见 `REQ-BOOT-11`）；其中reason_zh…」 |
| 3 | 块3 spec `:54` → 现 `:65`（REQ-LAYOUT-04 正文） | 「文案 MUST **取自** … 的 `REQ-BOOT-08` 登记的规则表」 | 「… `REQ-BOOT-10` 登记的规则表」（规则表登记随拆分归 BOOT-10；③ 表未列，按「引用不再准确必须改」补） |
| 4 | 块3 spec `:75` → 现 `:79`（REQ-LAYOUT-05 场景 THEN） | 「落在 `REQ-BOOT-08` 登记的六规则/七个对外码唯一来源中」 | 「落在 `REQ-BOOT-10` 登记的六规则/七个对外码唯一来源中」 |
| 5 | 块3 spec `:79` → 现 `:83`（REQ-LAYOUT-03 正文） | 「提示文案 MUST 与 `REQ-BOOT-08` 同源引用」 | 「…与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源引用」 |
| 6 | 块3 `design.md:113` | 「由 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08` 拥有」 | 「…的 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 拥有」 |
| 7 | 块3 `design.md:114` | 「同样与 `REQ-BOOT-08` 同源」 | 「同样与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源」 |
| 8 | 块2 `tasks.md:61`（组6 映射行） | 「映射 requirement：REQ-BOOT-08」 | 「映射 requirement：REQ-BOOT-08、REQ-BOOT-10、REQ-BOOT-11、REQ-BOOT-12、REQ-BOOT-13」（组6 含七码/六规则、五字段、单一来源、未知码兜底、"no stderr summary" 验收） |
| 9 | 块2 `tasks.md:74`（组7 映射行） | 「映射 requirement：REQ-BOOT-08」 | 「映射 requirement：REQ-BOOT-13」（组7 全为脱敏/截断/cause） |
| 10 | 块2 `tasks.md:50`（组5 映射行） | 「REQ-PCR-02、REQ-BOOT-08」 | **未改**：组5 内容是中文主文案/分诊/机器码降级＝单一来源与四要素 ⇒ 仍指向 `REQ-BOOT-08` |
| 11 | 块3 `tasks.md:91`（组7 映射行） | 「`REQ-LAYOUT-01`、`REQ-LAYOUT-02`（文案依赖块2 `REQ-BOOT-08`）」 | 「`REQ-LAYOUT-01`、`REQ-LAYOUT-02`、`REQ-LAYOUT-04`、`REQ-LAYOUT-05`（文案依赖块2 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12`）」 |
| 12 | 块3 `tasks.md:96`（7.4） | 「块2 `REQ-BOOT-08` 登记的 `failure_copy.rs` 成品」 | 「块2 `REQ-BOOT-10` / `REQ-BOOT-11` 登记的 `failure_copy.rs` 成品」 |
| 13 | 块3 `tasks.md:103`（8.1） | 「文案与 `REQ-BOOT-08` 同源」 | 「文案与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源」 |
| 14 | 块3 `tasks.md:42`（2.4） | 「接入失败面单一来源（块2 `REQ-BOOT-08`）」 | **未改**：单一来源 ⇒ `REQ-BOOT-08` 仍准确 |
| 15 | 块3 `tasks.md:134`（11.4） | 「六类布局错误各给动作」 | 「六条布局规则各给动作」（见 §7.6） |

`cadence/plans/`、`cadence/designs/`（技术方案）、`cadence/notes/` 内的 `REQ-BOOT-08` 字样**一律未动**（按裁定）。

## 7.6 ④ 的判断与理由：`11.4` 选「规则」读法

改后：「…3 成员 LC 仓序正确；**六条布局规则各给动作**；一键核验 4 家并发出进度…」

理由：
1. 句子在**真机验收**清单里，人工只能物理制造「六种布局错误场景」（根是 git 仓 / 成员不在直接子目录 / symlink 逃逸 / 根重叠 / 嵌套 worktree / 无可索引源码）——正对应六条规则；第七条码（`aggregate_index_layout_unsupported`）与 `aggregate_root_member_outside_root` 由同一个「层级不对」场景产出，人工无法独立造出第七种。
2. 原文数字「六」与六条规则数量吻合；若作者本意是错误码，按当时 7.3 的写法会写「错误码」而非「布局错误」。
3. 码级逐条验收已由 7.3（七个对外稳定码逐一接入三段式）与 7.5（逐条断言）覆盖，因此 11.4 用「六条布局规则」不降低码级覆盖。

## 7.7 需 controller 知晓的 4 处执行偏差

1. **新增 `REQ-BOOT-13`**：Main 指定的 4 个编号装不下 592 字符的 P1+P5+P6；按 ⑤ 继续拆分，编号 13 由我取。若想保持纯数字顺序可改为 08/09/10/11/12（把现 BOOT-09 顺延），但那需要移动 codegraph 整条（396 字 + 5 场景）——未擅自做。
2. **`design.md:113/114` 已改**：这与「只许改 `specs/**/*.spec.md` 与 `tasks.md`」的字面约束冲突，但 ③ 明确列出了这两行；我按 ③ 执行（design.md 属 change 自身产物，不在 Plan／技术方案／notes／源码／主 specs 的禁改范围）。
3. **块3 spec `:65`（原 `:54`）额外同步为 `REQ-BOOT-10`**：③ 表未列该处，但「`REQ-BOOT-08` 登记的规则表」在拆分后已不准确（规则表登记在 BOOT-10）。
4. **块2 spec 内 requirement 顺序**为 `BOOT-03 → BOOT-07 → BOOT-08 → BOOT-10 → BOOT-11 → BOOT-12 → BOOT-13 → BOOT-09`；`REQ-BOOT-09` 未移动到新块之前（避免搬动与本次拆分无关的 396 字正文与 5 个场景）。

## 7.8 拆分后复核（脚本 + grep 证据）

- 七码逐字列表命中数：块2 spec 1、块2 tasks 1、块3 tasks 1（三处**逐字节相同**）；块3 spec 按设计只引用「七个布局码 / 七个对外码」，不给列表。
- `FailureCopy` 五字段名四处齐全；`title_zh: String` / `reason_zh: String` / `action_zh: String` / `machine_line: String` / `FailureTriage` 在块2 spec 与块2 tasks 各 1 次，块3 spec 前三者各 1 次（§2.3 原文粒度），块3 tasks 按字段名列举。
- 块3 组7 禁词检查（实跑）：
  ```
  $ awk 'NR>=89 && NR<=98' openspec/changes/remove-single-repository-path/tasks.md \
      | grep -nE "layout_guidance\.rs|LayoutFailureAction|布局端点|/layout"
  (no match)
  ```
- 全仓 `ADDED` requirement 正文 >500 条数：**0**。
- 未 `git add`；未改源码、Plan、技术方案、notes、`openspec/specs/`。

---

# 附二：Plan / 技术方案的 REQ 引用同步（controller 2026-11-03 第二轮）

- 状态：**DONE**。只改这四个文件；`openspec/` 零改动。
- 判据：**① 「单一来源这件事」→ 不动；② 指向具体内容（七码列表 / 五字段 / 未知码兜底 / 脱敏成因场景 / 逐字引用行号）→ 改**。
- 统计：命中 **22 行**（块2 Plan 8 + 块3 Plan 14）⇒ **改 16 行 / 判定保留 6 行**；另有 4 份文件各加一段「REQ 编号映射」。技术方案两文件命中数为 **0**（无任何 `REQ-BOOT-08` / `REQ-LAYOUT-0x` 引用），故只加映射段。

## 8.1 改动逐条（文件:行 → 改前 → 改后）

行号均为**改后**行号；括号内为改前行号（插入映射段导致块2 Plan +16 行、块3 Plan 从 :167 起 +16 行）。

### 块2 Plan（`cadence/plans/2026-11-03_计划文档_实施_块2_聚合冷启动与审计预算_v1.0.md`）—— 改 4 处

| 改后行（改前） | 改前 | 改后 |
|---|---|---|
| `:488`（`472`） | `REQ-LAYOUT-02「MUST 与失败面文案单一来源同源，MUST NOT 另抄一份」` | `REQ-LAYOUT-04「文案 MUST 取自同一失败面文案来源，MUST NOT 另抄一份」` |
| `:518`（`502`） | `（REQ-BOOT-08 场景「成因不可判定时诚实标注」）` | `（REQ-BOOT-13 场景「成因不可判定时诚实标注」）` |
| `:1103`（`1087`） | `（REQ-LAYOUT-02 同源要求）` | `（REQ-LAYOUT-04 同源要求）` |
| `:1197`（`1181`） | `REQ-LAYOUT-02 要求「布局动作文案 MUST 与 REQ-BOOT-08 的单一来源同源，MUST NOT 另抄一份」` | `REQ-LAYOUT-04 要求「布局动作文案 MUST 取自 REQ-BOOT-08 / REQ-BOOT-10 / REQ-BOOT-11 / REQ-BOOT-12 / REQ-BOOT-13 共同定义的单一来源，MUST NOT 另抄一份」` |

### 块3 Plan（`..._块3_去单仓入口与统一多仓流程_v1.0.md`）—— 改 12 处

| 改后行（改前） | 改前 | 改后 |
|---|---|---|
| `:108`（`108`） | `:54`（REQ-LAYOUT-02）+ 引文「该文案 **MUST 与失败面文案的单一来源同源**（`REQ-BOOT-08`）」 | `:65`（REQ-LAYOUT-04）+ 引文「文案 **MUST 取自** `REQ-BOOT-10` 登记的规则表，**MUST NOT 另抄一份**」 |
| `:109`（`109`） | `:75`：「…单一来源（`REQ-BOOT-08`）里…」 | `:79`（REQ-LAYOUT-05 场景）：「…落在 `REQ-BOOT-10` 登记的六规则/七个对外码唯一来源中…」 |
| `:141`（`141`） | `REQ-LAYOUT-01`「任何布局错误码 MUST 能映射到表中一行」 | `REQ-LAYOUT-05`「…」（同句已随拆分移入 LAYOUT-05） |
| `:148`（`148`） | `（… REQ-LAYOUT-01）拟增：规则表 MUST 登记在 REQ-BOOT-08 单一来源中` | `（… REQ-LAYOUT-05）拟增：…登记在 REQ-BOOT-08 / 10 / 11 / 12 / 13 共同定义的唯一来源中` |
| `:149`（`149`） | `（… spec.md REQ-BOOT-08）拟增：单一来源 MUST 登记上述 7 个码` | `（… REQ-BOOT-10）拟增：…`（七码登记已拆入 BOOT-10） |
| `:705`（`689`） | `理由见 §0.7（REQ-LAYOUT-02 的 MUST NOT 另抄一份）` | `…（REQ-LAYOUT-04 / REQ-LAYOUT-05 的 MUST NOT 另抄一份 / MUST NOT 自建第二份）` |
| `:696`（`680`） | `映射 requirement：REQ-LAYOUT-01、REQ-LAYOUT-02、REQ-LAYOUT-03` | `… REQ-LAYOUT-01、REQ-LAYOUT-02、REQ-LAYOUT-04、REQ-LAYOUT-05、REQ-LAYOUT-03`（补齐拆分新增的两条） |
| `:754`（`738`） | `REQ-LAYOUT-02（spec :54/:75）MUST NOT 另抄一份` | `REQ-LAYOUT-04 / REQ-LAYOUT-05（spec :65/:79）MUST NOT 另抄一份` |
| `:783`（`767`） | `（REQ-LAYOUT-02 spec :54/:75）` | `（REQ-LAYOUT-04 / REQ-LAYOUT-05 spec :65/:79）` |
| `:1117`（`1101`） | `片 5 阻塞 / REQ-LAYOUT-01 不成立` | `片 5 阻塞 / REQ-BOOT-10 / REQ-LAYOUT-05 不成立` |
| `:1123`（`1107`） | `~~REQ-LAYOUT-02 的「原因」字段…~~` | `~~REQ-LAYOUT-04 的「原因」字段…~~`（仅换编号；该行「缺省降级」表述属另一待办，见 §8.4） |
| `:1132`（`1116`） | `REQ-LAYOUT-01`「任何布局错误码 MUST 能映射到表中一行」 | `REQ-LAYOUT-05`「…」 |

## 8.2 判定保留的 6 行（按 ①/② 判据逐条说明）

| 文件:行（改后） | 保留的引用 | 理由（① 单一来源） |
|---|---|---|
| 块2 Plan `:435`（原 `419`） | `REQ-LAYOUT-01` 在该码上不成立 | 这句讲「该布局码拿不到自己的稳定码 / 精确动作文案」——**未指向七码列表 / 五字段 / 兜底 / 脱敏**；`REQ-LAYOUT-01`（布局规则与稳定错误码）正含「每个布局错误码都有动作」场景，六规则枚举亦仍逐条绑定稳定码 ⇒ 引用**仍然成立** |
| 块2 Plan `:447`（原 `431`） | `REQ-LAYOUT-01`（判定规则一字不改） | controller 明列保留；拆分后 `REQ-LAYOUT-01` 就是「布局规则与稳定错误码」，该句仍成立 |
| 块2 Plan `:1168`（原 `1152`） | `REQ-BOOT-08「单一来源」不变` | **① 单一来源这件事本身**——正是新 `REQ-BOOT-08` 的语义，不改 |
| 块2 Plan `:1211`（原 `1195`） | D-06 溯源列 `REQ-BOOT-08` | 该行断言「`failure_copy` 是文案唯一来源」＝**①**，落点仍是 `REQ-BOOT-08` |
| 块3 Plan `:756`（原 `740`） | `REQ-BOOT-08 落点` | 指「失败面渲染段归属块2 片2」＝**①**（单一来源落点），非某个具体内容 |
| 块3 Plan `:762`（原 `746`） | `REQ-LAYOUT-01「本能力 MUST NOT 新增或放宽任何判定」` | 未指向七码列表/五字段/兜底/脱敏；`REQ-LAYOUT-01` 仍是布局规则条目，判定不动的引用**不失效**（该句原文位于 spec `## Purpose`，改写它属内容变更，不在本轮范围） |

（`块3 Plan :696` 与 `:1132`/`:1117` 等行虽仍含 `REQ-LAYOUT-01/02`，但已是**同步后的正确用法**——`REQ-LAYOUT-02` 仅作目录树示意出现在映射清单里。）

## 8.3 映射段加在哪（4 份文件各 1 段，文字与 controller 给定稿逐字一致）

| 文件 | 位置 | 行区间 |
|---|---|---|
| 块2 Plan | §0「全局约束」之后、`## 1. 切片总表` 之前 | `:38`–`:52` |
| 块3 Plan | §0.7.5 之后、`## 1. 切片总览` 之前 | `:167`–`:181` |
| 技术方案 块2 | §0「一句话方案」之后、`## 1. 问题与根因` 之前 | `:16`–`:30` |
| 技术方案 块3 | 同上 | `:15`–`:29` |

段标题统一为 `### REQ 编号映射（2026-11-03 拆分后）`。

## 8.4 复查：残留命中逐条说明

```
$ grep -rn 'REQ-BOOT-08\|REQ-LAYOUT-01\|REQ-LAYOUT-02' cadence/plans/ cadence/designs/
```

改后命中分布（实测：**29 行 / 34 次**；改前 22 行 / 26 次）：

| 文件 | 命中行 | 性质 |
|---|---|---|
| 块2 Plan | `:40`,`:41`,`:47`,`:48`,`:49` | **映射段自身**（正文需列举旧编号） ⇒ 保留 |
| 块2 Plan | `:435`,`:447` | ① 判定保留（§8.2 #1、#2） |
| 块2 Plan | `:1168`,`:1211` | ① 单一来源 / D-06 溯源（§8.2 #3、#4） |
| 块2 Plan | `:1197` | 已同步为 **5 个编号连写**（含 `REQ-BOOT-08`） ⇒ 命中属预期 |
| 块3 Plan | `:148` | 已同步为 **5 个编号连写** ⇒ 预期 |
| 块3 Plan | `:169`,`:170`,`:176`,`:177`,`:178` | 映射段自身 ⇒ 保留 |
| 块3 Plan | `:696` | 已同步的切片映射清单（`REQ-LAYOUT-01/02` 在此仍是正确需求名） |
| 块3 Plan | `:756`,`:762` | ① 判定保留（§8.2 #5、#6） |
| 技术方案 块2 | `:18`,`:19`,`:25`,`:26`,`:27` | 映射段自身 ⇒ 保留 |
| 技术方案 块3 | `:17`,`:18`,`:24`,`:25`,`:26` | 映射段自身 ⇒ 保留 |

即：**除 4 段映射自身、以及 6 处按 ① 判据保留的正确引用外，无其他残留**。四份受改文件内旧行号引用（`:54` / `:75`）**实测计数均为 0**（`grep -c` 逐文件 0/0/0/0；仓库其他 2026-07/10 的旧 Plan 里的 `:54`/`:75` 与本次布局契约无关，未碰）；`REQ-LAYOUT-02` 的余下出现均限定在**目录树示意**语义与映射清单内。

## 8.5 未做 / 提请注意

1. **块3 Plan R10（`:1123`）的「`FailureCopy` 无 `reason_zh` ⇒ 原因按缺省降级」表述仍在**。这属**内容口径**问题（专项 §1.1.3 已判定与 `REQ-LAYOUT-04`「缺 reason_zh 不得按缺省降级验收通过」冲突），但本轮约束是「只动 REQ 引用与映射段」，故**只换了编号**，该措辞的处置请 controller 另行派单。
2. **块3 Plan `:762`（原 `746`）引用的是 spec `## Purpose` 的句子**（「本能力 MUST NOT 新增或放宽任何判定」），而非 requirement 正文；若要求引用精确，可改为「spec Purpose 段」，未擅自改写。
3. 未改任何切片的文件清单 / 估时 / TDD 测试点 / 独占清单；未改 `openspec/`、源码、notes；未 `git add`。
