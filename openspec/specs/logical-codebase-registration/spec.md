# logical-codebase-registration Specification

## Purpose

以 Project 为逻辑代码库，批量登记同一非 git 公共父目录下的多个真实 git 仓库为成员（attach-only、预检、批次状态、幂等），并定义旧数据迁移与聚合根准入。

## Requirements

### Requirement: 逻辑代码库成员登记（REQ-REG-01）

系统 SHALL 支持将同一非 git 公共父目录下的多个真实 git 仓库登记为 Project（逻辑代码库）成员；每个成员对应一个 `RepositoryRecord`（物理 git 仓，兼容投影）与一个 `CodebaseMemberRecord`（稳定 UUID 逻辑身份 `LogicalRepositoryId`）与一个 `RepositoryCheckoutRecord`（可用 checkout）；聚合根绝不注册为 RepositoryRecord。三层身份映射：`LogicalRepositoryId` →（member 解析）→ `RepositoryCheckoutId` →（物理定位）→ `RepositoryRecord.id` + canonical_path + git_dir_identity。

系统 SHALL 为逻辑代码库建立唯一、可审计的 authority 路由：明确逻辑代码库身份时，成员、checkout、manifest、selection、policy 与 index 只能从该逻辑代码库的权威子树解析；单仓代码库只能从其合法物理仓布局解析。系统 SHALL 禁止按路径猜测、跨逻辑代码库回退或把旧项目级布局静默当作当前 authority。发现旧布局、authority 冲突、路径越界、来源身份不一致或无法唯一解析时，系统 SHALL fail-closed，返回可操作的迁移/核验等待项且不得产生成员或仓库写入。

#### Scenario: 扫描公共父目录登记成员

- **WHEN** 用户提供公共父目录并触发登记
- **THEN** 系统 SHALL 扫描发现全部子 git 仓，逐项校验（canonical path、git root、非嵌套、非重复），并为每个成员创建 CodebaseMemberRecord 与 RepositoryRecord，全程不修改任何仓库内容

#### Scenario: 成员身份与去重

- **WHEN** 同一仓库以不同路径或别名重复提交
- **THEN** 系统 SHALL 按 canonical git-dir/source identity 去重，不产生重复成员，并返回重复原因

#### Scenario: 新逻辑代码库使用唯一 authority

- **WHEN** 用户创建一个没有历史 seed 的逻辑代码库并提交成员登记
- **THEN** 所有后续 selection、policy、checkout 与 aggregate index 引用 SHALL 指向同一逻辑代码库 authority；不得要求用户复制成员文件或先手工建立索引

#### Scenario: 旧布局与 LC authority 冲突

- **WHEN** 请求同时发现项目级历史布局和逻辑代码库子树，且两者的成员或来源身份不一致
- **THEN** 系统 SHALL 返回冲突诊断及迁移/核验操作，保持当前权威事实不变，不猜测路径、不自动合并、不切换读取来源

### Requirement: 登记零 Git 副作用（REQ-REG-02）
登记阶段系统 SHALL NOT 执行任何 `git add`/`git commit`/`git push`，SHALL NOT 操作任何成员主 checkout；该承诺适用于登记阶段（每仓最小指针的受控发布属于受控副作用，见 session-policy-envelope REQ-ENV-07）。

#### Scenario: 登记前后仓库无变化
- **WHEN** 完成 50 仓登记后
- **THEN** 每个成员仓库的 `git status --porcelain`、HEAD、refs、worktree list、`.git/config`、hooks、index 与登记前完全一致；聚合根的 `.aria`/`.codegraph` 允许写入，成员仓内禁止写入运行文件

### Requirement: 批量预检与分类展示（REQ-REG-03）
系统 SHALL 在批量登记提交前执行预检并按类别展示：可登记 / 非 git / 重复 / 嵌套 / 脏仓 / 路径不存在 / 越界，由用户确认后才实际登记；有效项不被无效项阻塞；脏仓默认可登记但标记 `needs_attention`（可经用户显式确认后登记）。

#### Scenario: 混合清单预检
- **WHEN** 提交含有效项与非 git/重复项的 manifest
- **THEN** 预检结果 SHALL 按类别分组展示；用户确认后仅登记有效项与显式确认的脏仓

#### Scenario: 确认后的 TOCTOU 复验
- **WHEN** 用户确认到实际登记之间成员状态可能变化
- **THEN** 系统 SHALL 冻结 preflight revision，登记前对每个成员复验 canonical path 与 git root，变化者标记为需重新确认

### Requirement: 批次状态与幂等（REQ-REG-04）
系统 SHALL 提供批次状态（queued/running/partial_failed/completed/cancelled）与逐项状态（pending/skipped/completed/failed/needs_attention）、失败原因、重试计数与幂等键（project + canonical manifest digest + revision）；支持取消、重启恢复与并发批次仲裁；删除后重导使用稳定 UUID 与 tombstone/source identity 映射，不产生 ID 冲突。

#### Scenario: 相同 manifest 重放
- **WHEN** 同一 manifest 重复提交
- **THEN** 系统 SHALL 通过幂等键识别为同一批次或返回既有批次，不产生重复成员

#### Scenario: 批次中断后重启
- **WHEN** 批次执行中服务重启
- **THEN** 系统 SHALL 从持久化状态恢复，未完成项可重试，已完成项不重跑；成员已创建但 manifest 未写完成时执行补偿

