# 聚合索引源码范围判定与 Story 前共享索引刷新 · 设计

产品边界以 [proposal](proposal.md) 为准；可验收条款见 [specs/](specs/)；高层工作包见 [tasks](tasks.md)。本文件记录**已确认的架构边界与取舍**，路径相对仓库根，行号一律使用符号锚点或函数名（易漂移的行号只在必要时给出）。

本版相对上一版的三处裁决（已定，不再讨论）：

1. **D-2 收敛为 Story 专用**：只有 Story 生成前的刷新要求「最新」，其余只读规划（Design、WorkItemPlan、工作区上下文读取）继续 last-known-good + 告警，不因 degraded 停机。
2. **`REQ-IND-06` 不做自动发现**：新增源码只在「下次验收/重建」时重新判断，不引入后台扫描、不在 freshness 里盘检。
3. **告警结构化**：卡片与成员列表的跳过提示是 `{code, message_zh}` 对象，与证据面 `not_searched.code` 同构；不是纯字符串。
4. **`not_searched` 的审计原因是纯加法**：既有审计字段与判定口径不变，新字段可选、只解释「为什么没检索」。
5. **源码后缀探针不是开工门禁**：探针并行前置，先按保守清单落地盘检；探针结论回来以「增补清单」方式落地。

## D1 · 判定与跳过的数据流（谁在何时判、事实落在哪、谁消费）

**判定点唯一。** `AggregateIndexAcceptance::verify`（`src/product/logical_codebase/aggregate_index/operation.rs` 的 `verify`，返回 `Ok(Self { … })`）是**首次建库、手动重建、Story 前同步**三条入口共用的验收入口；修复必须落在这里，不能只改「重建索引」按钮的报错。

判定顺序（先覆盖、后盘检、再汇总）：

1. **成员覆盖**：SDK 逐成员 `hasFilesUnder` 为主路径；SDK 不可用/不兼容才回退 CLI `files --json`（`verify` 内的回退块与 `verify_member_coverage`）。CLI 的空索引提示 `codegraph_files_empty_index` 在验收层归一为「全员未覆盖 → 进入交叉验收」，**不**解释成截断、非法 JSON 或协议错误。
2. **有界盘检**：只对**未被覆盖**的成员，在其**主工作目录**内按 Aria 自有源码类型清单做一次有界盘检（口径见 D2：源码候选只取 **git 可见范围** `git ls-files --cached --others --exclude-standard` ∩ Aria 源码后缀集，扣除 Aria 自身排除；**被 `.gitignore` 忽略的文件不算源码**），产出四值结论：`SourceInNormalPath` / `SourceOnlyInWorktrees` / `NoSource` / `Unknown`（权限、超时、超限、IO 失败、git 侧失败一律 `Unknown`）。
3. **汇总**：
   - 至少一个成员被覆盖，且其余未覆盖成员**全部**为 `NoSource` ⇒ 通过其它验收后可发布 active，并把 `NoSource` 成员记为本世代的**跳过事实**；
   - 出现 `SourceInNormalPath`（漏收）/ `SourceOnlyInWorktrees` / `Unknown` / 全部成员 `NoSource` ⇒ **失败**，不发布新 active；
   - 成员在覆盖判定结果里**没有键**（未被判定、未被扫描）⇒ 按 `Unknown` 处理，MUST NOT 计入「全员确认无源码」。

**事实落在哪。** 跳过事实是**成功世代**里成员快照上的一个可选标记（见 D4），不是另一张表、不是第二份 durable 成员列表、不是运行期缓存。同一逻辑代码库只有一份共享索引世代记录。**跳过事实不跨代继承**：每次验收都重新判定，MUST NOT 把上一代结论直接当成本次结论。

**谁消费。**

| 消费者 | 位置 | 消费方式 |
|---|---|---|
| 聚合索引卡片 | `src/web/handlers/aggregate_index.rs`（`AggregateIndexActiveResponse`、`read_active_projection`） | 从**本次实际被投影的那一份记录**派生一条结构化告警 `{code, message_zh}`；不扫盘、不启 CodeGraph。投影选哪一代（degraded 优先、Building、Failed+LKG 等）已经由 `read_active_projection` 决定，跳过告警 MUST 复用同一个选择结果，不得另选一遍记录 |
| 创建 Issue 选成员 | `src/web/handlers/logical_codebase_members.rs`（`list_logical_codebase_members_for_lc`，入参 `project_id` + `logical_codebase_id`） | 同一 LC 当前世代 → 逐成员 `{code, message_zh}`；按 **(成员 ID, 当前 Main checkout ID)** 匹配（见 D7），仅影响提示，不影响资格 |
| 跨仓证据链 | `src/product/logical_codebase/evidence_mediator.rs` | 目标为跳过成员 ⇒ 不发起 CodeGraph 查询，返回显式 `not_searched: {code, message_zh}`（D6） |
| 规划指纹 | `src/product/logical_codebase/planning_context_resolver.rs`（`build_member_fingerprints`） | 跳过成员仍产出指纹（成员 ID / checkout / revision / dirty），`available` 取既有口径，不因跳过而变成不可用 |

