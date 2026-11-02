# work-item-plan-single-instance Specification

## Purpose
规定单 plan 不变量与 plan 代际取代（supersede）契约：同一 issue 任一时刻至多存在一个 plan 实体；新 plan 建立的三个触发点必须先删除旧 plan；删除必须覆盖旧 plan 拥有的全部从属产物，失败 fail-closed 并可重入，重复请求幂等，且不得以历史实体形式保留旧 plan。

补充边界：本 capability 只规定「旧 plan 作为整体被取代时如何删干净」；用户面单实体删除的分级确认、引用门禁与 coding 拒删仍以 `lifecycle-entity-deletion`、`work-item-group-deletion` 为唯一来源，本 capability 不重定义其确认语义。

## Requirements

### Requirement: 单 plan 不变量与建立前先删旧（REQ-WPS-01）

系统 SHALL 保证同一 issue 任一时刻至多存在一个存活 plan 实体，覆盖 legacy 与 schema-v2 两路存储。任何建立 plan 的路径 MUST 在 issue 级 plan 作用域互斥内先完成旧 plan 的完整删除（REQ-WPS-03），旧 plan 删除未成功前 MUST NOT 建立新 plan；建立完成后该 issue 的存活 plan 数恒为 1。MUST NOT 以摘除列表摘要、解绑 session、改写 plan_id 引用、置软删标记或仅增代际版本号代替删除；旧 plan 的 id、session 与新 plan 的 id、session MUST 互不复用。

#### Scenario: issue 已有 plan 时再次建立
- **WHEN** 某 issue 已存在一个存活 plan，客户端或自动链再次触发 plan 建立
- **THEN** 系统 MUST 先按 REQ-WPS-03 完成旧 plan 删除，再建立新 plan，对外不出现两个存活 plan 同时可见的窗口
- **AND** 新 plan 的 plan_id/session_id/session 目录 MUST 与旧 plan 不同，不得沿用旧实体

#### Scenario: 建立后不变量成立
- **WHEN** 新 plan 建立并绑定完成
- **THEN** 该 issue 下存活 plan 数 MUST 恰为 1，plan 列表、workspace 投影与 issue 详情 MUST NOT 显示两个 plan

#### Scenario: 旧 plan 删除未完成则不建立
- **WHEN** 旧 plan 删除返回失败、处于进行中或被运行互斥阻挡
- **THEN** 系统 MUST NOT 建立新 plan、不绑定新 session、不写新 compile/draft，并返回可定位错误与旧 plan 身份

#### Scenario: 不同 issue 的同名逻辑 plan 不受影响
- **WHEN** 为 issue A 建立新 plan 并删除其旧 plan
- **THEN** issue B 的 plan 及产物 MUST 原样保留；不变量按 issue 作用域独立成立

### Requirement: 三个建立触发点必须删除旧 plan（REQ-WPS-02）

系统 MUST 在以下三类路径上删除旧 plan，且 MUST NOT 依赖用户先手工删除：(a) 自动化 enrollment 在 terminated 后经人工 Enable 重新开启并轮换 `prepare_intent_id`（旧 plan/session 由旧 intent 派生，沿用会让 prepare 幂等复用被终止的旧实体）；(b) 重新 prepare；(c) 重新编译确认。三者 MUST 复用与用户面删除相同的闭包清单（REQ-WPS-03）、失败语义（REQ-WPS-04）与幂等语义（REQ-WPS-05），并 MUST 记录触发原因、旧 plan 身份与新代身份以便审计与排障。若触发点选择沿用同一 plan_id 原地演进，MUST NOT 以「未建立新实体」为由跳过旧代运行资源清理。

#### Scenario: re-enable 轮换 prepare_intent_id 时删除旧 plan
- **WHEN** terminated enrollment 经人工 Enable 重新开启，新代 `prepare_intent_id` 与终止前不同
- **THEN** 系统 MUST 在新 prepare 建立 plan/session 之前删除旧 plan 及其完整闭包
- **AND** 随后建立的 plan/session MUST 由新 intent 派生；不得因旧 intent 仍存在而幂等复用旧 plan

