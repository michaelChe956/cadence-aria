# logical-codebase-knowledge-base Specification

## Purpose
LC 仓管理内的知识库能力面：把 Cadence-skills 的 Schema 4.0 KnowledgeBase 接入平台，让 owner 能在 LC 内建立知识库、用显式变更包增量维护它、并在页面上阅读与编辑当前 LC 的知识库文档。

## Requirements

### Requirement: 知识库模块入口
LC 仓管理面板 SHALL 提供「知识库」模块，其中包含且仅包含三个用户可见按钮：【知识库初始化】【知识库更新】【查看知识库详细内容】。模块 SHALL 与既有「聚合初始化」「聚合索引」卡片同构（同层级渲染、同一视觉规范）。按钮的可用状态 SHALL 由当前 LC 的知识库状态决定，而非固定可见。

#### Scenario: LC 具备成员仓时知识库模块可见
- **WHEN** 当前 LC 的成员仓数量大于 0
- **THEN** LC 仓管理面板渲染「知识库」模块，含【知识库初始化】【知识库更新】【查看知识库详细内容】三个按钮

#### Scenario: LC 无成员仓时知识库模块不渲染
- **WHEN** 当前 LC 的成员仓数量为 0
- **THEN** LC 仓管理面板不渲染「知识库」模块（与聚合初始化卡的既有前置条件一致）

### Requirement: 知识库 provider 选择
知识库初始化与更新 SHALL 复用聚合初始化的 provider 选择器：候选顺序 MUST 为 Pi > Claude Code > Codex（不含 Kimi Code），判定 MUST 与聚合初始化同源（L1 命令可用 + L2 能力已核验），选定结果 MUST 冻结进 operation 且重试不换 provider。选择结论 MUST NOT 创建 operation；不可用时 SHALL 以聚合初始化既有的四枚失败码原样透出。

#### Scenario: 按 owner 顺序选中第一家已核验候选
- **WHEN** Pi 命令可用且能力已核验，Claude Code 与 Codex 亦可用
- **THEN** 知识库 operation 冻结的 provider 为 Pi，候选表按 Pi、Claude Code、Codex 顺序呈现

#### Scenario: 候选全未核验时不创建 operation
- **WHEN** 三家候选命令均可用但无一完成能力核验
- **THEN** 请求以 `aggregate_initialization_provider_needs_verification` 失败，details 含候选表与核验入口文案，且不产生知识库 operation

#### Scenario: 重试不更换 provider
- **WHEN** 知识库 operation 已冻结 provider 为 Pi 并进入失败态，用户重试
- **THEN** 重试沿用 Pi，冻结值不被改写

### Requirement: 知识库初始化前置检查
创建知识库初始化 operation 前，系统 MUST 依次只读校验：① 聚合初始化已完成（根规则与已发布政策存在）；② 被选 provider 对应 skill 层中 7 个 `knowledge-base-*` skill 全部存在；③ 六领域输入按领域状态条件齐全——`base-info.md` 必需，状态为「全量」或「指定」的领域其 scope 文件必需，状态为「不适用」的领域其 scope 文件不要求，`database-ddl.sql` 与 `product.md` 可选。任一项不满足 MUST 拒绝创建 operation 并给出可执行的补救指引。

#### Scenario: 缺少必填领域输入时给出补救路径
- **WHEN** `base-info.md` 声明数据模型状态为「全量」但 `data-model-scope.md` 不存在
- **THEN** 请求失败并列出缺失文件，同时给出「创建输入模板」的可执行入口，不产生 operation

#### Scenario: 领域状态为不适用时不误判输入不全
- **WHEN** `base-info.md` 声明接口领域状态为「不适用」且 `api-scope.md` 不存在，其余必需输入齐全
- **THEN** 前置检查通过，不因缺少 `api-scope.md` 判输入不全

#### Scenario: 缺少任一知识库 skill 时拒绝执行
- **WHEN** 被选 provider 的 skill 层缺少 `knowledge-base-overview`
- **THEN** 请求失败并说明知识库 skill 不可用，指向安装或更新 Cadence-skills，不产生 operation

