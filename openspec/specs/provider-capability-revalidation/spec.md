# provider-capability-revalidation Specification

## Purpose
为 LC 作用域内的 provider capability 提供用户显式的真实验证通道：一次点击触发真实现场探针并原子导入 durable Confirmed；已验证且版本一致时秒回不重跑；失败如实上报。门本身（admission fail-closed）不变。

## Requirements

### Requirement: 用户显式 capability 重验证通道

系统 SHALL 提供 per-provider 的显式 capability 重验证动作：仅由用户显式触发（HTTP POST，携带 `provider_type`），对该 provider 执行真实现场边界探针（复用既有 `run_cli_boundary_probe` 通道：覆盖 CodingTargetWrite、PlanningReadOnly、ReviewReadOnly 三个 action，含 resume 面），并把每个通过三方一致性校验的探针结果经 `record_verified_probe` 原子导入 durable Confirmed。导入写入的 capability store SHALL 是**机器级（整机）**记录，且 SHALL 与 gateway/admission 消费的作用域**完全同一份**。系统 SHALL NOT 在无显式点击时运行任何探针，SHALL NOT 提供手工改写 Confirmed 的旁路。SHALL NOT 存在按项目或按逻辑代码库分别维护或分别读取的 capability 记录。

#### Scenario: 点击核验补齐 Unknown

- **WHEN** provider 的 capability 行为 Unknown 且用户对该 provider 触发重验证
- **THEN** 系统 SHALL 执行真实现场探针，探针通过后将该 provider 的 action 行导入**机器级** durable Confirmed（launch/write_boundary/resume 由工件签发），随后机器级作用域的 admission 读到 Confirmed

#### Scenario: 不点击不探测

- **WHEN** capability 行为 Unknown 且用户未触发重验证
- **THEN** 系统 SHALL NOT 运行任何 provider CLI 探针；admission 维持既有 fail-closed 拒绝

#### Scenario: Fake 与未知 provider 拒绝

- **WHEN** 重验证请求的 `provider_type` 是 Fake 或未知值
- **THEN** 系统 SHALL 返回稳定错误码（provider_capability_probe_unsupported），不执行任何探测、不写入 capability 记录

#### Scenario: 核验只核验一次

- **WHEN** 用户对某 provider 完成一次重验证，随后在任意上下文里触发该 provider 的准入判定
- **THEN** 准入 SHALL 读到**同一份**机器级记录里的 Confirmed；系统 SHALL NOT 要求再核验一次，SHALL NOT 因某个逻辑代码库的子树下没有 capability 文件而判定为未核验

#### Scenario: 无项目也能核验

- **WHEN** 当前工作区没有选中任何项目（或该工作区根本没有项目），用户仍显式触发某 provider 的重验证
- **THEN** 系统 SHALL 正常执行探针并把结果写入机器级记录；SHALL NOT 因缺少项目或逻辑代码库而拒绝、SHALL NOT 静默跳过；该动作面向整机 provider CLI 能力，与项目、逻辑代码库、聚合根、成员仓均无关

### Requirement: 版本钉定的一次性幂等语义

重验证 SHALL 尊重 capability 记录的 exact version 钉定：action 行已在当前 CLI exact version 上 Confirmed（launch、write_boundary、resume 全 Confirmed）时，该行 SHALL NOT 重跑探针；三行全齐时点击 SHALL 快速返回 `already_confirmed`（仅执行一次 CLI `--version` 版本探测，不启动探针会话）。CLI 版本相对 durable 记录漂移，或任一必需行缺失/非 Confirmed 时，系统 SHALL 仅对缺失或漂移的 action 执行探针；探针内部版本门漂移时 SHALL 如实失败，不沿用旧版本证据。

#### Scenario: 全行已验证秒回
- **WHEN** provider 三个 action 行均已在当前 CLI 版本 Confirmed 且用户再次点击核验
- **THEN** 系统 SHALL 不运行边界探针并返回 already_confirmed，durable 记录与 probed_at 保持不变

#### Scenario: 版本升级触发重验
- **WHEN** CLI 升级导致 durable 记录 version 与当前 CLI 不一致
- **THEN** 系统 SHALL 重新执行探针并以新 exact version 导入；导入前的旧 Confirmed 行不跨版本沿用（沿既有 import 语义）

### Requirement: 探针失败如实上报