#### Scenario: 重新 prepare 不产生第二个 plan
- **WHEN** 某 issue 已有存活 plan，再次调用 prepare 且服务端判定需要建立新代
- **THEN** 系统 MUST 先删除旧 plan 再 prepare；prepare 返回后该 issue 存活 plan 数 MUST 仍为 1

#### Scenario: 重新编译确认不产生第二个 plan
- **WHEN** 某 issue 已有 Confirmed 或曾发布 plan，重新编译并确认建立新代
- **THEN** 系统 MUST 在进入新代 compile/publish 前删除旧 plan 闭包；旧 publication/revision MUST 不再可读，该 issue 存活 plan 数 MUST 仍为 1

#### Scenario: 原地重编译确认不得跳过旧代运行资源清理
- **WHEN** 某 issue 已有 Confirmed 或曾发布 plan，重新编译确认时系统选择沿用同一 plan_id 原地演进而非建立新实体
- **THEN** 系统 MUST NOT 以「未建立新 plan」为由跳过旧代从属清理：旧代 coding attempts、worktree 实体、reviewer windows/snapshots/leases、attempt locks 与 workspace lifecycle MUST 被终止并删除，新代 MUST NOT 复用旧 attempt/worktree identity
- **AND** 若该触发点改为以新 plan_id 建立新代，则 MUST 先按 REQ-WPS-03 删除旧 plan 实体后才发布新代

#### Scenario: 同 command_id 重放不重复删除
- **WHEN** 以相同 `command_id` 重放触发点请求
- **THEN** 系统 MUST 返回首次 durable 结果，不重复执行删除、不新增墓碑、不递增代际

### Requirement: 旧 plan 取代删除覆盖完整从属闭包（REQ-WPS-03）

系统 MUST 在旧 plan 被取代时物理删除该 plan 拥有的全部从属产物，清单以本 requirement 首个场景的逐项枚举为准。删除 MUST 按真实拥有关系先清子树、最后删 plan 根，MUST 以真实 owner-key（plan_id/work_item/attempt_id 及 `(project, issue, repository)`）定位，MUST NOT 连带删除其他 plan、其他 issue 或其他 repository 的同名产物；对象本已缺失 MUST 视为已清。

#### Scenario: 逐项闭包清单必须为零残留
- **WHEN** 旧 plan 取代删除成功完成
- **THEN** 以下每一项产物 MUST 均不存在，且按旧 plan_id 反查均无结果：
  - plan 记录（legacy 与 schema-v2 两路）
  - work-item-plan-sources
  - work_item_plan_drafts 与 work_item_plan_compiles
  - work-item-revisions 与 publications
  - coding-plan-runs
  - coding-attempts 记录，及该 attempt 的 runs/units/events 目录
  - coding-plan-workspace-lifecycles
  - shared-worktree 记录与锁
  - worktree 实体
  - reviewer windows、snapshots 与 leases，**含其归档目录**（归档 MUST NOT 充当删除）
  - advance-records，含 initialization journal
  - group-initializations
  - work-item-attempt-locks

#### Scenario: 归档不等于删除
- **WHEN** 旧 plan 的 reviewer 窗口已进入终止态并被移入归档目录，或快照已归档
- **THEN** 系统 MUST 把归档记录与归档文件同样视为该 plan 的从属产物并物理删除
- **AND** MUST NOT 以「窗口已归档/已终止」为由保留窗口记录、租约记录或快照基线文件

#### Scenario: coding attempt 及其运行目录随 plan 清除
- **WHEN** 旧 plan 存在 coding attempt，且该 attempt 有 runs/units/events 子目录
- **THEN** attempt 记录与其全部运行子目录 MUST 一并删除；不得只删记录留下孤儿目录，也不得只删目录留下记录