#### Scenario: 删除后重导
- **WHEN** 删除成员后重新导入同一仓库
- **THEN** 系统 SHALL 通过 tombstone/source identity 映射复用或安全重建逻辑身份，不产生 ID 冲突或引用错配

### Requirement: 前端工程差异化（REQ-REG-07）
系统 SHALL 在成员 manifest 支持 `repo_type`（backend/frontend/lib）与前端初始化 profile（package.json/pnpm/Vite 探测），前端成员不套用 Java 六步初始化。

#### Scenario: 登记含前端仓库的 manifest
- **WHEN** manifest 含 repo_type=frontend 的成员
- **THEN** 系统 SHALL 按前端 profile 处理其检测与初始化命令，与 Java 后端差异化

### Requirement: 旧数据迁移与 repo_id 双读双写（REQ-REG-08）
系统 SHALL 提供既有单仓数据向逻辑代码库的迁移：单仓 Project 自动生成默认 member；`IssueRecord.repo_id` 迁移为 focus/primary 投影并在双读双写窗口内保持兼容；提供 feature flag 回退到单仓行为；历史 attempt/worktree/session 在 target 快照缺失时 display-only 或人工处置，不得静默恢复至错误仓库。

#### Scenario: 既有单仓 Project 迁移
- **WHEN** 开启逻辑代码库功能的既有单仓 Project 被访问
- **THEN** 系统 SHALL 自动为其生成默认 member 与投影，旧 Issue/Story/Design/WorkItem 均可读，不破坏既有数据

#### Scenario: feature flag 关闭回退
- **WHEN** 逻辑代码库 feature flag 关闭
- **THEN** 系统 SHALL 回退到单仓行为；已生成的多仓 Work Item/attempt 标记为兼容投影或阻塞，不静默改写到错误仓库

### Requirement: 聚合根目录所有权与准入（REQ-REG-09）

系统 SHALL 在登记前对聚合根执行准入 preflight：canonical **非 git 根**、成员必须位于其下、拒绝路径越界 / symlink 逃逸 / 嵌套 worktree、**同一目录不得被第二个逻辑代码库占用（跨项目亦然；同一逻辑代码库自身的复验与重跑不在此列，软删墓碑期间仍占用该根）**。

