# Spec Delta

## ADDED Requirements

### Requirement: 知识库初始化的交互式两段流程
系统 SHALL 把「起草完成」到「知识库建好」这一段实现为可交互的闭环：起草完成后系统 MUST 给出打开输入文件的入口（卡片上的【打开输入文件并补充】），并 MUST 在起草刚完成时进入知识库详情页、打开第一份待确认的 `user-input/` 文件（右侧可写编辑器，与【查看知识库详细内容】同一形态）。详情页的输入确认区 MUST 给出可执行的四步操作指引、确认进度与未填阻塞说明。输入确认区 SHALL 提供【确认并初始化】动作：先批量确认全部待确认文件，随即执行 `/knowledge-base-bootstrap`；建库运行中 MUST 显示进度，完成后 MUST 刷新文件树与状态投影并停留在详情页，用户对知识库文档的后续修改以用户的落盘内容为准。

#### Scenario: 起草完成后卡片给出下一步入口
- **WHEN** 某个逻辑代码库的知识库起草 operation（`kind=user_input`）进入 completed 终态
- **THEN** 知识库卡片呈现【打开输入文件并补充】入口与下一步说明（提示补充并确认后才会建库），用户点击该入口进入知识库详情页

#### Scenario: 起草刚完成时自动打开输入文件
- **WHEN** 用户在当前逻辑代码库的卡片上发起起草，且起草在用户仍停留在当前工作台期间进入 completed 终态
- **THEN** 系统进入知识库详情页，并打开第一份待确认的 `user-input/` 文件（右侧可写编辑器）

#### Scenario: 进入知识库详情页的导航不被顶回
- **WHEN** 用户在逻辑代码库管理抽屉里点击【打开输入文件并补充】或【查看知识库详细内容】
- **THEN** 浏览器地址落到知识库详情页路由并保持在该页（工作台抽屉焦点同步不得用 `replace` 把地址顶回工作台）

#### Scenario: 输入确认区给出操作指引与阻塞说明
- **WHEN** 用户在知识库详情页看到输入确认区
- **THEN** 该区展示四步操作指引（逐份核对补充 → 改完确认 → 全部确认 → 执行初始化）、确认进度（已确认数/总份数），以及存在未填写文件时的阻塞说明（未填文件无法被确认，需先在编辑器里补充）

#### Scenario: 确认并初始化直接执行建库
- **WHEN** 用户在输入确认区点击【确认并初始化】，且全部已填写文件均已确认
- **THEN** 系统批量确认待确认文件并创建 `/knowledge-base-bootstrap` operation，用户无需再回到卡片触发第二次初始化

#### Scenario: 存在未填写文件时不允许建库
- **WHEN** 仍有输入文件未填写（内容与模板逐字相同）
- **THEN** 【确认并初始化】处于禁用态并说明原因（未填文件无法确认），系统不创建任何 operation

#### Scenario: 建库完成后停留在详情页
- **WHEN** `/knowledge-base-bootstrap` operation 进入 completed 终态
- **THEN** 知识库详情页刷新文件树与状态投影，知识库文档出现在文件树中，页面给出可继续编辑的提示，且用户停留在详情页

## MODIFIED Requirements

### Requirement: 输入草稿生成与两段式初始化
当 `user-input/` 下的知识库输入**未准备**（9 份输入文件缺失，或全部与插件模板逐字相同即仍是未填写的占位）时，【知识库初始化】MUST 改为用选定的 provider 执行 `/knowledge-base-user-input` skill 在 canonical 聚合根起草这 9 份输入，MUST NOT 由平台拷贝模板充数。起草 MUST 只写 `cadence/knowledge-base/user-input/` 目录。起草完成后系统 MUST 停在「待用户确认」状态，MUST NOT 继续执行 `/knowledge-base-bootstrap`。当输入已准备但尚未全部确认时，创建初始化 operation MUST 被拒绝并列出未确认文件，MUST NOT 产生任何 operation。全部确认后，【知识库初始化】动作或输入确认区的【确认并初始化】动作 MUST 执行 `/knowledge-base-bootstrap`。

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
- **WHEN** 所有输入文件均已确认，用户点击【知识库初始化】或输入确认区的【确认并初始化】
- **THEN** 系统创建初始化 operation 并执行 `/knowledge-base-bootstrap`
