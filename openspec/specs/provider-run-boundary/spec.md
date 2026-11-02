# provider-run-boundary Specification

## Purpose
为作者侧 provider 运行提供宿主可验证的运行边界：每轮 prompt 无条件携带「本轮运行边界」块，宿主为每个 workspace session 托管短命的专属 scratch 并保证其不跨 run 残留，宿主对作者侧 tool-call 的四类越界读写给出稳定码告警，且全程只告警不阻断、provider 侧零改动。本能力是软约束加检测，不构成物理沙箱。

### 目标行为映射

| 目标行为 | 对应需求 |
|---|---|
| A1 作者侧 prompt 无条件注入边界块 | REQ-PRB-01 / 02 / 03 |
| A2 per-session scratch 位置与权限 | REQ-PRB-04 |
| A3 scratch 随 run 收口删除 + 崩溃遗留回收 | REQ-PRB-05 / 06 / 07 |
| A4 四类越界稳定码告警 | REQ-PRB-09 / 10 |
| A5 provider 侧与既有契约零改动 | REQ-PRB-12 |
| A6 既有测试保持绿色 | REQ-PRB-12 |

## Requirements

### Requirement: REQ-PRB-01 边界块对作者侧 prompt 无条件注入

系统 MUST 在每个 workspace session 的作者、计划与修订 provider 启动前，于其 prompt 中注入「本轮运行边界」块。注入 MUST NOT 依赖基线分支解析结果：聚合/LC 会话、无基线、无 `repository_path` 的会话 MUST 同样注入。块 MUST 位于最终合同尾之前，MUST NOT 破坏该 prompt 对输出格式的末端约束。无持久 store 的内存态 engine MUST 省略整块，且其 prompt MUST 与改动前字节一致。

#### Scenario: 聚合/LC 无基线会话仍获得边界块

- **WHEN** 聚合规划会话（story、design 或 work_item_plan，基线解析返回无基线）启动作者 provider
- **THEN** prompt MUST 含边界块标题与全部必选条目，MUST NOT 因基线教学块被跳过而缺失

#### Scenario: 有基线会话两块并存

- **WHEN** 单仓会话基线可解析
- **THEN** prompt MUST 同时含边界块与既有基线教学块，二者互不替代，既有基线教学块文案 MUST 保持不变

#### Scenario: 内存态 engine 不受影响

- **WHEN** 无持久 store 的内存态 engine 构造作者 prompt
- **THEN** prompt MUST 不含边界块，既有 prompt 预算与内容断言 MUST 保持通过

### Requirement: REQ-PRB-02 边界块内容为白名单与通道声明

边界块 MUST 以 `## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）` 为标题，并 MUST 包含三类条目：

1. 本轮身份（`workspace_session`、`workspace_type`）与「产物唯一权威通道是本轮最终响应流，磁盘文件（含自己的草稿、其他会话或历史运行遗留）不是权威来源，也不会被采集」；
2. 事实来源白名单（本 prompt 的 context、已确认 Story/Design/AC、当前工作区）与禁读清单（系统临时目录中非本轮创建的文件、`.aria` 状态目录、其他会话或历史运行的产物与日志、provider 自身历史会话记录），并规定发现此类文件时视为过期残留、不读取、不复用、不汇报其内容；
3. 落盘规则：默认无需写文件，直接在最终响应输出，不得提前执行落盘类步骤；并如实披露编辑类内建工具（Edit/Write/原生文件变更）受岗位纪律限制、不应使用。

块 MUST NOT 把系统临时目录整体列为禁区，MUST NOT 披露计划产物的真实状态路径。块总体积 MUST 受固定字节预算约束（不超过 1.5 KB）。

#### Scenario: 白名单而非 /tmp 禁令

- **WHEN** 检查边界块文案
- **THEN** 块 MUST 以「允许读什么 / 禁读什么类别」表述，MUST NOT 含「禁止访问 /tmp」这类整目录禁令，状态根位于临时目录之下的部署 MUST NOT 因此自相矛盾

#### Scenario: 不泄露状态路径