preflight MUST NOT 因聚合根已存在 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` / `.mcp.json` / `.gitignore` 而拒绝登记 —— 这些是 Cadence-skills 初始化的正常产物，且登记本身不写任何文件。冲突时返回稳定错误码并阻塞。

#### Scenario: 聚合根准入失败

- **WHEN** 公共父目录为 git super-repo 或包含非成员目录/凭据/构建产物
- **THEN** 系统 SHALL 拒绝登记并返回分类错误（准入 preflight 范围限于聚合根属性校验；索引范围构造见 REQ-IND-02）

#### Scenario: 已初始化过的聚合根允许登记

- **WHEN** 聚合根目录中已存在 `AGENTS.md`、`CLAUDE.md`、`.claude/rules/language.md`、`.mcp.json` 与 `.gitignore`
- **THEN** 登记预检 SHALL 通过并正常返回候选成员分类；MUST NOT 返回 `aggregate_root_ownership_conflict`

#### Scenario: 同一目录的第二个逻辑代码库被拒绝

- **WHEN** 某聚合根已被一个存活或软删（墓碑）的逻辑代码库占用，另一个逻辑代码库（无论同项目与否）尝试以该目录为聚合根登记
- **THEN** 系统 SHALL 拒绝并返回 `aggregate_root_overlap`，错误信息含被占用根与占用方身份

#### Scenario: 同一逻辑代码库的复验不被自身拦掉

- **WHEN** 同一条逻辑代码库对其既有聚合根重跑预检或聚合初始化复验
- **THEN** 系统 SHALL 通过；MUST NOT 把自身 manifest 判为重叠

### Requirement: 登记生产 HTTP 入口

登记批次能力通过生产 HTTP 端点可用：预检、提交、查询、恢复、取消，全部为同步短操作（请求内完成并返回最终或部分结果）。

#### Scenario: 预检冻结快照
- **WHEN** 调用登记预检端点提交聚合根与候选路径
- **THEN** 服务端先执行聚合根准入校验（非 git 根、成员越界、symlink 逃逸、嵌套 worktree、根所有权冲突，返回对应 aggregate_root_* 稳定错误码），再执行候选分类；预检结果（含每候选的路径、git 根、来源身份、预检修订号）作为冻结快照持久化，返回 preflight_id；快照过期（24h）或不存在时提交返回 404 registration_preflight_not_found

#### Scenario: 同步提交
- **WHEN** 调用提交端点携带 preflight_id 与确认路径列表
- **THEN** 服务端以冻结快照重建确认输入并同步执行 TOCTOU 复验与逐项成员登记，返回批次最终状态（completed 或 partial_failed）与单项结果；登记期间成员主 checkout 无 git 写副作用

#### Scenario: 漂移语义
- **WHEN** 提交或恢复时某成员仅预检修订号变化
- **THEN** 该项标记 needs_attention 并跳过，批次继续
- **WHEN** 提交或恢复时某成员路径、git 根或来源身份变化
- **THEN** 整批中止并返回 409 registration_batch_conflict

#### Scenario: manifest 首批原子创建与 root 一致性
- **WHEN** 提交时 project 尚无 manifest
- **THEN** 以提交的聚合根为 provider_context_root 原子创建 manifest 并登记首批成员
- **WHEN** 提交的聚合根与既有 manifest 的 provider_context_root 不一致
- **THEN** 返回 409 aggregate_root_mismatch，不产生任何成员变更

#### Scenario: 恢复与取消
- **WHEN** 对 partial_failed 批次调用 resume
- **THEN** 同步重跑未完成项，漂移语义同上
- **WHEN** 对 Queued/PartialFailed 批次调用 cancel
- **THEN** 批次标记终态 cancelled；对已终态批次调用返回 409 registration_batch_not_cancelable

### Requirement: 登记端点的前端入口

多仓 project 的逻辑代码库页必须提供登记向导入口，覆盖预检分类展示、确认与同步提交。

#### Scenario: 登记向导
- **WHEN** 用户在多仓 project 的逻辑代码库页发起成员登记
- **THEN** 向导引导：填写聚合根 → 展示候选分类（eligible/non_git/duplicate/nested/needs_attention/missing/outside_root）→ 勾选确认（勾选 needs_attention 项视为显式确认）→ 同步提交（带 loading）→ 展示批次结果与单项状态，partial_failed 可恢复

### Requirement: 预检候选自动发现

登记预检必须支持以聚合根自动发现候选：当请求携带 auto_discover=true 时，服务端以聚合根下直接子目录中的 git 仓库为候选执行预检分类，无需调用方逐一提供候选路径；auto_discover 缺省或 false 时行为不变。

#### Scenario: 自动发现候选
- **WHEN** 调用预检端点携带 aggregate_root 与 auto_discover=true
- **THEN** 服务端扫描聚合根直接子目录，对其中发现的 git 仓库执行预检分类并返回带 class 的候选列表（快照照常冻结持久化）

#### Scenario: 默认行为兼容
- **WHEN** 调用预检端点未携带 auto_discover（或 false）
- **THEN** 服务端按 candidate_paths 显式候选执行预检，行为与既有实现一致

### Requirement: 登记向导候选自动发现交互

登记向导在用户填入聚合根后必须自动拉取候选并以勾选列表展示，不要求用户手工输入候选路径。

#### Scenario: 向导自动展示候选
- **WHEN** 用户在向导中填入聚合根并进入预检步骤
- **THEN** 前端以 auto_discover=true 请求预检，展示候选列表与分类徽标；eligible 候选默认勾选，needs_attention 候选需用户显式勾选（视为确认），其余分类默认不勾选

#### Scenario: 手填兜底
- **WHEN** 自动发现结果为空或用户选择手工模式
- **THEN** 向导仍允许手工输入候选路径列表（既有交互保留）

### Requirement: 冷启动登记的可重入步骤与操作面（REQ-REG-10）

系统 SHALL 将全新逻辑代码库的首次可用过程暴露为可观察、可重入的步骤事实，至少区分 identity、manifest/checkout、rules/policy、member index 与 aggregate index active；每个步骤 SHALL 具有未开始、进行中、已完成或失败/等待状态、稳定对象身份、checkpoint/失败原因和允许的准备、继续、重试或核验操作。操作必须绑定目标与预期版本，重复同一命令键 SHALL 返回同一结果，过期或错误对象 SHALL fail-closed。GET/只读投影 SHALL 只返回状态、缺失材料、已完成 checkpoint 与允许操作，MUST NOT 启动 provider、checkout、索引或其他副作用。

#### Scenario: 全新 LC 无手工铺底完成首建

- **WHEN** 用户从产品面创建一个没有 active index、规则或 selection seed 的逻辑代码库并触发首建
- **THEN** 系统 SHALL 按 identity→manifest/checkout→rules/policy→member index→aggregate index active 的依赖顺序自动推进；每步完成事实可查询，最终进入 PlanningReady，成员仓登记不产生 Git 写副作用

#### Scenario: 首建中断后继续

- **WHEN** 首建在任一步骤产生可诊断失败或用户关闭页面后再次查询
- **THEN** 系统 SHALL 保留已完成步骤与失败证据，展示针对该步骤的继续/重试/准备操作；用户执行允许操作后 SHALL 从 checkpoint 继续，已完成步骤与 provider turn 不重复执行

#### Scenario: 只读查询不触发首建

- **WHEN** 客户端反复调用逻辑代码库详情、成员、policy 或 aggregate index 状态查询
- **THEN** 系统 SHALL 只返回 durable 投影和允许动作，不启动 provider、不创建 index、不修改 manifest/checkout/rules 或步骤状态

### Requirement: Failed identity journal 的产品化修复（REQ-REG-11）

系统 SHALL 提供不依赖普通成员列表或成功身份解析的 repair/diagnostic 入口，允许用户读取 Failed identity journal 的 source digest、已完成键、read mode、当前映射和冲突详情。对 digest 可证明一致的安全前缀，系统 MAY 提供继续 repair 操作；mapping 冲突 SHALL 要求用户提交明确 mapping 并执行再次核验。repair 成功前 SHALL 保持原 journal 与权威 JSON 不变、禁止删除 journal、禁止直接改数据、禁止切换到未确认的成员或来源身份；未完成裁决 SHALL 保持等待并可重复读取。

#### Scenario: Failed journal 安全前缀继续

- **WHEN** 用户打开一个 Failed identity journal，source digest 与安全前缀核验一致且不存在 mapping 冲突
- **THEN** 产品 SHALL 展示已完成键和可继续范围；用户确认继续后保留原失败记录并追加可审计 repair 事实，随后回到原登记/读取链

#### Scenario: mapping 冲突须人工确认

- **WHEN** Failed journal 检测到多个候选 mapping 或 source digest 不一致
- **THEN** 产品 SHALL 展示候选、digest 和影响范围，要求用户提交 mapping 并重新核验；用户确认前 SHALL 不切换读取 authority、不产生成员变更、不启动 provider

### Requirement: 规则与 capability 缺失的准入预检（REQ-REG-12）

逻辑代码库首次登记、root recipe、索引或 provider 准入前，系统 SHALL 预检实际 provider 将消费的 canonical LC root 规则材料、聚合 policy 引用、Codex/Kimi provider trust 前置条件和 capability/gateway 条件。成员规则门 SHALL 按普通根消费者、有效 AggregateBootstrap 与 LC SingleCandidate（SC）的实际消费来源区分。必需材料缺失、不一致、过期、信任登记失败或 provider 能力不满足时，系统 SHALL 在依赖该材料的真实 provider 启动前返回可诊断等待项，通知原因、目标、材料、外部副作用和准备/重试/撤销操作；准备完成后 SHALL 回到原步骤继续。系统 MUST NOT 通过写入 capability 记录、复制或自动生成成员规则、绕过 gateway、改用备用 cwd 或以未登记 trust 将缺失能力视为可用。

#### Scenario: 成员规则缺失

- **WHEN** 普通请求只消费 canonical LC root 的材料，根规则、已发布政策与最终 policy receipt 经既有准入校验有效，而全部或部分成员仓的 `.claude/rules/language.md` 缺失
- **THEN** 系统 SHALL 使用已核验的根材料继续，不将成员缺失当作根缺失；`missing_member_rules` SHALL 继续作为非阻断审计事实记录，不读取不消费的成员文件正文、不复制或生成成员规则；若只有这些非必需成员材料缺失，准入 SHALL 为 ready，`allowed_actions` SHALL 为空，不提示准备无关成员

#### Scenario: 非阻断成员缺失不掩盖真实阻断项

- **WHEN** 不消费的成员规则缺失同时伴随成员无 recorded checkout、manifest/身份异常或其他既有准入门失败
- **THEN** 系统 SHALL 保留相应真实阻断材料和既有拒绝，不清空整个 `missing_materials`，不以成员规则豁免授权 provider 启动；缺规则引用的空 digest SHALL 仅保留审计语义，不作为正文已存在或已核验的证明

#### Scenario: 根规则缺失时进入自举等待或相位

- **WHEN** 普通 provider 准入在既有真实已发布政策路径发现 canonical LC root 规则缺失、越界、symlink 逃逸或既有校验所覆盖的摘要漂移，且请求没有有效 bootstrap credential
- **THEN** 系统 SHALL 保持现有 fail-closed 拒绝与准备/重试或重新核验动作，不读取成员规则作为全局 fallback、不启动 provider；预存的 Normal 加自举桩政策的根缺失放行边界 SHALL 不变且不扩大，placeholder MUST NOT 被当作本次「根材料有效」放行的证明

#### Scenario: 普通请求的发布链失效不可借成员豁免放行

- **WHEN** 普通根消费者在既有发布链校验中遇到最终 receipt 缺失、损坏、policy locator 异常、AGENTS digest 或 root 身份漂移
- **THEN** 系统 SHALL 保留既有 fail-closed 拒绝和错误投影，不因成员规则非阻断而继续，不把错误统一伪装成已具备恢复 UI 的等待项；本次成员豁免 SHALL 不扩建 receipt TTL、全源摘要或五阶段历史校验范围

#### Scenario: 有效 bootstrap credential 解除裸仓首建死锁

- **WHEN** AggregateBootstrap 请求持有由 durable Running aggregate operation 派生且重验有效的 bootstrap credential，执行合法 root recipe，而 canonical LC root 规则、最终 receipt 和全部成员规则尚未生成
- **THEN** 系统 SHALL 通过既有自举通道豁免不消费的成员规则门，并按既有 bootstrap 边界容许根规则与最终 receipt 尚不存在；authority、policy、trust、capability、gateway、canonical cwd、target 与可用性 SHALL 继续核验，合法 recipe 不先被普通会话的 Normal capability 矩阵拒绝，不伪造 Confirmed、不写成员规则

#### Scenario: bootstrap 凭据缺失或漂移拒绝

- **WHEN** bootstrap credential 缺失、durable operation 不再为 Running，或 LC、canonical root、step、input digest 等凭据绑定事实漂移
- **THEN** 系统 SHALL 在真实 provider 启动前拒绝使用该凭据及其成员规则豁免，保留可诊断的既有凭据失败路径，不借旧 recipe 证据启动、不切换备用 cwd

#### Scenario: 普通会话不得复用 root recipe 能力证据

- **WHEN** 普通会话的 Normal capability 不满足，但存在某次有效 root recipe 的能力证据或 bootstrap credential
- **THEN** 系统 SHALL 仍按普通会话的 capability/gateway 条件拒绝，不把 recipe 证据升级为普通会话 Confirmed，不借成员规则豁免绕过 capability 门

#### Scenario: LC SC 仅要求实际聚焦成员的消费材料

- **WHEN** LC SC 已解析的 `repository.path` 指向一个聚焦成员，该路径的 `.claude/rules/language.md` 可读取为 UTF-8，而其他 active 成员的同名规则缺失
- **THEN** 系统 SHALL 仅以该实际消费路径判定 SC 必需规则，成功读取的同一份正文 SHALL 用于 prompt；其他成员缺失 SHALL 不阻断、不产生成员准备动作，不枚举全部 active 成员作为 SC 文件门，不撤销根/policy/trust/capability 等其他既有准入条件

#### Scenario: LC SC 实际消费文件缺失或不可读

- **WHEN** LC SC 对实际解析的 `repository.path` 下规则文件的消费前读取返回 NotFound、权限错误、目录错误或非法 UTF-8
- **THEN** 系统 SHALL 在真实 provider 启动前以 `member_language_rules_missing` 语义可诊断停等，detail SHALL 区分实际读取错误并指明来源；Prepare/Retry、durable Prepare/Open 和 provider 零启动 SHALL 保持，不把首次告知落为终态 Failed、不回退为规则指针；材料恢复后 SHALL 在原会话重新按同一来源读取并继续

#### Scenario: 多 target 且无唯一 focus 的 SC 消费根规则

- **WHEN** LC SC 存在多个 target 且没有唯一 focus，既有 repository 解析结果为 aggregate root view
- **THEN** 系统 SHALL 以该 `repository.path` 指向的聚合根规则文件作为 SC 必需正文；根文件可读且其他既有门通过时 SHALL 继续，不因任意成员规则缺失停等；根文件读取失败 SHALL 适用 SC 实际消费文件的 waiting 合同

#### Scenario: SC 预检与消费使用同一已解析 checkout

- **WHEN** identity registry 的 primary checkout 与 active 成员列表中首个 Main 或其他 checkout 不同
- **THEN** 系统 SHALL 使用既有身份解析确定的 `repository.path` 完成 SC 消费前检查并复用其成功读取的正文，不另选 Main/任意 checkout 预检另一份材料，不从其他 worktree 兜底找文件

#### Scenario: 既有 SC 与非 LC 行为不变

- **WHEN** SC 通过规则门进入生成或 conversational-gate revision，或非 LC 会话读取自身规则
- **THEN** 系统 SHALL 保持既有 SC 缺必需正文拒绝启动、revision 注入及 durable flow 分流合同，不新增伪装 phase；非 LC 规则读取与失败行为 SHALL 不变

#### Scenario: provider trust 登记失败

- **WHEN** Codex 或 Kimi 的当前 LC root trust 登记写入失败、记录不匹配或撤销状态异常
- **THEN** 系统 SHALL 生成包含 provider、canonical root、登记动作和错误摘要的 fail-closed 等待项，不以未信任或无 MCP 的降级会话继续；登记记录 SHALL 可重试且不影响其他 LC trust 条目

#### Scenario: gateway 能力不满足

- **WHEN** capability 检查发现当前 provider/gateway 组合不被支持
- **THEN** 系统 SHALL 拒绝准入并通知合法配置/目标选择，不启动 provider、不伪造 capability 成功、不回落到另一套路径

#### Scenario: owner 四个裸成员仓启动聚合初始化

- **WHEN** `busi`、`api`、`gateway`、`frontend` 四个成员仓均为无任何规则文件的骨架仓，根材料尚未生成，owner 点击「启动聚合初始化」且现有身份、信任、自举凭据和 recipe 能力条件满足
- **THEN** 系统 SHALL 不再因 `member_language_rules_missing` 阻断，合法 root recipe SHALL 生成根材料并使五阶段后续可以继续；成员文件树与 Git 状态 SHALL 保持无规则注入，不以手工补成员规则完成验收

### Requirement: 首建失败的可操作通知（REQ-REG-13）

每个冷启动或 repair 等待项 SHALL 通过既有驾驶舱 inbox/系统通知投影失败原因、已完成步骤、逻辑代码库与成员身份、步骤/operation 身份、可能的外部副作用、可用按钮会做什么及成功后的下一阶段。通知投递失败不得回滚业务事实，客户端 SHALL 能通过只读查询补读同一等待项；用户操作成功后 SHALL 调用原有登记/索引/编排链继续，而不是创建第二套持久化操作状态机。

#### Scenario: 失败通知后产品操作续进

- **WHEN** member index 或 aggregate index 步骤失败且通知可投递
- **THEN** 用户 SHALL 能从通知或补读页面触发安全重试/继续，系统保留失败事实并在成功后推进到下一 checkpoint

#### Scenario: 通知投递失败仍可补读

- **WHEN** 业务步骤已持久化失败但 inbox/系统通知投递失败
- **THEN** 详情/状态查询 SHALL 返回同一失败原因和允许操作；补读或重试通知不得重复执行步骤

### Requirement: 逻辑代码库生命周期：软删、恢复、彻底删除与已删除列表

系统 SHALL 提供三段式生命周期，语义 MUST 区分「可恢复」与「不可恢复」：

1. **软删**：写墓碑、从存活列表隐藏、释放成员登记；**保留**LC 子树与工作区政策正文（政策正文是恢复后的运行前提）；MAY 清理成员仓的 `.aria-pointer.md`（可再生、无恢复不变量）
2. **恢复**：摘除墓碑、恢复成员登记，id 不变、issue / 成员引用不断裂；幂等（已存活返回现状）
3. **彻底删除（硬删）**：**仅对已软删（墓碑存在）的 LC 可用**；删除 LC 子树，清理 `<聚合根>/policy/<项目>/<逻辑代码库>/` 与各成员仓 `.aria-pointer.md`；MUST NOT 触及 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` 等 Cadence-skills 产物；MUST NOT 跨 project