### Requirement: 知识库初始化执行
知识库初始化 SHALL 由选定 provider 以单个 provider turn 执行 `/knowledge-base-bootstrap` 命令，cwd MUST 为 canonical 聚合根。系统 MUST 记录命令前后聚合根的写入面差分，允许写入面 MUST 含知识库目录、项目规则目录与聚合根的代理规则入口文件；越界写入 MUST 判 operation 失败。系统 MUST NOT 代写用户的六领域输入。

#### Scenario: 初始化产物落在聚合根知识库目录
- **WHEN** 知识库初始化 operation 成功完成
- **THEN** 聚合根下存在知识库目录，含 manifest、输入清单与各领域文档，且 operation 状态为 completed

#### Scenario: 越界写入判失败
- **WHEN** provider turn 在允许写入面之外改动了聚合根内其它成员仓的文件
- **THEN** operation 失败并以写入面越界稳定码记录失败事实

### Requirement: 知识库更新的变更包门禁
执行知识库更新前，系统 MUST 校验：① Manifest 存在且 `schema_version` 为 `4.0`；② `coverage.initialization` 不变量合法且状态为 complete、global validation 为 passed；③ 变更包目录位于知识库的 updates 目录下、目录名以 `CHANGE-` 开头且标识只含安全字符；④ 五份固定文档齐全且非空；⑤ 必填字段非空，未二选一的枚举占位视为未填；⑥ 「不适用」必须带具体原因；⑦ 七行领域矩阵齐全、状态只允许「有变更」或「无变更」、每个「无变更」须有非空判断依据；⑧ 变更包与附件不得含疑似明文凭证、连接串或未脱敏内部地址；⑨ 代码变更声明为「有变更」时起止提交必须能在本地成员仓解析。系统 MUST 只在机械可判定的范围内门禁，语义一致性 MUST 交回知识库 skill 判定。

#### Scenario: 字段未填时列出逐条问题并禁止执行
- **WHEN** 变更包的代码变更文档中「Merge Request 地址或编号」为空
- **THEN** 门禁结果给出该文档该字段的未填问题，更新操作被拒绝执行，且不产生 operation

#### Scenario: 枚举占位视为未填
- **WHEN** 变更摘要的代码行状态写成「有变更 / 无变更」而未二选一
- **THEN** 门禁结果判该行为未填并列出问题

#### Scenario: 疑似明文凭证阻断更新
- **WHEN** 变更包某份文档含形如密钥赋值的明文
- **THEN** 门禁结果给出敏感内容问题并指明命中位置（不回显命中值本身），更新操作被拒绝执行

#### Scenario: 起止提交无法解析时阻断更新
- **WHEN** 变更包声明的结束提交在本地成员仓无法解析
- **THEN** 门禁结果给出提交无法解析问题，更新操作被拒绝执行

#### Scenario: 未完成初始化的知识库不得更新
- **WHEN** Manifest 的初始化状态为 in_progress
- **THEN** 更新入口不可用并说明需先完成知识库初始化

### Requirement: 知识库更新执行
知识库更新 SHALL 由选定 provider 以单个 provider turn 执行 `/knowledge-base-update` 命令，命令参数 MUST 为用户显式指定的变更包目录路径（相对聚合根），系统 MUST NOT 自动选择 updates 目录下的最新目录，provider MUST NOT 代写变更包正文。

#### Scenario: 按用户指定路径调用更新
- **WHEN** 用户选定变更包 CHANGE-2026-1109-登录改造 且门禁通过
- **THEN** operation 冻结的变更包路径为该目录，provider turn 以该路径作为命令参数执行更新

#### Scenario: 更新失败不改动知识库文档
- **WHEN** 更新 turn 在门禁或校验阶段失败
- **THEN** operation 记录失败事实，且知识库文档内容保持更新前状态

### Requirement: 模板骨架创建
系统 SHALL 提供从 Cadence-skills 插件复制模板骨架的能力：知识库初始化输入模板共 9 份，知识库更新变更包模板共 5 份外加附件说明。创建 MUST 只补充缺失文件，MUST NOT 覆盖已存在文件；变更包目录标识 MUST 由用户填写，系统只做前缀归一与安全字符校验；同一标识重复创建 MUST 返回既有目录且不覆盖。