**不因跳过而改变的东西**：项目成员集合（不移除成员）、成员授权与 ACL、选仓资格、checkout 与工作区准入、Issue 的 `selection.focus_repository_ids` 语义。

## D2 · Aria 源码口径与探针（保守清单先开工）

**约束（硬）**：Aria 自有的源码类型清单必须满足「**由 Aria 自有**」且「**是 CodeGraph 支持后缀的真子集**」。理由：若清单纳入 CodeGraph 根本不索引的后缀，会把「CodeGraph 根本不索引的文件」误判成「索引漏收」并报错——这正是本变更要消灭的假阳性。

**实施顺序（已裁决）**：**保守清单先开工，探针并行前置，结论回来再增补**。

- 保守清单只纳入「确定被 CodeGraph 收录」的常见源码后缀（`.rs/.ts/.tsx/.js/.jsx/.py/.go/.java/...` 这类），实施不等待探针。
- 逐后缀探针按语言各造**最小有效样本**（不能拿同一个 JS 样本改后缀），每个版本用独立临时根，按生产 argv 跑 `init .` / `sync .` / `files --json`，正反两组逐项出表；产物只落 `/tmp`。
- 探针必须覆盖部署支持的版本基线（现行兼容区间 `>=1.6.0, <2.0.0`，`src/product/logical_codebase/aggregate_index/codegraph_cli.rs` 的版本常量与探测函数）。
- **可见范围口径探针（G5）**：每个版本基线各验一次「被 `.gitignore` 忽略的文件不被 codegraph 收录、git 可见范围内的文件（含未跟踪但未被忽略的）被收录」这一条，正反两组逐项出表，使探针结论与扫描口径一致；该探针与后缀探针同属**并行前置、非阻塞门禁**，未跑完不阻塞盘检实现与其余工作包。
- 探针结论 SHALL 以「增补已登记后缀」的方式落地；探针未跑完 **MUST NOT 阻塞**盘检实现与其余工作包。保守清单的偏保守后果是「多跳过」（不报错）而不是「误报错」，方向安全；探针回来后增补即可减少跳过面。
- 清单、兼容基线或推荐版本变更前重跑探针；探针未跑通则**不宣称**兼容已验收。

旧稿曾引用本机安装包内的上游表（`lib/dist/extraction/grammars.js`）——**未证实（沿用旧稿勘察记录）**，本次不得作为产品运行依赖，也不得复制整表或生成 `extensions` 配置。

**判定口径细则**：

- 只按**后缀**做元数据判定，不读文件内容、不推断语言或符号、不看仓库类型或标签；后缀按最后一个扩展名做 ASCII 不区分大小写匹配。
- `.yaml/.yml/.xml/.properties/.tf/.tfvars/.tofu`、`.json/.md/.toml/.txt/.csv/.html/.htm` **不算**源码（属配置/文档）。是否产出符号不是判据（上游对 yaml/xml 属 file-level-only，不据此断言「无符号」）。
- 符号链接（目录与文件）与非普通文件**都不算**源码；目录打开不跟随符号链接。
- 排除口径**复用**既有 11 条内建规则（`src/product/logical_codebase/aggregate_index/exclude.rs` 的 `BUILTIN_EXCLUDES`）与 `codegraph.json` 生成口径，**不另写副本**；普通遍历剪枝 `.git/.aria/node_modules/target/build/dist/.worktrees`。其中 4 条非目录固定名模式（`**/.env`、`**/.env.*`、`**/*credential*`、`**/*secret*`）在**目录条目**上按名字匹配剪枝（`credentials/`、`secret-notes/` 这类目录不进入），不得改成文件级规则或另行放宽。
- `.worktrees/` 是**独立的第二段诊断**：普通路径查完且无源码时才查 `.worktrees/` 子树；两段都无候选且都完成 ⇒ `NoSource`；在 `.worktrees/` 找到候选 ⇒ `SourceOnlyInWorktrees`（失败，不是跳过）。`.worktrees` 入口本身是符号链接或不存在 ⇒ 该段「无候选且完成」，不跟随、不算失败。
- **可见范围与 codegraph 同口径（G1 · 方向性更正，取代旧稿「不模拟 `.gitignore`」）**：判定「该成员有没有源码」时，源码候选**只取 git 可见范围**——在该成员**主工作目录**内执行 `git ls-files --cached --others --exclude-standard`（已跟踪的文件 + 未跟踪且未被忽略的文件），与 Aria 源码后缀集求交，再扣除 Aria 自身排除（与既有内建排除规则表同源、不另写副本；与本条直接相关的典型项为 `build/`、`dist/`、`node_modules/`、`.worktrees/`、`.aria/`、`.env*`、`*credential*`、`*secret*`）。**被 `.gitignore` 忽略的文件不算源码**：既不判「漏收」失败，该成员按「无源码成员」走跳过分支（只跳过代码检索，不移除成员、不阻断激活）。旧稿「普通路径有源码但被 ignore 仍是真漏收」**已作废**——它会把「codegraph 永不收录、git 也看不见」的文件算成漏收，产出「任何含被忽略源码文件的仓永远建不出索引」的死锁（同步多少次都没用）。
- **第二段诊断不受可见范围收窄**：上条只界定**普通路径**的「有没有源码」；`.worktrees/` 仍是 D2 既有的独立第二段诊断（物理扫描 `.worktrees/` 子树），它的用途正是报告「源码存在但被 Aria 排除」这一事实，失败码 `aggregate_index_member_source_excluded` 的语义不变。`.worktrees/` 条目在可见范围求交时按 Aria 自身排除扣掉，因此它**不计入**普通路径的源码判定。
- **git 侧异常按「无法确认」**：主工作目录不是 git 仓、git 不可用、命令失败或超时 ⇒ 结论为 `Unknown`（无法确认），MUST NOT 当成「无源码」。可见清单里可能含工具自身生成的非源码条目（如 `.codegraph/.gitignore`），按后缀与排除规则过滤即可，不构成异常。
- **有界性**：根深度 0、上限深度 32（深度上限只约束物理扫描段，即 `.worktrees/` 第二段诊断）；可见清单的枚举与后缀过滤同样计入预算——每成员最多 20,000 个条目、单次验收整批 100,000 条目、**2 秒**（含第二段诊断，共享同一计数与 deadline）；命中普通路径源码即停；不展开全量路径、不读内容。盘检允许且**必须**调用只读的 `git ls-files --cached --others --exclude-standard` 取可见范围，MUST NOT 起 CodeGraph 进程、MUST NOT 写 git 状态或改动工作区。超限/超时/权限不足/git 侧失败 ⇒ `Unknown`。
- 扫描期间已证实普通路径有源码时，无需继续收集无关错误；**仅部分完成且无正证据时必须返回 `Unknown`**（不能把「没查完」当「没有源码」）。
- 交叉验收入口要能为测试注入时钟/取消信号与预算：聚合内部函数现成地各自新建预算，测试不得靠真实 sleep；实现时把预算/时限作为可注入参数（详见 `tasks.md` 的 T01 验收要点）。