列表 MUST 能带出已删除（墓碑）LC 供前端呈现（含 `deleted_at`）；不带参数时行为与既有「只返回存活」完全一致。

#### Scenario: 存活逻辑代码库不允许硬删

- **WHEN** 对一个存活的逻辑代码库请求彻底删除
- **THEN** 系统 SHALL 拒绝并返回稳定错误码 `logical_codebase_not_tombstoned`（409），且零副作用

#### Scenario: 软删后可恢复

- **WHEN** 逻辑代码库被软删后请求恢复
- **THEN** 系统 SHALL 恢复为存活，id 不变；此前发布在工作区的政策正文 MUST 仍然存在，恢复后 gateway 政策校验 MUST 通过

#### Scenario: 彻底删除清理工作区产物

- **WHEN** 对已软删的逻辑代码库请求彻底删除
- **THEN** 系统 SHALL 删除其 LC 子树、`<聚合根>/policy/<项目>/<逻辑代码库>/` 与成员仓 `.aria-pointer.md`；`AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` MUST 保持原样

#### Scenario: 彻底删除幂等且不可跨项目

- **WHEN** 重复请求彻底删除，或以另一 project 的身份请求
- **THEN** 系统 SHALL 返回 NotFound，不产生任何跨项目副作用

### Requirement: 人工确认链按需登记（REQ-REG-15）