- **WHEN** 检查任一作者侧 prompt 中的边界块
- **THEN** 块 MUST NOT 含 `issue-work-item-plans` 等计划产物的真实状态路径

#### Scenario: 体积受预算约束

- **WHEN** 渲染含 scratch 条目的边界块
- **THEN** 块字节数 MUST NOT 超过预算断言上限

### Requirement: REQ-PRB-03 注入覆盖全部作者侧构造路径

边界块 MUST 覆盖 story、design、work_item、work_item_plan 四种作者类型的初次生成、增量续轮、修订，以及产物校验失败后的自动续写重试（该路径整体替换 prompt、不经常规漏斗）。reviewer 路径 MUST NOT 注入该块。同一 prompt 内边界块 MUST 至多出现一次。

#### Scenario: 自动续写重试同样携带边界块

- **WHEN** 作者产物校验失败并触发自动续写，续写 prompt 由重试构造路径生成
- **THEN** 续写 prompt MUST 含边界块，且 MUST NOT 与原 prompt 叠加成两份

#### Scenario: 修订面携带边界块

- **WHEN** 作者对已有产物发起修订
- **THEN** 修订 prompt MUST 含边界块

#### Scenario: reviewer 不注入

- **WHEN** reviewer provider 启动
- **THEN** reviewer prompt MUST NOT 含边界块，reviewer 的既有基线教学与权限 MUST 保持不变

### Requirement: REQ-PRB-04 per-session scratch 位置与权限

宿主 MUST 为每个 workspace session 提供专属 scratch 目录，位于状态根下的 `run-scratch/<workspace_session_id>/`，与 `tool-policy-run-audit/` 等 session 分区并列。目录 MUST NOT 位于仓库工作区或 provider cwd 内；在 unix 下权限 MUST 为 0700。scratch 的绝对路径 MUST 在边界块中显式告知，且 MUST 声明运行结束系统会删除它。目录不存在时，边界块 MUST 省略 scratch 条目而非告知不存在的路径。新增分区 MUST NOT 引起 `PRODUCT_DATA_SCHEMA_VERSION` 变化。

#### Scenario: 位置与权限

- **WHEN** 某 session 的 run 注册完成
- **THEN** `run-scratch/<session_id>/` MUST 存在、MUST 位于状态根下而非仓库内，unix 权限 MUST 为 0700

#### Scenario: 目录缺失时退化

- **WHEN** 构造 prompt 时 scratch 目录不存在（创建失败或第二步尚未落地）
- **THEN** 边界块 MUST 省略 scratch 条目，其余条目 MUST 保持，run MUST NOT 因此失败

### Requirement: REQ-PRB-05 scratch 创建、清空与存活准入

run 在 manager 登记后，宿主 MUST 尽力准备该 session 的 scratch：先清空旧内容，再重建目录并设权限。准备 MUST 经 session 存活准入，purging 或 deleted 的 session MUST NOT 因准备而复活 scratch 目录。准备失败 MUST 仅告警，MUST NOT 使 run 失败。同一 run 内 provider 重启 MUST NOT 清空 scratch。supersede 与 claim-if-idle 两种登记路径 MUST 行为一致，且覆盖全部 provider run 种类。

#### Scenario: 新 run 不见前序内容

- **WHEN** 同一 session 的上一 run 在 scratch 中留有文件，新 run 完成登记
- **THEN** 新 run 的 scratch MUST 为空目录，前序文件 MUST 不可见

#### Scenario: 同 run 内重启保留内容

- **WHEN** 同一 run 内 provider 因重试或续写被重新启动
- **THEN** 已有 scratch 内容 MUST 保持不变

#### Scenario: 已删除 session 不复活

- **WHEN** session 处于 purging 或 deleted 时发起准备
- **THEN** 准备 MUST 被拒绝，`run-scratch/<session_id>/` MUST NOT 被创建

### Requirement: REQ-PRB-06 scratch 随最后一个活动 run 收口删除