### G0 实测事实（可见范围口径，2026-11-18）

**结论**：被 `.gitignore` 忽略的文件，对 **git 与 codegraph 同时「不存在」**——codegraph 永不收录它，因此增量同步也救不回来。凡是「codegraph 收录范围」的口径判定（有无源码、真漏收）都必须与 git 可见范围同口径，否则会出现「含被忽略源码文件的仓永远建不出索引」的死锁。

**实测环境**：本机 `codegraph` **1.6.3**（部署兼容区间 `>=1.6.0, <2.0.0` 的下沿实测点）；临时 git 仓，产物只落 `/tmp`。

**可复现命令与结果**（本轮已实测复核）：

```bash
T=$(mktemp -d /tmp/cg-vis-XXXX); cd "$T"
git init -q .
printf 'sandbox/\n' > .gitignore
mkdir -p src sandbox
printf 'export const a = 1;\n' > src/tracked.ts     # 已跟踪
printf 'export const u = 3;\n' > src/untracked.ts   # 未跟踪、未被忽略
printf 'export const b = 2;\n' > sandbox/ignored.ts  # 被 .gitignore 忽略
git add .gitignore src/tracked.ts
git -c user.email=a@b -c user.name=t commit -qm init

codegraph init .
codegraph files
git ls-files --cached --others --exclude-standard
git status --porcelain
```

- `codegraph files` ⇒ 收录 `src/tracked.ts` 与 `src/untracked.ts`（**未跟踪但未被忽略的文件同样被收录**），**未收录** `sandbox/ignored.ts`。
- `git ls-files --cached --others --exclude-standard` ⇒ 同一可见集合（已跟踪 + 未跟踪且未被忽略），`sandbox/ignored.ts` 不在其中；可见清单里另含工具自身生成的非源码条目 `.codegraph/.gitignore`。
- `git status --porcelain` ⇒ 仅 `?? src/untracked.ts`（被忽略文件**不参与** dirty 判定）。

**该事实对契约的三处约束**：

1. 盘检的「有没有源码」判定**只取 git 可见范围**（G1，见上）；
2. 「真漏收」只在 git 可见范围内成立，被忽略文件不判漏收、不判失败（G3）；
3. 「上次与本次都无未提交修改」以 `git status --porcelain` 为准，被忽略文件不参与（G4）。

**未证实**：`.worktrees/` 第二段诊断是否也应收窄到 git 可见范围——本轮**不做收窄**（保持 D2 既有物理扫描语义），因为它的用途正是报告「源码存在但被 Aria 排除」；该边界的最终形态留给实施时验证 `.worktrees/` 条目在 `git ls-files --others --exclude-standard` 下的实际输出后再定，本次只冻结「普通路径按 git 可见范围」这一条。

## D3 · 覆盖验收四态与错误码取舍（十个失败码 + 一个告警码）

**四态与出口**：

| 盘检结论 | 事实 | 出口 | 稳定码 |
|---|---|---|---|
| `SourceInNormalPath` | 普通路径**git 可见范围内**有 Aria 源码，索引未收录 | 失败，不发布新 active | **复用** `aggregate_index_member_coverage_failed` |
| `SourceOnlyInWorktrees` | 源码只在被排除的 `.worktrees/` | 失败，不发布新 active | 新增 `aggregate_index_member_source_excluded` |
| `Unknown` | 权限/超时/超限/IO 失败，无法确认 | 失败，不发布新 active | 新增 `aggregate_index_source_scan_incomplete` |
| `NoSource` | 两段范围都确认无源码 | **跳过该成员**（非阻断告警）；若因此没有任何被覆盖成员 ⇒ 失败 | 新增告警码 `aggregate_index_member_source_less`；无被覆盖成员时用新增 `aggregate_index_all_members_source_less` |