系统 SHALL 在合法 LC 人工确认的真正关门/定稿之前，按权威会话的有效 provider 与冻结 canonical root 复用 REQ-REG-14 生命周期完成按需 trust 登记；覆盖 WS 批量、live driver、同形单会话确认及 REST human action approve。未 Ready SHALL 保持确认门、返回可诊断等待与场景化人工指导；原仲裁、CAS、唯一成功广播与 gateway 只读复验 MUST 保持。方案 A MUST NOT 在首次生成自动登记，但首次生成因缺 trust 停等时 SHALL 展示同源手工登记指引。

#### Scenario: 开态门确认缺失条目自动补齐

- **WHEN** 已抵达合法开态人工门的 LC 会话由有权用户确认，其有效 author 或 reviewer 为 Codex/Kimi，冻结 canonical root 的 trust 条目缺失
- **THEN** 系统 SHALL 在该门关闭、定稿或创建子会话前登记本会话所需 trust，全部 Ready 后才沿原确认裁决继续；成功 SHALL 产生 `ensure/registered` 审计并关联 `workspace_confirm:<session_id>`、project、LC、provider、root、key，不生成不存在的 `gate_registered`

#### Scenario: 公共 WS 与 REST approve 使用相同等待合同

- **WHEN** 用户经收件箱短连接 runner、当前 live driver、同形单会话 WS confirm 或 REST human action approve 确认同一类合法人工门
- **THEN** 系统 SHALL 共用产品层准备与关门裁决；trust 未 Ready 时 WS SHALL 返回既有 protocol error，REST SHALL 返回非成功 ApiError，稳定 code 均为 `workspace_confirmation_trust_waiting`，不得落入 compile failure、generic engine error 或 accepted 成功；不增加 bulk 来源字段、第二次 confirm 或独立登记 endpoint