当 session 的 run 收口且此时已无活动 run 时，宿主 MUST 删除该 session 的 scratch；该判定 MUST 在 manager 状态锁内作出，删除 MUST 在释放 engine 之后进行。若新 run 已在位（旧 token 的迟到收口），收口 MUST NOT 删除 scratch。收口 MUST 覆盖正常结束、取消收口与 abort 迟到收口。删除 MUST 幂等，NotFound 视为成功；删除失败 MUST 仅告警，由下次 run 注册时的清空与启动 sweep 回收。session 停在等待人工状态时 MUST NOT 以 session 结束为由额外清理。

#### Scenario: 正常收口清理

- **WHEN** session 唯一的活动 run 正常结束
- **THEN** `run-scratch/<session_id>/` MUST 被删除

#### Scenario: supersede 竞态不误删

- **WHEN** 旧 run 被新 run 取代，新 run 已登记后旧 token 才完成收口
- **THEN** 新 run 的 scratch MUST 保持存在且内容完整

#### Scenario: 取消与失败路径不漏清

- **WHEN** run 以取消、watchdog 超时或失败收口
- **THEN** 只要此时无活动 run，scratch MUST 被删除

#### Scenario: 清理失败不影响收口

- **WHEN** 删除因权限等原因失败
- **THEN** 收口 MUST 继续完成，系统 MUST 仅记录告警，残留 MUST 由后续注册清空或启动 sweep 回收

### Requirement: REQ-PRB-07 崩溃遗留由启动 sweep 回收

服务启动并完成端口绑定及其他启动副作用的前置条件后，宿主 MUST 执行一次孤儿 scratch 回收。回收 MUST 仅处理 `run-scratch/` 的直接子目录，并仅删除满足任一条件者：对应 session 记录不存在；session 状态为 `Confirmed`、`Terminated` 或 `Failed`；目录 mtime 超过孤儿 TTL。孤儿 TTL MUST 为 6 小时，且 MUST 以具名常量表示、可由 owner 调整。回收失败 MUST 仅记录日志，MUST NOT 阻塞服务启动，单项失败 MUST NOT 影响其余项。同一状态根上并存的其他实例的活 run scratch（session 存活、状态非终态、mtime 在 TTL 内）MUST NOT 被删除。

#### Scenario: 三类孤儿被回收

- **WHEN** 启动时存在 session 记录已不存在、状态已终态、mtime 超过 6 小时的 scratch 目录
- **THEN** 三者 MUST 均被删除

#### Scenario: 活 run 的 scratch 保留

- **WHEN** 某 scratch 对应 session 存活、非终态且 mtime 在 TTL 内
- **THEN** sweep MUST NOT 删除它

#### Scenario: 非法目录名跳过

- **WHEN** `run-scratch/` 下出现无法通过 id 校验的目录名
- **THEN** sweep MUST 跳过且 MUST NOT 删除，其余合法孤儿 MUST 继续被处理

### Requirement: REQ-PRB-08 所有删除必须带护栏

本能力的每一次删除（收口清理、注册时清空、启动 sweep）MUST 满足：只删除 `run-scratch/` 分区下经 id 校验的直接子目录，分区根自身 MUST NOT 被删除；删除前核对目标仍是该分区根的直接子项，路径漂移 MUST 不删除并告警；目标若为符号链接或非目录 MUST NOT 被删除也 MUST NOT 被跟随；目录内部的符号链接 MUST 只删链接本身，MUST NOT 进入其指向的外部目录；MUST NOT 删除不是由本能力创建的路径。本能力 MUST NOT 提供对任意路径的递归删除入口，MUST NOT 删除历史版本产物。

#### Scenario: scratch 内的符号链接不诱导外删

- **WHEN** provider 在 scratch 内创建指向外部目录的符号链接，随后 run 收口
- **THEN** 清理 MUST 只移除链接本身，外部目录及其内容 MUST 保持不变

#### Scenario: 目标本身是符号链接

- **WHEN** `run-scratch/<session_id>` 被替换为指向别处的符号链接
- **THEN** 清理与 sweep MUST NOT 删除也 MUST NOT 跟随该链接，MUST 告警

#### Scenario: 非本能力路径永不被删