**失败主码优先级**：普通路径漏收 > `.worktrees/` 独有源码 > 盘检无法确认；仅当不存在上述阻断且全部成员确认无源码时，才用全员无源码码。按 manifest 成员顺序点名同类全部成员；一条失败文案里不得把不同类型的成员都说成漏收。

**码集（与 `REQ-BOOT-10` / `REQ-LAYOUT-04` 严格相等）**：布局失败面必须完整三段的码共**十个**——既有七码（`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`）加本条三个补充失败码。告警码 `aggregate_index_member_source_less` **不在**失败码集合内，MUST NOT 以失败三段渲染。

（`REQ-BOOT-10` 的 requirement 标题仍写作「七个对外稳定码」——这是与现行规格做 delta 匹配所需的既有标题，正文登记的码集已扩为十个；不要为了正名去改标题，否则 delta 不再匹配。）

**为什么这四个码不能复用既有码**（登记处：`non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10`，唯一来源）：

- `aggregate_index_member_source_less`（**告警码，非 FailureCopy**）：既有码表里没有「已确认无源码、可继续」的语义。`aggregate_index_member_coverage_failed` 是**阻断**码且成品文案是「成员仓里没有可索引的代码文件 / 把真实源码放进成员仓」——用它表达「跳过、可以继续」既会阻断激活，又会把用户没做错的事说成用户错误。跳过是成功世代里的一条**告警**，须与失败封套隔离（`AggregateIndexActiveResponse.failure_copy` 与 `evidence_copy` 已按后端级互斥），不分配 `FailureTriage`。
- `aggregate_index_member_source_excluded`：既有码都会断言「索引里没有该成员的可索引文件」并给出「把源码放进成员仓」的动作，而本事实是**磁盘上有源码、只是位置在开发工作树**，动作是「把源码合入成员主 checkout 的普通路径」。复用会让用户去改忽略规则，而真正要做的是换源码位置。
- `aggregate_index_source_scan_incomplete`：本事实是**证据不足**。复用 `aggregate_index_member_coverage_failed` 会断言「没有源码」——正是本变更禁止的「无法确认当无源码」；复用通用 `aggregate_index_io` 会丢掉「不能确认」这一关键限定，且该码的处置动作（换机器/重试 IO）不等于「检查主检出可读性与扫描范围」。
- `aggregate_index_all_members_source_less`：与告警码的区别是**后果不同**——此处没有任何被覆盖成员，索引不可激活，必须阻断并给出「加入含源码的成员」的动作。同一码同时充当告警与阻断失败会破坏告警/失败互斥。

**第六条规则依据（唯一措辞）**：`LAYOUT_RULES` 的第六条 `rule_zh` 改为「**聚合索引至少需要一个已覆盖成员；确认无源码的成员可以跳过**」。该句 MUST NOT 两处各写一版：与第六条相关的四个失败码（覆盖失败码双变体 + 三个补充码）的 `reason_zh` 都包含它；既有断言 `failure_copy.rs` 的 `layout_rules_cover_seven_external_codes` 改为「十个失败码」的集合断言并保留逐码「`reason_zh` 含所属 `rule_zh`」的强度，**不得放宽测试迁就实现**。

**漏收（`SourceInNormalPath`）为何复用 `aggregate_index_member_coverage_failed`**：既有码的机器事实就是「该成员在索引里没有任何可索引文件」，与漏收事实一致；码本身已登记、已被 Bootstrap 投影消费、已被 `failure_copy.rs` 的 `subject_from_message` 解析点名（抽 `included members [...]` 并用顿号连接成员名）。为不新增码，**同一码在唯一码表内登记两个成品变体**，按持久化失败事实里的机器事实前缀选择：

- 旧记录前缀 `no indexable code file under included members [...]` ⇒ 保留既有成品（只说「覆盖不足」，**不编造**磁盘有源码的证据；事实句与动作句逐字保留，只把规则依据句换成第六条新措辞）；
- 新事实前缀 `source found on disk; no indexed files under included members [...]` ⇒ 新成品（点名漏收成员、说明「磁盘上有源码但索引没收录」、指引检查成员路径与忽略规则后重建）。

两个变体都登记在 `failure_copy_table.inc.rs` 这**同一张表**里，不另建文案表；新事实**不得**渲染成旧成品（会把用户的忽略规则问题说成「没放源码」），旧记录**不得**渲染成新成品（会编造磁盘证据）。既有 `subject_from_message` 的点名解析对新旧两种前缀都成立（都以 `included members [` 结尾），不重做三段 UI。**变体只挑模板**：归因与码表查找仍用 canonical 码（`failure_copy.rs` 的 `canonical_key`），`machine_line` 仍写原始稳定码——内部变体键（如 `.disk_source`）MUST NOT 替换 canonical key。

