## MODIFIED Requirements

### Requirement: 用户显式 capability 重验证通道

系统 SHALL 提供 per-provider 的显式 capability 重验证动作：仅由用户显式触发（HTTP POST，携带 `provider_type`），对该 provider 执行真实现场边界探针（复用既有 `run_cli_boundary_probe` 通道：覆盖 CodingTargetWrite、PlanningReadOnly、ReviewReadOnly 三个 action，含 resume 面），并把每个通过三方一致性校验的探针结果经 `record_verified_probe` 原子导入 durable Confirmed。导入写入的 capability store SHALL 是**项目级全局**记录（`projects/{project_id}/logical-codebase/capabilities.json`），且 SHALL 与 gateway/admission 消费的作用域**完全同一份**。系统 SHALL NOT 在无显式点击时运行任何探针，SHALL NOT 提供手工改写 Confirmed 的旁路。SHALL NOT 存在按逻辑代码库分别维护或分别读取的 capability 记录。

#### Scenario: 点击核验补齐 Unknown
- **WHEN** provider 的 capability 行为 Unknown 且用户对该 provider 触发重验证
- **THEN** 系统 SHALL 执行真实现场探针，探针通过后将该 provider 的 action 行导入**项目级全局** durable Confirmed（launch/write_boundary/resume 由工件签发），随后全局作用域的 admission 读到 Confirmed

#### Scenario: 不点击不探测
- **WHEN** capability 行为 Unknown 且用户未触发重验证
- **THEN** 系统 SHALL NOT 运行任何 provider CLI 探针；admission 维持既有 fail-closed 拒绝

#### Scenario: Fake 与未知 provider 拒绝
- **WHEN** 重验证请求的 `provider_type` 是 Fake 或未知值
- **THEN** 系统 SHALL 返回稳定错误码（provider_capability_probe_unsupported），不执行任何探测、不写入 capability 记录

#### Scenario: 一次核验对全项目生效
- **WHEN** 用户对某 provider 完成一次重验证，随后在**另一个**逻辑代码库的上下文里触发该 provider 的准入判定
- **THEN** 准入 SHALL 读到**同一份**项目级全局记录里的 Confirmed；系统 SHALL NOT 要求对该逻辑代码库再核验一次，SHALL NOT 因该逻辑代码库的子树下没有 capability 文件而判定为未核验

### Requirement: 只读 capability 状态投影

系统 SHALL 提供 capability 状态 GET 面（路由仍按逻辑代码库定位，承载「哪个逻辑代码库的面板发起」与未知逻辑代码库的 404 守卫）：逐 provider 投影**项目级全局** durable action 矩阵（Confirmed/Denied/Unknown）、exact version、probed_at、probe_artifact_ref 与 provider 通道可用性（四家真实 provider 之外不出现）；该面零写入、零探针触发。投影内容 SHALL NOT 随所查询的逻辑代码库而变化。

#### Scenario: 状态面只读
- **WHEN** 用户读取 capability 状态
- **THEN** 系统 SHALL 返回当前 durable 事实且不运行任何 CLI、不修改任何记录

#### Scenario: 未探测投影 Unknown
- **WHEN** provider 尚无任何真实探针证据
- **THEN** 状态面 SHALL 投影全 Unknown（bootstrap 默认记录不得被当作真实证据显示为 Confirmed）

#### Scenario: 不同逻辑代码库投影同一份事实
- **WHEN** 用户先后在两个不同逻辑代码库的面板读取 capability 状态
- **THEN** 两次投影 SHALL 来自**同一份**项目级全局记录并给出相同结论；MUST NOT 出现「某个逻辑代码库已核验、另一个未核验」的分叉

### Requirement: 一键批量核验的异步作业入口（REQ-PCR-03）

系统 SHALL 提供批量核验入口 `POST /api/projects/{project_id}/logical-codebases/{lc_id}/capabilities/revalidate-all`（路由形状不变），其响应 SHALL **立即返回 `job_id`** 与初始作业状态，MUST NOT 在该响应内等待任何 provider 探针完成。批量核验的结果 SHALL 导入**项目级全局** capability 记录。

系统 SHALL 提供 `GET /api/projects/{project_id}/logical-codebases/{lc_id}/capabilities/revalidate-all/{job_id}` 返回作业状态与逐家结果。作业 SHALL 只由用户显式点击发起，MUST NOT 因聚合初始化失败或任何其它流程自动触发。

#### Scenario: 发起即返回作业号
- **WHEN** 用户触发批量核验
- **THEN** 系统 SHALL 立即返回 `job_id` 与 `running` 状态；MUST NOT 在 HTTP 响应里等待任何 provider 探针完成

#### Scenario: 轮询作业与逐项结果
- **WHEN** 客户端携带 `job_id` 轮询
- **THEN** 系统 SHALL 返回作业状态（`running` / `completed`）与逐家结果（`provider_type` / 状态 / 原因或各 action 结果）