#### Scenario: reviewer 资源按精确归属清除
- **WHEN** 旧 plan 的 attempt 关联 reviewer windows、snapshots 与 window lease 记录
- **THEN** 系统 MUST 按真实 attempt/plan 归属物理清除这些窗口记录、租约记录与快照基线文件，而非只置终止态或只移入归档；不得清理其他 plan 的 reviewer 资源

#### Scenario: 共享工作树仍被其他 plan 使用
- **WHEN** 目标 plan 的 shared-worktree 记录/锁所属 repository 键仍被同 issue 另一存活 plan 使用
- **THEN** 系统 MUST 只清目标 plan 的专属引用与该 plan 独占的 worktree 实体，保留仍被他人使用的共享资源

#### Scenario: 摘要残缺但仍须清全
- **WHEN** plan 摘要缺少某个实际绑定的 work item 或 session，而该对象仍有真实 plan binding
- **THEN** 系统 MUST 依真实归属将其纳入清理集合，不得因摘要缺失而漏清

### Requirement: 从属产物删除失败时 fail-closed 并显式 needs_human（REQ-WPS-04）

系统 MUST 使旧 plan 取代删除为 fail-closed：任一从属产物删除失败时，整体 MUST NOT 报告成功，MUST NOT 建立新 plan，且 MUST NOT 出现「plan 根已删但从属产物仍存在」的中间态——因此 plan 根 MUST 最后删除，任何子树失败时 plan 根 MUST 保留为可达的待清理入口（purging/needs_human 状态）并持久记录失败项、错误与触发点。系统 MUST NOT 静默跳过失败项后继续建立新代；显式标记为 needs_human 时必须给出可定位的失败清单，且重入 MUST 能从同一入口继续清到完整闭包。

#### Scenario: 子树清理失败不建立新 plan
- **WHEN** 旧 plan 的 source/revision/attempt 等任一子树清理发生真实 IO 或归属错误
- **THEN** 系统 MUST 返回可定位错误，plan 根仍存在且标记 needs_human，新 plan MUST NOT 被建立

#### Scenario: 禁止「根已删产物还在」
- **WHEN** 任一从属产物删除失败
- **THEN** 系统 MUST NOT 已完成 plan 根的删除与成功墓碑落盘；不得以「已删除」事实掩盖仍存在的从属产物

#### Scenario: needs_human 后重入完成
- **WHEN** 故障排除后以同一 plan 身份重入删除
- **THEN** 系统 MUST 从同一待清理入口继续清除剩余项，成功后落单条无内容墓碑并允许建立新代，不产生重复墓碑或恢复已删内容

### Requirement: 取代删除幂等且进行中不产生二次副作用（REQ-WPS-05）

系统 MUST 使旧 plan 删除按 plan 身份幂等：删除已完成后重发同一删除/取代请求 MUST 返回幂等已删除结果，墓碑数量与时间不变，不新增副作用、不复活内容、不阻挡新代建立。删除正在进行中再次提交 MUST NOT 并发启动第二份清理、MUST NOT 重复取消 runner、MUST NOT 重复清理目录，而 MUST 收敛到同一次进行中的删除事实（返回进行中或等待其结果）。

#### Scenario: 完成后重放幂等
- **WHEN** 旧 plan 已完成删除并落墓碑，触发点或重试再次请求删除同一 plan id
- **THEN** 系统 MUST 返回幂等已删除，墓碑仍为 1 条、删除时间不变，新 plan 建立照常进行

#### Scenario: 删除进行中再次提交
- **WHEN** 同 plan 的取代删除仍在进行，另一个请求（含不同 command_id）再次触发删除
- **THEN** 系统 MUST NOT 启动第二份清理或二次副作用；MUST 返回同一进行中事实或等待其完成，最终仍只落 1 条墓碑