**G1 之后这段文案的边界**：漏收事实现在**只在 git 可见范围内成立**（被 `.gitignore` 忽略的文件不算源码、不判漏收），因此成品文案**保持既有登记文本不变**（文案单一来源在 `failure_copy_table.inc.rs`，本变更不改码表、不新增第二份文案）；文案里「检查成员路径与忽略规则」这句是**通用排查指引**，不是「你的源码被忽略了」的断言。

**失败封套的解析只有一个出口（冷启动必须接住）**：持久化失败封套（`aggregate_index_failed:<code>: <message>` / `aggregate_index_degraded:<code>: <message>`）到「内层稳定码 + 原始事实 + 三段成品」的解析 SHALL 放在**产品层**（`src/product/logical_codebase/failure_copy`），由 HTTP 出口（`src/web/handlers/aggregate_index.rs` 的 `failure_copy_from_warning`，改造后委托给该产品层入口）与**冷启动投影**（`src/product/logical_codebase/bootstrap_projector.inc.rs` 中 `AggregateIndexStatus::Failed` 分支决定 `reason_code` 的那段）共用。

冷启动现在只用裸前缀特判两个码（`warning.contains("aggregate_index_layout_unsupported")` / `contains("aggregate_index_member_coverage_failed")`），其余一律落 `aggregate_index_failed`——这是本变更必须修的：冷启动路径传进来的是**完整封套**，前缀判断在新码上必然不成立，三个补充失败码会退化成通用码。要求：

- 两出口都经同一解析入口取「内层码 + 消息」，再走同一码表成品；
- 冷启动的 `BootstrapFailure.reason_code` 写**内层稳定码**（不是 `aggregate_index_failed`）；
- HTTP 与冷启动对同一封套输出同码同文案（验收断言直接对比两处输出，不只对比码名）。

## D4 · durable 形状与向前兼容（无 `schema_version` ⇒ 只能靠 serde default）

**形状。** `AggregateIndexMemberSnapshot`（`src/product/logical_codebase/aggregate_index/types.rs`）新增一个可选字段：

```
skip_reason: Option<AggregateIndexMemberSkipReason>   // wire: "no_source"
```

序列化带 `#[serde(default, skip_serializing_if = "Option::is_none")]`；枚举当前只有一个 wire 值 `no_source`。**只改 `types.rs` 的这一处定义**：既有 `AggregateIndexMemberSnapshot::indexed(...)` 构造函数（五参）与 `current()` 的采集路径都不需要改签名——`indexed()` 里给出 `skip_reason: None` 即可（`src/product/logical_codebase/aggregate_index/snapshot.rs` 的 `current()` 调的就是 `indexed()`，不是 struct literal）。

**语义**：`Some(no_source)` = 该成员经盘检**确认无源码**，本次未纳入代码检索范围；`None` = 无跳过事实（覆盖，或旧记录）。跳过成员**仍然出现在快照里**。

**`included` 语义不变。** `included` 保持既有原义（该成员出现在这次索引快照里），被跳过的成员**不改** `included`；跳过与否只由 `skip_reason` 表达。（早期草稿曾提议把跳过成员的 `included` 写成 `false`，**本变更作废该提议**：用户已锁定「不改 `included` 的既有语义」，且 `included` 还被 `planning_context_resolver.rs` 的指纹构造当作 `available` 使用，写 `false` 会连带把跳过成员变成不可用目标仓。）

**向前兼容。** 记录形状**没有 `schema_version`**（`AggregateIndexRecord` 无该字段），因此兼容只能靠 serde 的缺省语义：

- 老记录（无 `skip_reason` 字段）**必须可读** ⇒ 字段必须 `#[serde(default)]`，缺省 `None`；
- 新记录写盘后老代码读会忽略未知字段（serde 默认行为），不需要回填历史、不需要迁移脚本、不引入第二份 durable 成员列表；
- 合法组合只有两种：`skip_reason = None`（覆盖）与 `skip_reason = Some(no_source)`（已确认无源码）。失败/未验收世代的默认快照、未知字段组合**不得**被投影为跳过证据。

**写入时点。** 首次采集、`Building` 与失败的命令前后快照照旧；**只有验收成功**才在同代快照上标记 `skip_reason`，随后 `replace_active` 并更新 manifest 指针；失败世代不写跳过事实，last-known-good 保持**它自己那一代**的跳过事实（本次失败的猜测不写进旧代）。

**漂移与巡检。** 跳过成员与其它成员一样参与 membership revision、成员集合、主 checkout 身份、revision、dirty 的漂移判据；**不**拿 `skip_reason` 与只采 Git 的当前快照直接比大小（否则恒 stale）。本变更**不改**全局漂移函数（`freshness.rs` 的 `member_evidence_drifted` 仍只比 revision / dirty / 成员数）；「不能确认没变」是 Story 路径自己的判断（D5）。

## D5 · Story 前更新：专用入口、三分支与失败语义

**专用入口（D-2 收敛）。** 刷新 SHALL 是 **Story 生成专用**的入口/模式：