#### Scenario: 不自动触发
- **WHEN** 聚合初始化或其它流程因 provider 能力未核验而失败
- **THEN** 系统 SHALL NOT 自动发起批量核验作业；MUST NOT 运行任何探针（沿用主规范「不点击不运行探针」契约）

#### Scenario: 批量核验结果落在全局记录
- **WHEN** 批量作业完成并导入某 provider 的探针结果
- **THEN** 该结果 SHALL 落在**项目级全局** capability 记录中；项目内任一逻辑代码库的准入判定 SHALL 读到同一份记录

### Requirement: 一键核验的全部入口与逐项进度（REQ-PCR-06）

能力卡 SHALL 提供「一键核验全部」按钮。点击后系统 SHALL 发起批量作业并按固定间隔（2~3 秒）轮询，作业终态后停止。进行中 SHALL 显示「核验中（N/4）」并逐项实时更新状态，按钮 SHALL 禁用。

失败项 SHALL 每行保留单项「重试」，其动作 SHALL 走既有单 provider 端点。`skipped`（CLI 未安装）项 SHALL 引导到该 provider 的【安装】按钮、版本落后项 SHALL 引导到【更新】按钮；安装/更新机制由块2 `provider-cli-installation` 定义（`REQ-PCI-02` / `REQ-PCI-05`），本 requirement MUST NOT 重新定义它。

按钮区域 SHALL 说明：核验是**项目级全局**的（**一次核验全项目可用**，不再逐逻辑代码库分别核验）、会真实跑一次探测并消耗 provider 额度、可能需要几分钟。

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
- **THEN** 该区域 SHALL 说明核验是**项目级全局**的、**一次核验全项目可用**、会真实跑一次探测、可能需要几分钟；MUST NOT 让用户误以为这是零成本的后台检查，MUST NOT 让用户误以为需要为每个逻辑代码库分别核验

#### Scenario: 终态停止轮询
- **WHEN** 作业到达终态（`completed`）
- **THEN** 前端 SHALL 停止轮询并渲染最终逐项结果；MUST NOT 无限轮询

## ADDED Requirements

### Requirement: 能力记录的项目级全局作用域（REQ-PCR-07）

provider capability 记录 SHALL 只有**一个**作用域：**项目级全局**（`projects/{project_id}/logical-codebase/capabilities.json`）。

- 写入 SHALL 一律落该全局文件。
- 读取 SHALL 一律读该全局文件。
- 系统 SHALL NOT 按逻辑代码库分别维护、读取或提升任何 capability 记录；SHALL NOT 存在「全局无记录时回退扫描各逻辑代码库旧记录」的读取分支。
- 能力是**本机 provider CLI 的事实**，与逻辑代码库的代码内容无关；因此一次核验 SHALL 对项目内全部逻辑代码库同时生效。
- capability 门本身的强度 SHALL NOT 改变：仍只有 `Confirmed` 放行，`Denied` / `Unknown` / 记录缺失 / 快照漂移一律 fail-closed；既有稳定错误码与文案形态 SHALL 保持不变。

#### Scenario: 写入只落项目级全局
- **WHEN** 任何路径导入或自举 provider capability 记录（探针导入、自举占位、外部入口导入）
- **THEN** 记录 SHALL 落在项目级全局文件；系统 SHALL NOT 在任何逻辑代码库子树下写出 capability 记录

#### Scenario: 读取只读项目级全局
- **WHEN** 任何路径读取 capability 记录（provider 选择门、gateway 准入、只读状态投影、批量作业前置）
- **THEN** 读取 SHALL 来自项目级全局文件；系统 SHALL NOT 读逻辑代码库子树下的 capability 文件，SHALL NOT 在全局无记录时回退扫描其它作用域

#### Scenario: 多逻辑代码库共用同一份记录
- **WHEN** 项目内存在两个及以上逻辑代码库，且用户对某 provider 完成一次核验
- **THEN** 两个逻辑代码库的准入判定 SHALL 读到**同一份**记录的同一结论；系统 SHALL NOT 要求对第二个逻辑代码库再核验一次

#### Scenario: 记录缺失仍按未核验处理
- **WHEN** 项目级全局 capability 文件不存在或该 provider 无记录
- **THEN** 判定 SHALL 为「未核验」并沿既有失败面给出可点击的核验入口；SHALL NOT 静默放行，SHALL NOT 改变既有错误码与文案形态

#### Scenario: 准入强度不变
- **WHEN** 某 provider 的某 action 行不是 `Confirmed`
- **THEN** gateway SHALL 继续 fail-closed 拒绝；MUST NOT 因作用域扩大而放行

#### Scenario: 不产生第二作用域
- **WHEN** 检查一个逻辑代码库子树
- **THEN** 该子树下 SHALL NOT 存在 capability 记录文件；若存在即为缺陷；系统 SHALL NOT 提供把该文件迁入全局记录的兼容路径