#### Scenario: 删除失败并发重试不叠加
- **WHEN** 一次删除已 fail-closed 标记 needs_human，重试与触发点请求并发到达
- **THEN** 系统 MUST 只推进同一待清理入口，不得并行清理同一子树或产生互相覆盖的部分删除

### Requirement: 与 terminated 交互时一并删除 attempts 与 worktrees（REQ-WPS-06）

在 re-enable 取代 terminated plan 的路径上，系统 MUST 一并删除旧 plan 上的 coding attempts 及其 worktree 实体、shared-worktree 记录与锁、attempt locks、reviewer leases，MUST NOT 复用旧 attempt id、旧 worktree 路径或旧锁；删除前 MUST 先把未终结 attempt 收口到终止终态（复用整 plan 终止语义），MUST NOT 以「仍存在 attempt」为由永久阻断取代。相对边界：用户面单实体 DELETE 遇到 coding attempt 仍按既有 `coding_workspace_exists` 拒删（REQ-DEL-05 不变）；本 requirement 只覆盖系统授权的 re-enable 取代路径，不把该路径开放给普通删除入口。

#### Scenario: re-enable 删除旧 plan 的 attempts 与 worktree
- **WHEN** terminated plan 仍持有已 Completed/Aborted 的 coding attempts 与 worktree 实体，用户提交重新开启
- **THEN** 收口完成后系统 MUST 删除这些 attempts 及其 runs/units/events、worktree 实体与 shared-worktree 记录/锁、attempt locks 与 reviewer leases
- **AND** 新代 attempt 与 worktree MUST 使用新 identity 与路径，不复用旧资源

#### Scenario: 未终结 attempt 先收口再删除
- **WHEN** terminated plan 仍存在未终结 attempt 或未收口运行事实
- **THEN** 系统 MUST 先按其既有终止收口语义完成收口，再执行取代删除；收口失败 MUST 返回既有 cleanup_required 语义并零副作用

#### Scenario: 普通删除入口不被放宽
- **WHEN** 用户对仍有 coding attempt 的 plan 走普通 DELETE
- **THEN** 系统 MUST 维持既有 coding 拒删语义，不把 re-enable 的「级联删 attempt」能力暴露给普通删除入口

### Requirement: 不保留历史 plan 实体，仅留无内容删除事实（REQ-WPS-07）

取代删除完成后，旧 plan 实体与其从属产物 MUST 均不存在；系统 MUST NOT 以历史版本、归档副本、软删行或「保留旧 plan 供追溯」的列表项保留旧 plan。允许且仅允许保留 `entity-deletions/plan/<id>.json` 形式的无内容删除事实（scope、实体种类、id、删除时间、完成状态、触发原因），该事实 MUST NOT 含标题、正文、source/IR/report/review、原 hash 或恢复副本，且 MUST NOT 冒充存活 plan、不得被列表/workbench attach 视为可编辑实体、不得为新代提供幂等复用依据。

#### Scenario: 按旧 id 查询只剩墓碑
- **WHEN** 取代删除完成后按旧 plan id 读取 plan、workspace attach、issue 详情或 plan 列表
- **THEN** 系统 MUST 报不存在或已删除，且仅存在 1 条 `entity-deletions/plan/<id>.json` 事实；无历史 plan 版本可读

#### Scenario: 墓碑不授权复活写入
- **WHEN** 已取代删除的旧 plan 身份收到迟到的生成、compile、advance、coding 创建或旧 session 提交
- **THEN** 系统 MUST 拒收并保持闭包不存在；墓碑 MUST NOT 被当作可用 plan 放行

#### Scenario: 触发审计可定位
- **WHEN** 因 re-enable 轮换、重新 prepare 或重新编译确认发生取代删除
- **THEN** 系统 MUST 在删除事实或等价审计记录中留下触发原因与旧 plan 身份，使排障可区分「用户删除」与「代际取代」