任一 action 探针失败（CLI 缺失、边界不可用、正/负探针未达预期、resume 面失败、导入被拒）时，系统 SHALL 返回携带稳定错误码、action 名与原始错误详情的失败响应； SHALL NOT 把失败静默为通过、不伪造 Confirmed、不部分遮蔽（同一请求中先成功的 action 导入保留，失败 action 如实点名）。

#### Scenario: CLI 缺失如实失败
- **WHEN** 目标 provider CLI 不在场（`--version` 探测失败）
- **THEN** 系统 SHALL 返回稳定错误码（provider_capability_probe_failed）与缺失详情，capability 记录保持点击前状态

#### Scenario: 探针中断不伪造
- **WHEN** 边界探针在任一环节失败关闭
- **THEN** 系统 SHALL 上报该 action 的失败详情；未通过校验的证据 SHALL NOT 进入 durable

### Requirement: 只读 capability 状态投影

系统 SHALL 提供 capability 状态 GET 面（**面向平台，不按项目或逻辑代码库定位**）：逐 provider 投影**机器级** durable action 矩阵（Confirmed/Denied/Unknown）、exact version、probed_at、probe_artifact_ref 与 provider 通道可用性（四家真实 provider 之外不出现）；该面零写入、零探针触发。投影内容 SHALL NOT 随项目或逻辑代码库而变化。

#### Scenario: 状态面只读

- **WHEN** 用户读取 capability 状态
- **THEN** 系统 SHALL 返回当前 durable 事实且不运行任何 CLI、不修改任何记录

#### Scenario: 未探测投影 Unknown

- **WHEN** provider 尚无任何真实探针证据
- **THEN** 状态面 SHALL 投影全 Unknown（bootstrap 默认记录不得被当作真实证据显示为 Confirmed）

#### Scenario: 不存在按项目或逻辑代码库区分的投影

- **WHEN** 用户在不同上下文（不同项目、不同逻辑代码库）读取 capability 状态
- **THEN** 各次投影 SHALL 来自**同一份**机器级记录并给出相同结论；MUST NOT 出现分叉

### Requirement: 能力状态可机读与拒绝分类（REQ-PCR-01）

provider capability 的 launch 分格 SHALL 以**稳定小写 token** 表达其状态（`confirmed` / `denied` / `unknown`），错误文案与任何分类器 SHALL 共用该词表；MUST NOT 依赖 Debug 格式化输出（`{other:?}`）来推断状态。

被 capability 门拒绝时，系统 SHALL 给出**可机读的拒绝分类**：记录缺失或分格为未核验 → 未核验（Unverified）；分格为 `Denied{reason}` → 真实拒绝（Denied，携带真实原因）；capability snapshot 引用漂移 → 证据过期（EvidenceStale）。分类 SHALL 由**既有**拒绝原因文本判定，MUST NOT 新增一套独立的能力判定逻辑；判定 MUST NOT 放宽 gateway 的准入（只有 `Confirmed` 放行）。

无法归入上述三类的拒绝 SHALL **保持既有错误码与既有文案形态**（零回归），MUST NOT 被改写成三态之一。

#### Scenario: 三态各有稳定 token

- **WHEN** capability launch 分格分别为 `Confirmed` / `Denied{reason}` / `Unknown`
- **THEN** 系统 SHALL 分别产出稳定 token `confirmed` / `denied` / `unknown`；MUST NOT 产出依赖 Debug 输出的可变文本

#### Scenario: 未核验被归为 Unverified

- **WHEN** 拒绝原因为「能力记录缺失」或「某 action 的 launch 分格为未核验」
- **THEN** 分类 SHALL 为**未核验（Unverified）**；MUST NOT 与「实测不支持」混为一类

#### Scenario: 真实拒绝携带真实原因

- **WHEN** 拒绝原因为「某 action 的 launch 分格被判定为 denied」并带原因文本
- **THEN** 分类 SHALL 为**真实拒绝（Denied）**且 SHALL 携带该真实原因；MUST NOT 把原因藏进括号或 Debug 文本

#### Scenario: snapshot 漂移归为证据过期

- **WHEN** 拒绝原因为 capability snapshot 引用与记录不一致
- **THEN** 分类 SHALL 为**证据过期（EvidenceStale）**；MUST NOT 归为未核验

#### Scenario: 不匹配时零回归

- **WHEN** 拒绝原因不匹配上述任何一类
- **THEN** 系统 SHALL 产出与改造前**逐字节相同**的 `reason_code` 与 `action`；MUST NOT 改写文案

