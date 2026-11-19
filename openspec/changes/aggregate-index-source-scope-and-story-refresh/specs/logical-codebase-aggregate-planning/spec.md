## ADDED Requirements

### Requirement: Story 生成前的共享索引刷新（REQ-PLN-18）

用户点**生成或重新生成 Story 之后、AI 开始写之前**，系统 SHALL 先确保该逻辑代码库的**共享聚合索引**是最新的：走 Story 生成**专用**的刷新入口（模式），复用既有的规划前索引检查路径完成「检查 → 必要时同步 → 重新验收」，通过后才启动 AI。刷新 SHALL 只针对该逻辑代码库共享的那一份索引，MUST NOT 为每个 Issue 另建一份索引；创建 Issue、勾选成员、刷新页面以及同一次生成中的后续对话 MUST NOT 触发一次额外刷新；本能力 MUST NOT 引入后台定时轮询。

**允许降级放行的范围只有本能力之外的只读规划。** Design 生成、WorkItemPlan 与工作区上下文读取等路径 SHALL 保持既有语义：以 last-known-good 加可审计告警继续，MUST NOT 因 degraded 停机。Story 生成 MUST NOT 复用那条放行口径——它是唯一要求「索引必须是最新且已验收」的入口（与 `REQ-IND-04` 的 degraded 分界一致）。

刷新 SHALL 走三个分支，且 SHALL 依据可核验的证据选择分支：

1. **无可用索引** ⇒ 走既有的**首次建库**流程（前提是成员登记与主工作目录已就绪），而不是每次新建 Issue 都从头建库。此分支只适用于「该逻辑代码库没有任何可读世代」；若世代记录存在但读不出来、归属不明或彼此矛盾，SHALL 走既有失败规则报错，MUST NOT 当作「没有索引」静默触发首建。
2. **能确认没变** ⇒ **仅当 published 世代的 `status` 为 `Active`**，且以下比对全部通过，才复用该世代：成员集合未变、主工作目录身份未变、Git 提交未变、**上次与本次都没有未提交修改**。「没有未提交修改」SHALL 以该成员主工作目录的 `git status --porcelain` 为准（**被 `.gitignore` 忽略的文件不参与判定**）。判定 SHALL NOT 附加「当前世代不含跳过成员」这类条件。
3. **有变化或不能确认没变** ⇒ 实际执行增量同步并**重新验收**。成员集合、主工作目录身份或 Git 提交变了、上次或本次显示有未提交修改，SHALL 归入本分支——revision 相同不能证明工作区文件未变，所以「上次与本次都显示有未提交修改」MUST NOT 跳过同步（两次都脏时即使提交号相同也必须同步，否则同一个 dirty 布尔会掩盖两次检查之间的改动）。published 世代的 `status` 为 **`Degraded` 或 `Stale`**，或 **`current_snapshots` 采集失败**，也一律走分支 3：SHALL 尝试恢复/同步并重新验收，仍非 `Active` 就停在 AI 启动前。上次更新失败留下的旧版本 MUST NOT 直接当作最新版。归入本分支 SHALL NOT 以「当前世代含跳过成员」为条件：被 `.gitignore` 忽略的文件对 CodeGraph 与 git **同时不可见**（增量同步也收录不到），不构成可见范围内的变化；git 可见范围内的变化仍按前述比对判定。

若初始化或另一次重建正在更新同一份索引，系统 SHALL 等待并复核其结果，MUST NOT 并行写两份；拿到锁并不意味着要跳过复核——锁只串行化写入，调用方 SHALL 以本次实际验收并发布的记录是否为 `Active` 判断成功与否，MUST NOT 用仍带 dirty 的重新评估结果否定刚刚成功的验收。

刷新或验收失败、或恢复/同步并重新验收后仍非 `Active` 时，系统 SHALL **停在本次生成之前**（AI 不启动），说明原因并允许重试；SHALL 保留已有 Issue、Story 与上次成功的索引，MUST NOT 拿旧索引冒充最新继续生成。「最新」指本次检查或同步时**本地成员主工作目录**里的代码，系统 MUST NOT 主动拉取远端，MUST NOT 承诺生成过程中后来保存的改动已经收录。