#### Scenario: 不合法确认在登记前拒收

- **WHEN** 确认违反 driver/observer、stale lease、stage/flow 白名单、durable WaitingForHuman、Busy、候选完整性或批准资格；其中 SC 的 WaitingForHuman 记录为 phase=None/Prepare/Generate/Failed，或 snapshot=None
- **THEN** 系统 SHALL 在任何 ensure/home/ownership/trust audit 调用之前沿既有拒绝、Busy、AlreadyClosed 或 Conflict 翻译返回；SC 合法批准仍仅接受 phase=Approval/Evaluate 且 snapshot 非空；Abandon 的既有脱困能力、反馈与 Running 普通 confirm SHALL 不变且不触发登记

#### Scenario: 有效 provider 集与免登记会话

- **WHEN** 合法确认读取本会话已锁定的 author/effective reviewer，包含同 provider、禁用 reviewer、provisional reviewer、Claude/Pi/Fake，或无 LC gateway 的单仓会话
- **THEN** 系统 SHALL 仅为有效且需要 trust 的 Codex/Kimi 去重登记，不采信建联陈旧快照、前端 root/provider、默认配置或安装列表；禁用/provisional reviewer 与无需 trust/非 LC 会话 SHALL 零 home/ownership/trust audit 调用，不改变 recipe 的 Claude-only 选择

#### Scenario: 冻结身份缺失或漂移等待

- **WHEN** LC 会话有 gateway 却缺确认登记上下文/precondition，issue 原始 LC 归属已改变，冻结 root 不能 canonicalize，或同 scope manifest root 与 gateway 冻结 root 不一致
- **THEN** 系统 SHALL 以相应 `confirmation_trust_context_unavailable`、`confirmation_trust_scope_changed`、`confirmation_trust_root_unavailable`、`confirmation_trust_root_changed` reason 停等，零 home 调用且门不变；只返回已知身份，不编造 provider/root/key、不向错误根提供手工 trust 指引，不降级到另一 LC、成员 checkout 或原路径；历史 LC SHALL 沿既有 legacy scope 约定解析

#### Scenario: 已可信条目复用与归属隔离