- Story 生成（`src/web/handlers/lifecycle.rs` 中构造 `AggregateStorySpecScope` 的那条路径）SHALL 走新的「必须先确保最新」入口（例如 `PlanningContextResolver` 上新增 `build_with_verified_index`，名字由实施定）；
- `build_with_fresh_index` **保持现语义**，继续服务 Design 生成（`lifecycle.rs` 的 `AggregateDesignSpecScope` 路径）与工作区上下文读取（`src/web/workspace_context/builder.rs` 的 `Story | Design | WorkItemPlan` 分支）：stale 才按需同步、degraded 只加告警、绝不停机；
- 两个入口 MUST NOT 互相借用语义：Story 不停机不成立，Design/WorkItemPlan 停机也不成立。

**不触发的动作**：创建 Issue、勾选成员、刷新页面、同一次生成中的后续对话，都**不**另跑一次更新；**不新增后台定时轮询**（既有兜底口径不变，本次不扩大）。

**三个分支**：

1. **无可用索引** ⇒ 走既有**首次建库**流程（登记与主工作目录已就绪时），不是每新建一个 Issue 就从头建库。这里的「无可用索引」精确指**没有任何可读世代**（`assess()` 走 `active_required` 报 `aggregate_index_active_missing` 的情形，`src/product/logical_codebase/aggregate_index/store.rs`）；记录**读坏/归属不明/彼此矛盾**是另一回事，SHALL 按既有失败规则报错，MUST NOT 静默首建。
2. **能确认没变** ⇒ 复用当前这一版。判据（全部满足才算「能确认」）：成员集合未变、主工作目录（唯一 available Main checkout 的身份）未变、Git 提交未变、**上次与本次都无未提交修改**。任一成员上次或本次 `dirty = true` ⇒ **不能确认没变**（工作区内容可能在两次检查之间被改过，revision 不变也说明不了文件没变）。**「无未提交修改」以 `git status --porcelain` 为准**（既有快照采集即用 `git status --porcelain=v1`，`src/product/logical_codebase/aggregate_index/snapshot.rs` 的成员快照采集路径）：**被 `.gitignore` 忽略的文件不参与 dirty 判定**，因此既不会让分支 2 失效，也不会让分支 3 被无谓触发。**当前世代含跳过成员不再收窄分支 2**（G2：旧稿的「当前世代不含跳过成员」附加条件已删除）。
3. **有变化或不能确认没变** ⇒ 实际做**增量同步**（`codegraph sync` 语义）并**重新验收**（走 D1 的同一套判定）。归入本分支的情形：成员集合/主工作目录/提交变了、**上次或本次显示有未提交修改**（两次都脏时即使 revision 相同也必须同步，否则同一 dirty 布尔会掩盖两次检查之间的改动）。上次更新失败留下的旧版**不能**直接当成最新版：先尝试恢复并验收。

**为什么分支 2 不再要求「当前世代不含跳过成员」（G2 · 方向性更正）**：旧稿把「含跳过成员」收窄进分支 3，理由是「被 `.gitignore` 隐藏的新增源码对 revision/dirty 完全不可见」。G0 实测（见 D2「G0 实测事实」）推翻了这条前提：**被 `.gitignore` 忽略的文件，codegraph 永不收录**（增量同步也救不回来），因此它既不构成漏收、也不构成「不能确认没变」——它对 codegraph 收录范围**根本不可见**，谈不上「变了」。git 可见范围内的任何变化已经被「脏 ⇒ 同步」与「提交/成员集合变化 ⇒ 同步」两条覆盖。因此分支 2 的四项判据（成员集合 / 主工作目录 / Git 提交 / 上次与本次都无未提交修改）已经足够，**删除**「当前世代不含跳过成员」这一附加条件。

**代价与收益**：旧稿的收窄条件会让「含跳过成员的世代」每次 Story 都强制多跑一次增量同步（实测 3.4s 量级，临时夹具）；撤销后该副作用消失——干净且未变的世代（含跳过成员）直接复用，不再无谓同步。**被忽略的文件既不判漏收、也不参与变更检测**（D2 的 G1 与本节的 G4 一致）。

**并发**：若初始化或另一次重建正在更新同一份索引，就**等待并复核结果**，不并行写两份（既有 single-writer 互斥）。**锁只串行化写入，不替调用方复核**：拿到锁/拿到结果之后，成功与否 SHALL 以**本次实际验收并发布的记录**为准，MUST NOT 用仍带 dirty 的重新评估结果去否定一次刚刚成功的验收（否则「同步成功但因为工作区仍脏被判失败」会误停 AI）。

**失败语义**：更新或验收失败 ⇒ **停在本次生成之前**（AI 不启动），说明原因并允许重试；**保留已有 Issue、Story 与上次成功的索引**；**不得拿旧索引冒充最新**。「最新」指这次检查或同步时**本地成员主工作目录**里的代码，**不主动拉取远端**，也不保证生成过程中后来保存的改动已经收录。