#### Scenario: 准入不放宽

- **WHEN** 任意 provider 的某 action 行不是 `Confirmed`
- **THEN** gateway SHALL 继续 fail-closed 拒绝；MUST NOT 因新增了分类能力而放行

### Requirement: 能力相关失败面按三态分流并逐家点名（REQ-PCR-02）

与 provider 能力相关的失败面 SHALL 按三态分流为三个稳定码：未核验 → `aggregate_provider_capability_unverified`，实测拒绝 → `aggregate_provider_capability_denied`，证据过期 → `aggregate_provider_capability_evidence_stale`；三路文案 SHALL 与 provider 选择阶段的结论复用**同一套文案来源**。

三类失败 SHALL **逐家点名**：候选表 SHALL 齐全列出四家候选并按 owner 顺序排列，每家 SHALL 给出命令可用性、命令原因码、能力状态与**可执行动作提示**。机器码与原始 detail SHALL 降级为次要行（复用既有 `details.stderr_summary`，MUST NOT 改变 DTO 形状）。核验指引 SHALL 只指向**单个** provider 的核验入口，MUST NOT 指向尚未提供的「一键核验全部」入口。

#### Scenario: 选择阶段的四种结论各有稳定码

- **WHEN** provider 选择阶段得出结论
- **THEN** 四种结论 SHALL 各自有稳定码：无可用命令 → `aggregate_initialization_no_provider_available`；有命令但都未核验 → `aggregate_initialization_provider_needs_verification`；有命令但都被实测拒绝 → `aggregate_initialization_provider_capability_denied`；provider health 异常 → `aggregate_initialization_provider_health_degraded`；四者的文案 SHALL 与执行期三态文案同源

#### Scenario: 未核验给出单家核验指引

- **WHEN** 失败原因是某 provider 的能力尚未核验
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_unverified`；文案 SHALL 点名该 provider 并说明「已安装但未核验」；`details.revalidate` SHALL 给出可点击的核验入口（含 `ui_path` / `button_label` / `method` / `path` / `body` / `expect`）；MUST NOT 指向一键核验全部

#### Scenario: 真实拒绝指向下一家

- **WHEN** 失败原因是某 provider 的能力被实测判定为不支持，并带真实原因
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_denied`；文案 SHALL 点名该 provider 与真实原因，并给出优先顺序中的下一家；`details.fallback_provider` 与 `details.revalidate` SHALL 指向下一家

#### Scenario: 证据过期要求重新核验