- **WHEN** 同 provider/root 已可信，包括本 LC 已管理条目、用户自建条目或其它 LC 创建的条目
- **THEN** 每次合法确认 SHALL 仍进入既有 ensure 归属/审计流程，但 adapter home 写次数 SHALL 为零；本 LC replay SHALL 保持 owned=true 并记 `ensure/replay`，首次复用用户或其它 LC 条目 SHALL 保持字节不变、owned=false 并记 `ensure/reuse_user_entry`，后续撤销 SHALL 为 NotOwned；不得跨 LC 合并 durable 事实

#### Scenario: Aria 协作写方并发保持条目与归属

- **WHEN** 遵守同一 home 工件锁约定的 Aria 写方并发操作同 root 同 LC、同 root 不同 LC、不同 root 的共享 Codex config，或交错 ensure/revoke
- **THEN** 系统 SHALL 串行化从读取到 home 与 ownership/clear-waiting/audit 提交的完整生命周期，保留 digest CAS 与 atomic rename；不得丢失其他 root、把本 LC owned=true 降为 false、认领其它 LC 归属或误撤销用户条目；平台不支持锁或锁获取失败 SHALL fail-closed 等待，不宣称任意外部非协作编辑受此锁保护

#### Scenario: 失败 waiting 不覆盖较新成功当前态

- **WHEN** 一个同 scope/provider/key 的失败尝试与另一成功尝试受控交错，失败的 waiting/audit 提交可能晚于成功清理
- **THEN** 当前 waiting 的写入 SHALL 与 ensure 全体排序，旧失败 MUST NOT 在较新 Ready 之后重新留下陈旧当前 waiting；失败尝试审计可保留，审计历史 MUST NOT 被当作当前等待状态；等待事实/审计 store 自身失败时仍 SHALL 返回 fail-closed 等待，不伪称所有失败均落盘

#### Scenario: 取消锁等待与确认结果未知

- **WHEN** 用户确认仍在等待 home 锁时取消，已开始 trust 提交时取消/断连，或客户端既有 8 秒超时先发生
- **THEN** 待锁取消 SHALL 不留下无界阻塞 waiter、也不在锁释放后复活业务；已开始提交 SHALL 安全收尾且不据取消误删条目，不释放确认互斥后去关闭另一扇门；超时 SHALL 保持结果未知、无自动重发，用户先读取权威门态再显式操作；home 锁 SHALL 不跨越 compile、网络或 provider run

#### Scenario: 确认登记失败给出真实指导且门不变

- **WHEN** home 读取/写入/格式/冲突/CAS 失败，或 ownership/审计等 durable fact IO 失败导致任何所需 provider 未 Ready
- **THEN** 系统 SHALL 保持原 workspace status/phase/snapshot、门节点、预算、turn、compile reservation 与 provider-start ledger，不 compile、不创建子会话、不启动 provider、不发 close、不 abort；响应 SHALL 保留原始 reason_code，包含已知 session/project/LC/provider/canonical_root/trust_key、`retryable:true`、场景正确的 `retry_action` 与 `manual_guidance`，中文 message SHALL 展示原因、门未关闭、处置及「完成后重新选择该门并确认；失败不自动重试」，不得提示确认用户「重试初始化」；retryable 表示修复后可显式确认，不表示冲突可立即安全重试

#### Scenario: provider 特定人工指导安全展示

- **WHEN** 确认或 generation 等待面需要展示 Codex/Kimi 的人工处置
- **THEN** 指导 SHALL 来自实际服务端 adapter 的同源路径/root/key：Codex 为 `.codex/config.toml` 中正确转义的目标 projects 表与 `trust_level = "trusted"`，提示备份及仅改目标表；Kimi 为实际 `.kimi-code/workspace-trust/<trust_key>` 及正确 JSON root/trustedAt；冲突须人工核验，权限/格式错误先修权限/格式，fact IO 须修 durable store，缺 adapter 只提示装配修复；不得返回完整 home 配置/凭据、伪造路径、直接执行 shell 或覆盖冲突值

#### Scenario: 部分提交不构成跨介质事务

- **WHEN** 一个 provider 已登记而另一 provider 失败，或 home 写成功后 ownership/audit 写失败或进程中断
- **THEN** 本次确认 SHALL 保持门开，已成功的 home/trust 事实 SHALL 保留、不全批回滚、不自动撤销；下次显式确认 SHALL replay/reuse，缺可读归属时保守 user-owned，不猜测认领；系统 SHALL 如实告知可能部分副作用，不承诺 home 零变化、审计必落盘或 home/facts 原子事务

#### Scenario: 准备成功后仍使用原关门竞争判据

- **WHEN** trust Ready 期间另一 worker 已改变加载的 expected workspace record 或先关闭门
- **THEN** 原整记录 CAS 与迟到翻译 SHALL 继续裁决，不重读替换 expected 规避竞争；败者 SHALL 不 compile、不创建子会话、不发第二次 close；至多一个胜者在原 compile/durable Confirmed 核验后关闭该门；可能完成的幂等 trust 登记不等于确认成功

#### Scenario: 批量入口与可见性保持 REQ-CFC-06

- **WHEN** 收件箱选择两个真实可选 human_gate 执行跨会话批量确认，包含当前 live driver 会话及远端短连接会话，某项 trust 等待
- **THEN** 每会话 SHALL 去重、沿既有 driver 仲裁恰一次发送并逐项报告，成功 close SHALL 通过原 journal/session 广播对其它连接可见；waiting 项 SHALL rejected 并展示 message/retry_action/manual_guidance，操作审计 SHALL 只存稳定 code，其它 protocol error detail 行为不变；并发上限 3、8 秒超时、无自动重试 SHALL 不变；AuthorConfirm、batch_confirm、compile_recovery、危险动作、关闭/blocked/无合法 session 身份的门 SHALL 不被新增为可批量选择项