**失败面（不得落成通用 500）**：刷新失败 SHALL 携带稳定码与同源三段成品。稳定码优先取失败封套内层码（覆盖失败 / 源码仅在工作树 / 盘检无法确认 / 全员无源码等已登记码）；解析不出时用已登记的 `aggregate_index_unavailable`。生成入口 SHALL 显式映射成带 `details.copy` 的运行时错误——`src/web/handlers/support_parts/product_store_error.inc.rs` 的兜底分支会把 `ProductStoreError::Io` 落成通用 500「product store operation failed」，而 `planning_context_resolver.rs` 的 `map_index_error` 正是把它包成 `ProductStoreError::Io(format!("aggregate_index_unavailable: {error}"))`，所以「只包一层 Io」不满足要求。

**记录**：通过后记录这份 Story 参考的索引版本（沿用既有 planning snapshot 的 `aggregate_index_id` / `index_revision` 字段）。

**重新验收的触发点（不做自动发现）**：成员集合 / `membership_revision` 变化、成员 revision/dirty 漂移、手动重建、本节的第三分支（成员/主工作目录/提交变化，或上次或本次有未提交修改）。除此之外没有发现「跳过成员后来出现源码」的通路；两次验收之间新增的源码要等下一次触发才被发现，这是已接受的时间窗。**「当前世代含跳过成员」不再是触发点**（G2）。

**取舍论证（实测耗时）**：本机临时夹具（`/tmp` 隔离，**不是用户现场**）：69 个成员仓 / 1380 个源码文件 / 约 23 MB。

| 动作 | 耗时 |
|---|---|
| 首次建库（`init` + 全量验收） | **24.6s** |
| 未变检查（成员/主工作目录/提交/脏标记比对） | **0.24s** |
| 一仓改一文件后：检查 + 增量同步 + 重新验收 | **3.4s** |
| 强制 `codegraph sync`（命令本身） | **2.0–10.6s** |
| 已有库上走「重建索引」后端路径 | **1.8s** |

结论：**每次检查、变了才同步**。每次 Story 前必跑全量建库（24.6s 量级）不可接受；每次必跑增量同步（2.0–10.6s 量级，且 69 仓现场更慢）也不必要；而「未变检查」只要 0.24s 量级，可以每次都跑。**「上次与本次都显示有未提交修改」时必须同步**（3.4s 量级），因为 revision 相同不代表文件没变；**两次都脏时即使提交号相同也必须同步**，否则同一个 dirty 布尔会掩盖两次检查之间的改动。被 `.gitignore` 忽略的文件**不参与**变更检测（G4，`git status --porcelain` 口径），因此不会因为它们而触发同步。数字来自临时夹具，**不得**当成用户现场的规模承诺；用户现场规模更大时按比例放大，但分支结构不变。

## D6 · 配置仓目标的证据链语义：200 + 显式「未检索」

判定顺序 SHALL 是：**凭据 → ACL → 输入校验 → 钉住记录校验 → 再**按钉住世代判断目标成员是否被跳过 → 普通目标再取「授权范围 ∩ 钉住世代已覆盖成员」的交集。跳过支路 MUST NOT 进入库目录守卫（`codegraph_library_present`）、MUST NOT 建查询、MUST NOT 调 CLI；普通支路的既有守卫保持。

- 跳过支路：HTTP **200**，回包带 `not_searched: {code, message_zh}`（`code` 用同一个 `aggregate_index_member_source_less`；`message_zh` 与卡片/成员行同源），`text` 携带同一说明（例如「目标仓无源码，未检索聚合索引；请在目标工作区直接读取配置或文档。」），`truncated = false`，配额仍按既有预算规则核算（实际回包文本算），审计 `hit_count = 0`、空 `snapshot_refs`，并留可读的「未检索原因」；
- **审计是纯加法**：既有审计字段与判定口径不变，只为未检索增加一个可选原因字段，唯一职责是解释「为什么没检索」；不得把原「查询无命中」改写成「未检索」；
- **跳过 ≠ 查询无命中 ≠ ACL 放行**；越界请求、凭据不合法、坏钉住/坏记录仍按既有规则失败，MUST NOT 因目标属跳过成员而返回 200；
- provider 在该仓**已授权的目标工作区**直接读写配置/文档；**不加文件类型拦截、不要求把 JSON/文档纳入索引、不改 ACL/配额/审计口径**。

## D7 · UI 面、提示形状与文案单一来源

**结构化提示（唯一形状）。** 卡片与成员行的跳过提示都是 `{code, message_zh}`：

- `code` = `aggregate_index_member_source_less`；
- `message_zh` = 同源成品模板渲染结果：卡片按成员列表渲染（「已跳过无源码成员：…」并点名），成员行按单个成员渲染（「未纳入代码搜索，可修改配置或文档」）；
- 无跳过事实时字段省略（不返回空串/空对象）；旧响应无该字段 ⇒ 前端不显示提示。

字段名由实施统一（建议两处都用 `source_less_notice`），MUST NOT 一处叫 `notice` 一处叫 `skip_warning`；该字段是 `serde(default, skip_serializing_if = "Option::is_none")` 的可选对象。

**匹配口径（同时适用于卡片与成员列表）。** 提示与成员的关联只用 **(成员 ID, 当前 Main checkout ID)**：

- 成员 ID = 快照 `logical_repository_id`；
- 当前 Main checkout ID = 该成员**当前可用** `CheckoutKind::Main` 检出记录的 `checkout_id`（`src/product/logical_codebase/types.rs` 的 `RepositoryCheckoutRecord` / `CheckoutKind`）；
- MUST NOT 用显示名、别名或裸成员 ID 单独匹配；主 checkout 已更换、当前无可用 Main checkout、记录缺失或世代不同 ⇒ 该成员不出提示。