- **WHEN** 分区外的路径或用户文件与 scratch 同名片段相近
- **THEN** 任何清理 MUST NOT 触碰该路径

### Requirement: REQ-PRB-09 四类越界读写的稳定码检测

宿主 MUST 对作者侧 provider 的 `Command` 类 tool-call 事件做路径分类，命中下列越界类别之一时 MUST 产生带对应稳定码的告警。四个稳定码与类别：① `run_boundary_provider_history_read`（provider 历史目录本身、其之下或其祖先）；② `run_boundary_aria_state_read`（Aria 状态树：状态根之下且不在本 session scratch 之下，或系统临时目录下第一级名称为 `aria-*` 的状态/备份目录之下）；③ `run_boundary_tmp_draft_touch`（系统临时目录下第一级名称命中通用草稿名，`aria-*` 除外——它归类别②）；④ `run_boundary_write_outside_scratch`（写形态词元指向 scratch 之外的绝对路径）。扫描 MUST 手写切词，MUST NOT 引入新依赖。

#### Scenario: provider 历史目录判定范围

- **WHEN** 作者侧命令触碰的路径等于、位于或构成 provider 历史目录的祖先
- **THEN** 系统 MUST 产生 `run_boundary_provider_history_read` 告警；codex 历史目录 `~/.codex/sessions` MUST 覆盖（既有实证 `src/cross_cutting/local_usage.rs:134`），其余 provider 的历史目录 MUST 来自可配置表，其真机核实项待实现时完成

#### Scenario: 临时目录草稿名按第一级名称判定

- **WHEN** 作者侧命令触碰的路径位于系统临时目录（`/tmp`、`/var/tmp`）下，且其**第一级名称**不区分大小写地命中 `*plan*`、`*draft*`、`*prior*`、`*refined*`、`*work-item*`、`*spec*`、`*design*` 或 `*story*`
- **THEN** 系统 MUST 产生 `run_boundary_tmp_draft_touch` 告警；判定 MUST 仅依据第一级名称，MUST NOT 依赖 cwd；第一级名称为 `aria-*` 的目录 MUST NOT 归本码，而 MUST 归 `run_boundary_aria_state_read`

#### Scenario: 写形态词元判定

- **WHEN** 作者侧命令含写形态词元（重定向、`tee`、`cp`、`mv`、`sed -i`、`apply_patch` 的新增/更新文件、`write_text`、以写模式打开、here-doc 写入），且其目标为 scratch 之外的绝对路径
- **THEN** 系统 MUST 产生 `run_boundary_write_outside_scratch` 告警

#### Scenario: 事故读取向量全部命中

- **WHEN** 作者侧命令读取 `/tmp` 下的 `work-item-plan*.md`、遍历 `~/.codex/sessions` 下的 rollout 文件、读取 `/tmp/aria-restore` 或 `/tmp/aria-deploy` 下的状态与日志、读取 `/tmp/prior-plan*`
- **THEN** 系统 MUST 分别产生 `run_boundary_tmp_draft_touch`、`run_boundary_provider_history_read`、`run_boundary_aria_state_read`、`run_boundary_tmp_draft_touch` 对应的告警

#### Scenario: 写到 scratch 之外命中

- **WHEN** 作者侧命令把内容重定向到 `/tmp/work-item-plan.md`
- **THEN** 系统 MUST 产生 `run_boundary_write_outside_scratch` 告警

#### Scenario: 合法访问不误报

- **WHEN** 作者侧命令只读取 cwd 内文件、读取 provider 的 skills 目录，或读写本 session 的 scratch
- **THEN** 系统 MUST NOT 产生任何越界告警

#### Scenario: 位于临时目录下的仓库不误报

- **WHEN** 被审仓库位于 `/tmp/<repo>` 下，命令读取其内部任意文件
- **THEN** 系统 MUST NOT 产生 `run_boundary_tmp_draft_touch`

### Requirement: REQ-PRB-10 告警事件形状、范围与上界