**失败面要求**：刷新未通过 SHALL 以**稳定码 + 同源中文成品**呈现，MUST NOT 只抛通用存储错误。稳定码 SHALL 优先沿用失败封套的内层码（`aggregate_index_member_coverage_failed`、`aggregate_index_member_source_excluded`、`aggregate_index_source_scan_incomplete`、`aggregate_index_all_members_source_less` 或其它已登记码）；封套解析不出稳定码时 SHALL 用已登记的 `aggregate_index_unavailable`。中文成品 SHALL 取自 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10` 登记的同一张码表。生成入口 SHALL 把该失败映射为带稳定码与成品文本的运行时错误（HTTP `details.copy` 同构），MUST NOT 落成通用 500 的 `product_store_error`（`src/web/handlers/support_parts/product_store_error.inc.rs` 的兜底分支会把 `ProductStoreError::Io` 映射成「product store operation failed」）。

刷新通过后，本次 Story SHALL 记录其参考的索引版本，沿用规划上下文的既有索引标识字段。

#### Scenario: 没有索引时先建库再写 Story

- **WHEN** 用户请求生成 Story，而该逻辑代码库没有任何可读的聚合索引世代
- **THEN** 系统 SHALL 先走既有首次建库流程并完成验收，通过后才启动 AI；MUST NOT 跳过建库直接生成

#### Scenario: 记录读坏不等于没有索引

- **WHEN** 该逻辑代码库存在索引世代记录，但记录读不出来、归属不明或彼此矛盾
- **THEN** 系统 SHALL 按既有失败规则报错并给出稳定码与三段成品，停在 AI 启动之前；MUST NOT 把它当作「没有索引」去静默首建或复用旧索引

#### Scenario: 代码干净且未变时复用

- **WHEN** 用户请求生成 Story，published 世代的 `status` 为 `Active`，成员集合、主工作目录与 Git 提交均未变，上次与本次都没有未提交修改（`git status --porcelain` 为空）
- **THEN** 系统 SHALL 复用当前这一版索引，MUST NOT 触发一次增量同步；即使当前世代含跳过成员，判定结果也 SHALL 相同（不得以「含跳过成员」为由强制同步）

#### Scenario: 两次都显示有未提交修改时必须同步

- **WHEN** 成员工作区在本次检查与上次验收时都显示存在未提交修改，即便 Git 提交号相同
- **THEN** 系统 SHALL 判定为「不能确认没变」，实际做增量同步并重新验收；MUST NOT 以 revision 未变为由跳过同步

#### Scenario: 被忽略的文件不参与变更检测

- **WHEN** published 世代的 `status` 为 `Active`，成员集合、主工作目录身份与 Git 提交均未变，上次与本次 `git status --porcelain` 均为空，成员工作区只有被 `.gitignore` 忽略的文件发生变化
- **THEN** 系统 SHALL 判定为「能确认没变」并复用当前这一版索引，MUST NOT 因忽略文件触发增量同步；被忽略的文件同样 MUST NOT 被算作漏收（见 `REQ-IND-01`）

#### Scenario: 成员或提交变化时同步并重新验收

- **WHEN** 成员集合、主工作目录身份或 Git 提交自上次验收后发生变化
- **THEN** 系统 SHALL 实际做增量同步并按聚合索引的同一套验收规则重新验收，通过后才启动 AI

#### Scenario: 更新失败时停在 AI 启动之前

- **WHEN** 增量同步或重新验收失败
- **THEN** 系统 SHALL 停在本次生成之前、说明失败原因并允许重试；已有 Issue、Story 与上次成功的索引 SHALL 保留；MUST NOT 用旧索引冒充最新继续生成，MUST NOT 启动 AI

#### Scenario: 刷新失败给稳定码与中文成品

- **WHEN** Story 前刷新因覆盖失败、源码仅在工作树、盘检无法确认或全员无源码而失败
- **THEN** 响应 SHALL 携带对应稳定码与同源三段中文成品（问题/原因/怎么做/机器码）；MUST NOT 只返回通用存储错误或只有机器码的 500

#### Scenario: degraded 的索引不放行 Story 但放行只读规划

- **WHEN** 该 LC 只有 degraded 的 last-known-good，本次请求是生成 Story
- **THEN** 系统 SHALL 先尝试恢复并验收；仍非 Active 就停在 AI 启动之前并给出稳定码与成品；同一时刻的 Design / WorkItemPlan 读取 SHALL 仍以 last-known-good 加告警继续

#### Scenario: 非 Active 或当前快照采集失败时不复用

- **WHEN** 用户请求生成 Story，published 世代的 `status` 为 `Degraded` 或 `Stale`（即使成员证据看起来未变），或者 `current_snapshots` 采集失败（即使 published 世代为 `Active`）
- **THEN** 系统 SHALL 走分支 3，尝试恢复/同步并重新验收，MUST NOT 直接复用；验收结果仍非 `Active` 时 SHALL 停在 AI 启动前，给出稳定码与同源三段成品。

#### Scenario: 并发刷新不并行写两份

- **WHEN** 本次刷新与另一次初始化/重建同时更新同一份索引
- **THEN** 系统 SHALL 等待其结束并复核本次实际发布的记录，MUST NOT 并行写两份；成功与否 SHALL 以该发布的记录为准，MUST NOT 用仍 dirty 的重评估结果否定一次已成功的验收

#### Scenario: 上次失败的旧版不能直接当最新版

- **WHEN** 上一次更新失败留下了一个旧版本索引，而本次检查认为没有变化
- **THEN** 系统 SHALL 先尝试恢复并验收该版本，MUST NOT 直接把它当作最新版本复用

#### Scenario: 非生成动作不触发刷新

- **WHEN** 用户创建 Issue、勾选成员、刷新页面，或在同一次生成中进行后续对话
- **THEN** 系统 MUST NOT 为这些动作额外刷新共享索引；MUST NOT 为单个 Issue 另建一份索引

#### Scenario: 同库共享一份索引

- **WHEN** 同一逻辑代码库下的多个 Issue 分别请求生成 Story
- **THEN** 这些请求 SHALL 共享该逻辑代码库的同一份聚合索引，MUST NOT 各自新建索引世代；同一时刻只允许一份刷新在跑，其它请求等待并复核