#### Scenario: GET 与 gateway 复验仍零 trust 副作用

- **WHEN** readonly verify/build、GET/eligibility、early admission 或 spawn 前复验发现 missing、conflict，或发现此前登记后被用户删除的条目
- **THEN** 系统 SHALL 只读拒收，前后 home 与 durable trust ownership/waiting/audit 快照 SHALL 相等，不自动补条目、不伪造 trust/capability 证据；generation 等待补指导也 SHALL 使用只读同源指导能力，不调用会追加审计的 registry verify 或 ensure

#### Scenario: 新根首次生成带指导但不自动登记

- **WHEN** 全新 canonical LC root 无 Codex/Kimi trust，用户显式首次生成且其它既有材料/能力门满足，generation 在确认门出现前因缺 trust 被拒收
- **THEN** 系统 SHALL 仍 fail-closed、provider 零启动且 home/ownership/trust audit 不变，将同源 `manual_guidance`、原始原因、真实目标与「人工登记后显式重新生成」的 retry_action 带入既有 generation waiting/Prepare 回落面；不得只有无指导的 ProviderUnavailable 裸错，不新增 generation 自动登记或保证能自动抵达确认门

#### Scenario: owner 删除条目后重开确认自动恢复

- **WHEN** controller 在授权测试实例记录真实 project/LC/provider/root，备份移走目标 trust 条目，重开已经能抵达合法待确认门的会话并从原收件箱显式确认
- **THEN** 系统 SHALL 自动补同根条目，Kimi key/JSON 或 Codex 目标表正确，`ensure/registered` 审计可关联本次会话，原门恰一次关闭且其它观察连接同步可见；该验收 SHALL 不靠手工预写条目、不删除无关 home 内容，且与「新根首次生成带指导」场景分别留证，不伪称首次生成自动可用

### Requirement: LC provider 信任登记生命周期（REQ-REG-14）

系统 SHALL 为需要 workspace trust 的后续 LC provider 提供按 canonical `provider_context_root` 绑定的登记与撤销。LC 根准入（REQ-REG-09）确定并冻结 root identity 后、Claude Code recipe 启动前 SHALL 完成所需 trust 的独立硬前置登记；该登记不属于五步 recipe、不插入步骤之间，普通 Codex/Kimi LC session 启动前 SHALL 再次核验；任一所需 trust 未 Ready 时五步 recipe 不创建/不启动，初始化进入可重试 fail-closed waiting。Codex SHALL 使用用户级 projects trust 配置，Kimi SHALL 使用由 basename 与 canonical root SHA-256 前缀确定的 workspace-trust 记录；登记 SHALL 幂等、只影响当前 LC 根、有归属证明时在 LC 删除或解绑撤销本 LC 管理且未被外部修改的条目，不得覆盖或撤销已有的用户自建 trust，并生成不含敏感凭据的审计事实。Codex/Pi/KimiCode 的实际 LC gateway launch 由后续 `lc-gateway-multi-provider` change 交付。

#### Scenario: Codex trust 登记

- **WHEN** LC 根准入已冻结 Codex 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 在用户级 Codex projects trust 中登记该 canonical root 为 trusted，并配置后续 LC root 启动参数包含 `--skip-git-repo-check`；登记失败 SHALL 阻止五步 recipe 创建/启动，生成可重试 fail-closed waiting；实际 Codex LC gateway launch 由 `lc-gateway-multi-provider` 解锁

- **WHEN** LC 根准入已冻结 Kimi 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 写入 `wd_<basename>_<sha256(canonical_root)[:12]>` 对应 workspace-trust 记录，并在后续启动前复验 root 一致；登记失败 SHALL 阻止五步 recipe 创建/启动，生成可重试 fail-closed waiting；实际 Kimi LC gateway launch 由 `lc-gateway-multi-provider` 解锁

#### Scenario: 重试与多 LC 隔离

- **WHEN** 同一 LC 重复执行登记，或另一 LC 同时执行登记
- **THEN** 同一 canonical root 的登记 SHALL 幂等，其他 root 的记录 SHALL 保持不变；每次结果均可由 operation/lc/provider/root/key 审计关联

#### Scenario: LC 删除或解绑撤销

- **WHEN** LC 被删除或 provider trust 解绑操作确认执行
- **THEN** 系统 SHALL 仅撤销该 LC 管理且其 canonical root 对应的 Codex/Kimi trust 记录，并保留撤销前后摘要与结果；不得删除其他 root 或用户原有的信任登记

#### Scenario: 既有用户 trust 与并发修改

- **WHEN** 当前 root 已有用户自行登记的 trust、同 key 已有不可信值，或 LC 管理的 trust 在撤销前被外部修改
- **THEN** 已可信的用户自建条目 SHALL 原样复用并记为非本 LC 归属，未受信值或撤销时摘要不匹配 SHALL 进入等待/核验，不得覆盖或删除用户值；其他 LC 的登记 SHALL 保持不变

#### Scenario: 跨工作区写入失败

- **WHEN** 用户 home 下的 trust 配置不可写、格式冲突或撤销无法核验
- **THEN** 系统 SHALL fail-closed 停等并展示外部副作用说明；不得启动依赖该登记的 provider，也不得覆盖无关配置