**卡片从被投影的那一代取（投影一致性）。** `read_active_projection` 目前有两套选择：`degraded_record`（决定投影 `state`）与 `usable_generation`（决定证据面可用性）。跳过提示 MUST 派生自**实际投影的那一条记录**（与 `state` 同源），不得再选一遍；degraded 投影时提示用 degraded 世代的跳过事实，措辞由同名模板产生（必要时前端按既有失败三段的「上次可用索引」语境补语境，但正文仍来自同一模板）。

**三个提示落点**（都只渲染服务端下发的成品文本，前端**不自写中文正文**）：

1. **聚合索引卡片一行**：在索引摘要下方显示「已跳过无源码成员：…」，点名被跳过的成员；不着失败色、不添新按钮、不改既有重建动作。数据来自被投影的那一代；无记录/空列表/旧世代 ⇒ 不显示。
2. **创建 Issue 选成员逐仓标注**：在该成员行标注「未纳入代码搜索，可修改配置或文档」。成员列表 API（`list_logical_codebase_members_for_lc`，入参 `project_id` + `logical_codebase_id`）随成员行返回该结构化对象；**没有对应记录就不猜、不按仓名贴标签**；前端不额外拉聚合索引请求、不跨 LC 对表。
3. **入口目录说明**：三个入口（创建 / 登记 / 聚合初始化）共用既有组件 `web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx`，**改共用文案即可**；本次**不新建服务、不新增只读接口**。规则六的**判定**仍归聚合索引契约，此处只陈述范围。

**选择与提交资格不变**：提示**不是**准入条件。有无提示、有无跳过标记，都不改变：成员是否列出、是否可勾选、是否可提交、是否可作 Issue 目标仓、checkout 与工作区准入、授权判定。

**文案单一来源**：`src/product/logical_codebase/failure_copy_table.inc.rs` 的码表是**唯一**中文成品来源。新增告警成品（跳过提示 / 逐仓标注 / 未检索说明）以**同一个告警码 + 同一模板**登记一次，三处消费面渲染同一模板产出的文本。失败面仍走既有五字段合同（`title_zh` / `reason_zh` / `action_zh` / `triage` / `machine_line`，`REQ-BOOT-11`），告警**不**占用 `FailureCopy`、不分配 `FailureTriage`、不触发失败通知。**不新建第二份中文文案表、不在 Rust 投影 / 通知 / TSX 维护同义文案。**

## D8 · 取舍、残余风险与未证实项

- **判定放在验收层而非 UI 层**：UI 只消费成品，不复算「有没有源码」；避免前后端两套口径。
- **盘检是元数据扫描**：只按后缀、不读内容；源码候选只取 **git 可见范围**（`git ls-files --cached --others --exclude-standard`，只读调用 git，不起 CodeGraph）；代价是保守清单（探针未跑完前）会少认一些语言 —— 方向是「多跳过」而不是「误报错」，探针结论回来后增补后缀即可收窄。
- **超限宁可失败**：扫描超限要求修复可读性或缩小工作树，宁可保留 last-known-good，也不扩大预算临时放行；无法确认绝不等于无源码。
- **「上次与本次都脏」必须同步**：牺牲 3.4s 量级，换取 Story 引用的是当次工作区内容；这是把「不能确认没变」并入第三分支的代价。**「含跳过成员」不再强制同步**（G2 已撤回），否则每次 Story 都要多跑一次增量同步。
- **不拉远端**：「最新」只指本地成员主工作目录；远端更新由用户自己同步后再生成为准。
- **不做自动发现**：跳过成员的源码后来出现时，要等下一次验收/重建才重新判断；时间窗是已接受的残余风险，不用后台扫描或 freshness 盘检来消除。
- **残余风险（已知）**：CodeGraph 已覆盖但内容不足仍由既有验收边界负责；`REQ-BOOT-10` 的标题仍写「七个」（为 delta 匹配保留），以正文的十个失败码为准。
- **残余风险（G1 带来，已接受）**：源码写在被 `.gitignore` 忽略的路径里的成员，会被判为「无源码」并进入跳过告警——**不报漏收**（codegraph 同样不收录它，报漏收是死锁）。用户看到的措辞仍是「确认无源码」口径，**不新增文案**（文案单一来源在 `failure_copy_table.inc.rs`）；「被忽略的源码要不要纳入索引」是用户自己管理 `.gitignore` 的事，本变更不介入。
- **未证实（实施时必须验证，不得在契约里假定）**：
  1. Aria 源码类型清单的**具体后缀集合**（D2 的探针；不阻塞开工）；
  2. 本机 CodeGraph 安装包内上游扩展名表的准确位置与内容（旧稿勘察记录，**未证实**）；
  3. 全员无源码夹具下 `init/sync` 的真实输出形态；
  4. 配置目标的真实 provider 操作（工作区读写配置文件）未于本次运行验证；
  5. 上表耗时来自临时夹具（`/tmp` 隔离），**不是用户现场**。