#### Scenario: 创建输入模板只补缺失文件
- **WHEN** 用户输入目录中已存在填写过的 base-info.md，其余模板缺失
- **THEN** 创建后新增其余模板文件，base-info.md 内容保持不变

#### Scenario: 重复创建同一变更包不覆盖
- **WHEN** 变更包 CHANGE-x 目录已存在，用户再次以标识 x 创建
- **THEN** 系统返回已存在提示且目录内容保持不变

### Requirement: 知识库状态投影
系统 SHALL 提供当前 LC 的知识库状态查询，投影出知识库是否存在、Manifest 版本、初始化状态、全局校验状态、已完成阶段与跳过阶段。Manifest 解析 MUST 在服务端完成，前端 MUST NOT 自行解析 Manifest 文件。

#### Scenario: 未初始化时状态为不存在
- **WHEN** 聚合根下不存在知识库目录
- **THEN** 状态投影返回不存在，更新入口不可用并说明需先完成初始化

#### Scenario: 初始化完成时状态可读
- **WHEN** Manifest 存在且初始化状态为 complete、全局校验为 passed
- **THEN** 状态投影返回版本、状态与阶段清单，前端据此启用更新入口

### Requirement: 知识库文档读取与编辑保存
系统 SHALL 提供当前 LC 知识库目录的文档树、单文件读取与保存。路径 MUST 为相对路径，解析后 MUST 仍在知识库根内（禁目录穿越、绝对路径与符号链接逃逸）。写入白名单 MUST 为全域 Markdown 文件与用户输入目录下的 SQL 文件；机器管理的清单与派生关系文件 MUST NOT 可写。保存 SHALL 采用乐观并发：客户端携带读取时的内容摘要，服务端当前摘要不符时 MUST 拒绝覆盖。知识库 operation 运行期间 MUST 拒绝对该目录的保存。

#### Scenario: 保存被并发修改覆盖时拒绝
- **WHEN** 用户读取文件后另一会话先保存了同一文件，用户随后保存
- **THEN** 服务端以内容冲突拒绝该次保存，不覆盖已有内容

#### Scenario: 目录穿越路径被拒绝
- **WHEN** 请求读取或写入的相对路径解析后落在知识库根之外
- **THEN** 请求被拒绝且不发生任何读写

#### Scenario: 机器管理文件不可编辑
- **WHEN** 用户打开知识库清单文件或派生关系文件
- **THEN** 界面以只读呈现并说明其为机器管理文件，不提供编辑入口

#### Scenario: 知识库操作运行中禁止保存
- **WHEN** 知识库 operation 处于运行中，用户在详情页保存文件
- **THEN** 保存请求被拒绝并提示知识库操作进行中

### Requirement: 知识库 operation 单飞与取消
同一逻辑代码库的 bootstrap 与 update SHALL 互斥：任一知识库 operation 处于运行中时，创建新的知识库 operation MUST 被拒绝。运行中的 operation MUST 可被取消，取消后 MUST 停止后续执行。operation MUST 持久化在按逻辑代码库隔离的目录下，operation 标识 MUST 包含逻辑代码库身份。

#### Scenario: 重复点击不启动第二次
- **WHEN** 同一逻辑代码库已有知识库 operation 在运行，用户再次点击初始化
- **THEN** 新 operation 不被创建，界面提示已有操作在运行

#### Scenario: 取消后停止后续执行
- **WHEN** 用户取消运行中的知识库 operation
- **THEN** operation 进入取消终态且不再继续执行后续步骤

### Requirement: 失败面与稳定码登记
知识库新增的稳定码 MUST 在平台的失败文案单一来源中登记中文成品文案（含缺失文件、违规项等明细字段），并 MUST 在 HTTP 状态映射中登记为 4xx 或 422，不得落到通用 500。前置检查失败（未产生 operation）时，失败明细 MUST 作为请求期响应返回并可在界面上呈现；已开始执行的失败 MUST 走 operation 的失败事实。

#### Scenario: 前置检查失败的明细可见
- **WHEN** 初始化前置检查因输入缺失而失败
- **THEN** 界面呈现失败码对应的中文文案与缺失文件清单，而不是通用兜底文案或 500