- **WHEN** 失败原因是 capability snapshot 引用漂移
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_evidence_stale`；文案 SHALL 说明证据已作废并要求重新核验该 provider

#### Scenario: 未核验与实测拒绝不共用一句文案

- **WHEN** 两种失败分别发生
- **THEN** 二者 SHALL 使用不同的稳定码与不同的主文案；MUST NOT 出现同一句文案覆盖两种语义

#### Scenario: 三类候选结果逐家点名

- **WHEN** 产生「需要核验」「实测拒绝」或「无可用」类失败
- **THEN** 失败详情 SHALL 齐全列出四家候选（按 owner 顺序），每家 SHALL 含命令可用性、命令原因码、能力状态与可执行动作提示；MUST NOT 只说「没有可用 provider」而不点名

#### Scenario: 无可用命令时给出安装线索

- **WHEN** 四家候选的命令均不可用
- **THEN** 失败码 SHALL 为 `aggregate_initialization_no_provider_available`；每家候选的动作提示 SHALL 指向安装方式；MUST NOT 归因为「未核验」

#### Scenario: 机器码降级且 DTO 形状不变

- **WHEN** 上述任一失败面被投出
- **THEN** 原始 detail SHALL 经既有 `details.stderr_summary` 字段作为次要行呈现；MUST NOT 改变 DTO 字段形状，也 MUST NOT 把机器码当主文案

#### Scenario: 不匹配既有分类的失败保持原样

- **WHEN** 某条 provider turn 失败不属于能力三态
- **THEN** 失败码 SHALL 保持既有 `aggregate_{step}_failed` 形态与文案；MUST NOT 被改写为能力三态之一

### Requirement: 一键批量核验的异步作业入口（REQ-PCR-03）

系统 SHALL 提供批量核验入口 `POST /api/providers/capabilities/revalidate-all`，其响应 SHALL **立即返回 `job_id`** 与初始作业状态，MUST NOT 在该响应内等待任何 provider 探针完成。批量核验的结果 SHALL 导入**机器级** capability 记录。

系统 SHALL 提供 `GET /api/providers/capabilities/revalidate-all/{job_id}` 返回作业状态与逐家结果。作业 SHALL 只由用户显式点击发起，MUST NOT 因聚合初始化失败或任何其它流程自动触发。

#### Scenario: 发起即返回作业号

- **WHEN** 用户触发批量核验
- **THEN** 系统 SHALL 立即返回 `job_id` 与 `running` 状态；MUST NOT 在 HTTP 响应里等待任何 provider 探针完成

#### Scenario: 轮询作业与逐项结果

- **WHEN** 客户端携带 `job_id` 轮询
- **THEN** 系统 SHALL 返回作业状态（`running` / `completed`）与逐家结果（`provider_type` / 状态 / 原因或各 action 结果）

#### Scenario: 不自动触发

- **WHEN** 聚合初始化或其它流程因 provider 能力未核验而失败
- **THEN** 系统 SHALL NOT 自动发起批量核验作业；MUST NOT 运行任何探针（沿用主规范「不点击不运行探针」契约）

#### Scenario: 批量核验结果落在机器级记录

- **WHEN** 批量作业完成并导入某 provider 的探针结果
- **THEN** 该结果 SHALL 落在**机器级** capability 记录中；任何项目、任何逻辑代码库的准入判定 SHALL 读到同一份记录

#### Scenario: 作业与项目无关

- **WHEN** 用户在没有任何选中项目的工作区发起批量核验
- **THEN** 系统 SHALL 正常创建作业并推进逐家结果；作业记录 SHALL NOT 绑定任何项目或逻辑代码库

### Requirement: 批量核验的并发、失败隔离与逐项结果（REQ-PCR-04）

批量作业 SHALL 对四家真实 provider **全并发**执行（每家一个 provider 进程），MUST NOT 串行。每家 SHALL 直接复用主规范既有的单家重验证服务（`revalidate(project_id, lc_id, provider)`），因此既有的「全行已在当前版本 Confirmed → 秒回 `already_confirmed`（零探针）」与「CLI 缺失在版本探测阶段即失败」语义 SHALL 原样继承，本 requirement MUST NOT 重新定义它们。

单家失败 SHALL NOT 阻断其它家。逐项状态 SHALL 落在 `confirmed` / `failed` / `skipped` 三类并带原因（如 `cli_not_installed`）。作业到终态时仍未完成的家 MUST NOT 被报成已确认。

#### Scenario: 四家全并发

- **WHEN** 批量作业启动，四家 provider 都存在且都未核验
- **THEN** 四家的探针 SHALL 并发执行；MUST NOT 串行（串行 = 4 倍等待）

#### Scenario: 单家失败不阻断

- **WHEN** 某一家探针失败
- **THEN** 其它家 SHALL 继续执行到终态；该家 SHALL 标为失败并带原因，整批作业 MUST NOT 失败，其它家结果 MUST NOT 丢失

#### Scenario: 已核验的家瞬时返回

- **WHEN** 某家的三个 action 行均已在当前 CLI 版本 Confirmed
- **THEN** 该家 SHALL 按既有单家语义秒回 `already_confirmed`（零探针），作业结果标为已确认

#### Scenario: CLI 未安装的家标为跳过

- **WHEN** 某家 provider CLI 不在场（版本探测阶段失败）
- **THEN** 该家 SHALL 标为 `skipped` 并带稳定原因（如 `cli_not_installed`）；作业 SHALL NOT 因此挂起或整体失败

#### Scenario: 超时如实标注

- **WHEN** 作业到达终态时仍有家未完成
- **THEN** 状态 SHALL 为 `completed` 且未完成项 SHALL 标为超时/未完成；MUST NOT 谎报为已确认

### Requirement: 作业状态的 durable 与刷新恢复（REQ-PCR-05）

批量作业的状态（job 与每家状态）SHALL 持久化。页面刷新或重新进入后，系统 SHALL 能按既有 `job_id` 返回该作业的进度与逐项结果，MUST NOT 丢失进度或要求重新发起。

#### Scenario: 页面刷新后继续轮询

- **WHEN** 批量作业进行中，页面刷新或重新进入
- **THEN** 系统 SHALL 能按既有 `job_id` 继续返回该作业进度与逐项结果；MUST NOT 丢失进度或要求重新发起

#### Scenario: 终态作业仍可读

- **WHEN** 页面在作业终态后重新打开
- **THEN** 系统 SHALL 仍能返回该 `job_id` 的最终逐项结果；MUST NOT 返回「作业不存在」

### Requirement: 一键核验的全部入口与逐项进度（REQ-PCR-06）

能力卡 SHALL 提供「一键核验全部」按钮。点击后系统 SHALL 发起批量作业并按固定间隔（2~3 秒）轮询，作业终态后停止。进行中 SHALL 显示「核验中（N/4）」并逐项实时更新状态，按钮 SHALL 禁用。

失败项 SHALL 每行保留单项「重试」，其动作 SHALL 走既有单 provider 端点。`skipped`（CLI 未安装）项 SHALL 引导到该 provider 的【安装】按钮、版本落后项 SHALL 引导到【更新】按钮；安装/更新机制由块2 `provider-cli-installation` 定义（`REQ-PCI-02` / `REQ-PCI-05`），本 requirement MUST NOT 重新定义它。

按钮区域 SHALL 说明：核验是**机器级**的（**一次核验整机可用**，不再按项目或逻辑代码库分别核验）、会真实跑一次探测并消耗 provider 额度、可能需要几分钟。

#### Scenario: 一键按钮发起并显示逐项进度

- **WHEN** 用户点击「一键核验全部」
- **THEN** 系统 SHALL 发起批量作业并以「核验中（N/4）」显示进度，逐项状态随轮询更新；按钮 SHALL 保持禁用直到作业终态

#### Scenario: 失败项单独重试

- **WHEN** 作业结束且某家标为失败
- **THEN** 该行 SHALL 保留「重试」按钮，点击后走既有单 provider 核验端点，MUST NOT 重新发起整批作业

#### Scenario: 未安装项引导到安装

- **WHEN** 某家结果为 `skipped`（`cli_not_installed`）
- **THEN** 该行 SHALL 显示【安装】入口并走 `provider-cli-installation` 的作业；MUST NOT 显示为「核验失败」

#### Scenario: 版本落后引导到更新

- **WHEN** 某家本地版本落后于 registry 最新版本
- **THEN** 该行 SHALL 显示【更新】入口；判定与作业 SHALL 复用 `provider-cli-installation`，MUST NOT 由本能力自行判版本

#### Scenario: 说明额度与范围

- **WHEN** 用户看到「一键核验全部」按钮
- **THEN** 该区域 SHALL 说明核验是**机器级**的、**一次核验整机可用**、会真实跑一次探测、可能需要几分钟；MUST NOT 让用户误以为这是零成本的后台检查，MUST NOT 让用户误以为需要为每个项目或逻辑代码库分别核验

#### Scenario: 终态停止轮询

- **WHEN** 作业到达终态（`completed`）
- **THEN** 前端 SHALL 停止轮询并渲染最终逐项结果；MUST NOT 无限轮询

### Requirement: 能力记录的机器级作用域（REQ-PCR-07）

provider capability 记录 SHALL 只有**一个**作用域：**机器级（整机）**。

- 写入 SHALL 一律落机器级文件；读取 SHALL 一律读该机器级文件。
- SHALL NOT 按项目或按逻辑代码库分别维护、读取或提升任何 capability 记录；SHALL NOT 存在「机器级无记录时回退扫描其它作用域」的读取分支。
- 能力是**本机 provider CLI 的事实**，与项目、逻辑代码库、聚合根、成员仓、规则与信任登记均无关；**核验只核验一次**，机器级一份事实。
- capability 门本身的强度 SHALL NOT 改变：仍只有 `Confirmed` 放行，`Denied` / `Unknown` / 记录缺失 / 快照漂移一律 fail-closed；既有稳定错误码与文案形态 SHALL 保持不变。

#### Scenario: 写入只落机器级

- **WHEN** 任何路径导入或自举 provider capability 记录（探针导入、自举占位、外部入口导入）
- **THEN** 记录 SHALL 落在机器级文件；系统 SHALL NOT 在任何项目目录或逻辑代码库子树下写出 capability 记录

#### Scenario: 读取只读机器级

- **WHEN** 任何路径读取 capability 记录（provider 选择门、gateway 准入、只读状态投影、批量作业前置）
- **THEN** 读取 SHALL 来自机器级文件；系统 SHALL NOT 读项目级或逻辑代码库子树下的 capability 文件，SHALL NOT 在机器级无记录时回退扫描其它作用域

#### Scenario: 不存在第二作用域

- **WHEN** 用户对某 provider 完成一次核验
- **THEN** 任何上下文的准入判定 SHALL 读到**同一份**机器级记录的同一结论；系统 SHALL NOT 要求再核验一次

#### Scenario: 记录缺失仍按未核验处理

- **WHEN** 机器级 capability 文件不存在或该 provider 无记录
- **THEN** 判定 SHALL 为「未核验」并沿既有失败面给出可点击的核验入口；SHALL NOT 静默放行，SHALL NOT 改变既有错误码与文案形态

#### Scenario: 准入强度不变

- **WHEN** 某 provider 的某 action 行不是 `Confirmed`
- **THEN** gateway SHALL 继续 fail-closed 拒绝；MUST NOT 因作用域扩大而放行

#### Scenario: 不产生第二作用域

- **WHEN** 检查任意项目目录与任意逻辑代码库子树
- **THEN** 该位置 SHALL NOT 存在 capability 记录文件；若存在即为缺陷；系统 SHALL NOT 提供把该文件迁入机器级记录的兼容路径

### Requirement: 能力面不再按项目或逻辑代码库定位（REQ-PCR-08）

capability 相关的 HTTP 面 SHALL 面向平台：路径 SHALL NOT 携带 `project_id` 或 `logical_codebase_id`；读写 SHALL NOT 以「某个逻辑代码库是否存在」作为守卫或前置条件。

- capability 状态 GET、单家重验证、批量重验证发起、批量作业查询 SHALL 全部走机器级路由。
- 旧的项目级/逻辑代码库级 capability 路由 SHALL 被删除；系统 SHALL NOT 保留兼容别名、转发 shim 或双写路径。
- 前端能力卡与能力面板 SHALL NOT 以「是否选中项目 / 是否存在逻辑代码库」作为渲染或发起的前置条件。

#### Scenario: 状态面无需项目定位

- **WHEN** 客户端直接请求机器级 capability 状态面
- **THEN** 系统 SHALL 返回机器级事实；SHALL NOT 要求项目或逻辑代码库标识，SHALL NOT 因缺少标识而返回 404

#### Scenario: 核验面无需项目定位

- **WHEN** 客户端在机器级路由上发起单家或批量重验证
- **THEN** 系统 SHALL 正常执行；SHALL NOT 要求项目或逻辑代码库标识

#### Scenario: 旧路由不再存在

- **WHEN** 客户端请求已删除的旧 capability 路由
- **THEN** 系统 SHALL 返回「路由不存在」；SHALL NOT 转发到新路由、SHALL NOT 保持旧行为

#### Scenario: 面板入口不依赖选中项目

- **WHEN** 当前没有选中任何项目
- **THEN** 「Provider 管理」入口 SHALL 可用、能力面板 SHALL 能打开并展示机器级事实；SHALL NOT 置灰、SHALL NOT 不渲染

### Requirement: root-recipe 事实的 per-LC 归属（REQ-PCR-09）

root-recipe 交付事实（`root_recipe_evidence`：某逻辑代码库的 root recipe 是否已交付、其凭据是否仍有效）SHALL **不**存放在机器级 capability 记录里。

- 它 SHALL 存放在按逻辑代码库归属的独立事实中，并继续由 per-LC 的 durable operation 状态重核验（`AggregateInitializationOperationStore::for_lc`）。
- 机器级 capability 记录 SHALL NOT 携带任何绑定逻辑代码库、项目或路径的字段。
- root-recipe 凭据重核验的**准入强度** SHALL NOT 改变：失效时仍 fail-closed。

#### Scenario: 机器级记录不含 per-LC 字段

- **WHEN** 检查机器级 capability 记录
- **THEN** 其中 SHALL NOT 出现项目 id、逻辑代码库 id 或任何路径字段

#### Scenario: root-recipe 事实仍按 LC 判定

- **WHEN** 某逻辑代码库的 root recipe 凭据按其 durable operation 状态重核验为失效
- **THEN** 该逻辑代码库的 root-recipe 准入 SHALL 继续 fail-closed；SHALL NOT 因能力记录升为机器级而被跳过或放行

#### Scenario: 拆出不改变判定结论

- **WHEN** 同一逻辑代码库在 root-recipe 事实拆出前后分别做准入判定
- **THEN** 两次结论 SHALL 完全一致；MUST NOT 出现「拆出前拒绝、拆出后放行」或反之
