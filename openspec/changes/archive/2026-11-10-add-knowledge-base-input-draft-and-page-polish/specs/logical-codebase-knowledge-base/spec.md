# Spec Delta

## ADDED Requirements

### Requirement: 知识库全域聚合初始化前置门
系统 SHALL 在**所有**知识库端点（状态、文件树、文件读取、文件写入、模板创建、变更包列表、变更包 lint、输入清单、输入确认、创建初始化/更新/起草 operation）入口统一校验：当前逻辑代码库的冷启动投影中「规则/政策」步状态为 `completed`。任一端点在校验不通过时 MUST 拒绝请求并返回稳定码 `knowledge_base_requires_aggregate_initialization` 与中文可执行文案（指明需先完成聚合初始化），且 MUST NOT 发生任何写副作用。判定失败（投影读不出、IO 故障、LC 归属不明）MUST 按「未完成」处理。

#### Scenario: 聚合初始化未完成时读取知识库文件被拒绝
- **WHEN** 当前 LC 的「规则/政策」步不是 completed，用户请求读取任一知识库文件
- **THEN** 请求以 `knowledge_base_requires_aggregate_initialization` 失败，message 为中文可执行文案，且不返回文件内容

#### Scenario: 聚合初始化未完成时写入知识库文件被拒绝
- **WHEN** 当前 LC 的「规则/政策」步不是 completed，用户请求写入知识库文件
- **THEN** 请求以 `knowledge_base_requires_aggregate_initialization` 失败，且磁盘内容不变

#### Scenario: 聚合初始化完成后知识库可用
- **WHEN** 当前 LC 的「规则/政策」步为 completed
- **THEN** 知识库的读取、写入、模板与变更包端点正常服务，不再返回该前置失败码

### Requirement: 输入草稿生成与两段式初始化
当 `user-input/` 下的知识库输入**未准备**（9 份输入文件缺失，或全部与插件模板逐字相同即仍是未填写的占位）时，【知识库初始化】MUST 改为用选定的 provider 执行 `/knowledge-base-user-input` skill 在 canonical 聚合根起草这 9 份输入，MUST NOT 由平台拷贝模板充数。起草 MUST 只写 `cadence/knowledge-base/user-input/` 目录。起草完成后系统 MUST 停在「待用户确认」状态，MUST NOT 继续执行 `/knowledge-base-bootstrap`。当输入已准备但尚未全部确认时，创建初始化 operation MUST 被拒绝并列出未确认文件，MUST NOT 产生任何 operation。全部确认后，同一个【知识库初始化】动作 MUST 执行 `/knowledge-base-bootstrap`。

#### Scenario: 输入未准备时初始化触发起草
- **WHEN** `user-input/` 下 9 份输入文件缺失或全部仍是未填写的模板占位，用户点击【知识库初始化】
- **THEN** 系统创建起草 operation 并执行 `/knowledge-base-user-input`，该 operation 完成后系统停在「待确认」状态，不执行 `/knowledge-base-bootstrap`

#### Scenario: 起草只写 user-input 目录
- **WHEN** 起草 operation 执行完成
- **THEN** `cadence/knowledge-base/` 下除 `user-input/` 外没有新增任何文件

#### Scenario: 未确认时不允许执行初始化 skill
- **WHEN** 输入已准备但仍有文件未确认，用户点击【知识库初始化】
- **THEN** 请求以 `knowledge_base_user_input_not_confirmed` 失败，details 列出未确认文件，且不产生任何 operation

#### Scenario: 全部确认后执行初始化 skill
- **WHEN** 所有输入文件均已确认，用户点击【知识库初始化】
- **THEN** 系统创建初始化 operation 并执行 `/knowledge-base-bootstrap`

### Requirement: 用户确认状态与失效语义
系统 SHALL 提供知识库输入的逐份确认能力：确认记录 MUST 持久化在平台侧（按逻辑代码库隔离），MUST NOT 写入 `user-input/` 目录。确认时服务端 MUST 重新计算该文件当前内容的 sha256 并与请求携带的摘要比对，不一致 MUST 拒绝（过期确认）。查询确认状态时 MUST 逐份比对记录摘要与文件当前摘要，不一致 MUST 视为该份**未确认**（编辑后确认自动失效）。系统 SHALL 提供批量确认入口，且 MUST NOT 在存在未确认文件时允许进入初始化 skill。

#### Scenario: 逐份确认后状态为已确认
- **WHEN** 用户对某份输入文件提交确认且携带的摘要与服务端当前摘要一致
- **THEN** 该份状态变为已确认，确认记录持久化到平台侧存储，`user-input/` 目录内没有新增文件

#### Scenario: 过期摘要确认被拒
- **WHEN** 用户提交确认时携带的摘要与服务端当前文件摘要不一致
- **THEN** 请求以 `knowledge_base_user_input_confirmation_stale` 失败，不写入确认记录

#### Scenario: 编辑已确认文件后确认自动失效
- **WHEN** 某份文件已被确认，随后其内容被编辑
- **THEN** 该份状态重新显示为待确认，初始化入口保持不可用

#### Scenario: 批量确认
- **WHEN** 用户提交批量确认且各文件摘要均与服务端当前一致
- **THEN** 这些文件全部变为已确认

### Requirement: 知识库详情页工作区形态
知识库详情页 SHALL 呈现为全屏工作区：顶部 SHALL 有信息栏，包含知识库标题、所属项目与逻辑代码库的关联信息、当前状态徽标，以及返回工作台的入口按钮；该页面 SHALL NOT 显示全局设置触发器。页面 SHALL 为左侧文件树与右侧编辑/预览两栏自适应布局，右侧编辑区 SHALL 占满可用高度，页面下半屏 MUST NOT 出现无内容空白。机器管理文件的只读呈现 MUST 保持不变。

#### Scenario: 详情页呈现关联信息与返回入口
- **WHEN** 用户打开某逻辑代码库的知识库详情页
- **THEN** 顶部信息栏显示知识库标题、所属项目名与逻辑代码库名、状态徽标，并提供返回工作台的按钮

#### Scenario: 详情页不显示全局设置触发器
- **WHEN** 用户停留在知识库详情页
- **THEN** 页面上不存在全局设置触发器控件

#### Scenario: 编辑区占满可用高度
- **WHEN** 用户在知识库详情页打开一个文件
- **THEN** 编辑器区域占满右侧栏的可用高度，页面下半屏没有空白区域

#### Scenario: 机器管理文件仍为只读
- **WHEN** 用户在详情页打开被标记为不可编辑的文件
- **THEN** 该文件以只读呈现并说明其为机器管理文件，不提供编辑与保存入口