告警 MUST 以 `kind=Provider`、`status=Completed` 的执行事件写入同一 node detail，事件 id MUST 形如 `run_boundary_{code}_{path_hash8}` 且同 node 内按 id 去重，标题 MUST 为 `运行边界告警: {code}`，detail MUST 含 `path`、`access=read|write` 与 `source_event`，`command`、`cwd`、`output` MUST 为空。原 tool-call 事件 MUST 先于告警持久化且 MUST NOT 被修改。扫描 MUST 仅作用于作者侧节点（含作者运行、修订与 work item 系列作者节点），MUST NOT 作用于 reviewer 节点；MUST 只扫描 `command` 与 `detail` 的前 4 KiB；每个 node 的告警 MUST 至多 8 条。告警事件自身 MUST NOT 再触发扫描。扫描函数 MUST NOT panic，MUST NOT 返回 `Result`。

#### Scenario: 原事件不受影响

- **WHEN** 某 tool-call 命中规则
- **THEN** node detail MUST 同时含原事件与告警事件，原事件内容 MUST 与未启用检测时一致

#### Scenario: 同路径重复触碰去重

- **WHEN** 同一 node 内同一路径同一规则被多次触碰
- **THEN** 告警 MUST 因事件 id 相同而去重为一条

#### Scenario: 数量上界

- **WHEN** 同一 node 内命中超过 8 个不同告警
- **THEN** 该 node MUST 至多记录 8 条告警，超出部分 MUST 被丢弃且 MUST NOT 影响 run

#### Scenario: reviewer 节点不扫

- **WHEN** reviewer 节点的命令触碰临时目录下的 `*plan*` 路径
- **THEN** 系统 MUST NOT 产生告警

### Requirement: REQ-PRB-11 只告警，不阻断

检测与注入 MUST 为纯观察与提示：任何告警 MUST NOT 使 turn 失败、MUST NOT 触发审批拒绝、MUST NOT 终止或取消 provider、MUST NOT 改变产物确认或 auto_if_valid 判定。检测代码路径的任何内部失败 MUST NOT 向 run 传播。系统 MUST NOT 对 provider 读自身历史或读状态目录实施拦截。

#### Scenario: 命中告警后 run 照常完成

- **WHEN** 作者 run 触发多条越界告警后正常产出合法产物
- **THEN** run MUST 照常完成、产物 MUST 照常进入确认流程，告警 MUST 仅作为 node detail 中的记录存在

#### Scenario: resume 读取历史只记录

- **WHEN** 续接会话中 provider 读取自身历史目录
- **THEN** 系统 MUST 仅产生 `run_boundary_provider_history_read` 告警，MUST NOT 拦截该读取

### Requirement: REQ-PRB-12 provider 侧与既有契约零改动

本能力 MUST NOT 更改 codex 的 sandbox 与审批策略，MUST NOT 更改 provider 适配层的工具策略片段、gateway envelope（含 `writable_roots` 与 `readable_roots`）、`ProviderToolPolicy` 的 token 与 digest、`SessionResumeFingerprint`、`StreamingProviderInput` 结构、codex T13 的 FileChange 判失败语义、daemon 的 canonical writeback，MUST NOT 覆盖 provider 的 `TMPDIR`、`CODEX_HOME`、`HOME`。scratch MUST NOT 进入 envelope 或指纹。既有 prompt 预算测试与内存态 engine 测试 MUST 保持通过。

#### Scenario: 引入前后契约摘要一致

- **WHEN** 对同一作者 run 比较启用本能力前后的 `ProviderToolPolicy` 摘要、envelope 内容与 resume 指纹
- **THEN** 三者 MUST 完全一致

#### Scenario: 原生文件变更语义不变

- **WHEN** codex 作者 turn 出现原生 FileChange item，即使其目标位于 scratch 内
- **THEN** 该 turn MUST 仍按既有 T13 语义判失败，本能力 MUST NOT 为 scratch 开豁免

#### Scenario: 既有测试保持绿色

- **WHEN** 运行既有 prompt 预算断言与内存态 engine 测试
- **THEN** 它们 MUST 在不修改断言上限的前提下通过