### Requirement: 聚合初始化的 provider 依赖只覆盖被选中的那一家
聚合初始化 SHALL 允许仅安装一家 provider 即完成：provider 选择器 MUST 按 owner 顺序选中第一家已核验且可用的 provider（Pi > Claude Code > Codex），系统 MUST NOT 要求四家 provider 全部安装。Cadence-skills 四条命令的链接层校验 MUST 只覆盖**被选中 provider 对应的那一层**（Pi → shared、Claude Code → claude、Codex → codex）；未被选中、且其命令行未安装的 provider 的层缺失 MUST NOT 阻断本次聚合初始化。被选中 provider 那一层缺命令时，系统 MUST 以 `skills_commands_missing` 失败。

#### Scenario: 只装 Pi 也能通过链接层校验
- **WHEN** 机器上只安装了 Pi，shared 层具备四条 Cadence-skills 命令，而 codex 层与 claude 层缺失
- **THEN** 聚合初始化的链接层校验通过，不因 codex / claude 层缺失而失败

#### Scenario: 选中 provider 的层缺失仍被拒绝
- **WHEN** 被选中的 provider 对应的那一层缺少某条 Cadence-skills 命令
- **THEN** 聚合初始化以 `skills_commands_missing` 失败，且不把任何 step 标为失败

### Requirement: 知识库操作的无人值守执行
知识库 operation SHALL 作为无人值守任务执行：provider turn 的 prompt MUST 附带无人值守指令（不要发起提问或授权请求、不要等待回答），遇到需要用户决策的分支按 skill 规定的非破坏性默认路径继续，无法安全继续时停止并在产物中记录。provider 发起提问或请求授权时，系统 MUST 以 `knowledge_base_provider_interaction_required` 失败，MUST NOT 归因为通用的「provider 执行失败」，且 MUST 把 provider 问了什么写入失败诊断。

#### Scenario: provider 发问有独立稳定码与可执行文案
- **WHEN** 知识库 operation 的 provider turn 发起提问或请求授权
- **THEN** 失败记录的稳定码为 `knowledge_base_provider_interaction_required`，诊断里含 provider 的提问原文，界面文案说明「无人值守执行无法回答」并给出补输入或换 provider 的下一步

#### Scenario: prompt 带无人值守指令
- **WHEN** 知识库 operation 向 provider 发起 turn
- **THEN** prompt 由命令与无人值守指令组成，指令明确要求不要提问、不要请求授权、不要等待回答

### Requirement: 聚合初始化失败面的 provider 口径
聚合初始化失败面 SHALL 明确「一家可用 provider 即可执行」的口径：重新检测后若存在可用 provider，界面 MUST 说明已具备执行条件，并把未安装或不可用的 provider 表述为不影响本次执行的可选项，MUST NOT 把它们表述为必须先安装的阻塞项；仅当没有任何可用 provider 时，MUST 说明至少需要一家可用。环境类失败的诊断折叠标题 MUST 点明其中承载真实失败原因。

#### Scenario: 一家可用时重新检测结论不制造阻塞错觉
- **WHEN** 用户点「重新检测」，检测结果里有一家可用、另有两家未安装
- **THEN** 反馈说明已具备执行条件，并说明未安装的两家不影响本次执行；反馈中不出现「按安装指引处理后点重试」这类把未安装家说成必修项的措辞

#### Scenario: 一家都不可用时说明至少需要一家
- **WHEN** 重新检测后没有任何可用 provider
- **THEN** 反馈说明至少要有一家可用才能执行，并给出对应的安装指引


### Requirement: 聚合初始化 provider 顺序提示的视觉修正
【聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。】这行提示在 Provider 管理面板中 MUST 与上下容器保持内边距，且其视觉形态 MUST 与聚合初始化卡片中的同一提示一致；文案与可测试标识 MUST NOT 改变。

#### Scenario: Provider 管理面板提示不再贴边
- **WHEN** 用户打开 Provider 管理面板
- **THEN** 该行提示与上下元素之间存在可见内边距，且与聚合初始化卡片中的同名提示样式一致

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
