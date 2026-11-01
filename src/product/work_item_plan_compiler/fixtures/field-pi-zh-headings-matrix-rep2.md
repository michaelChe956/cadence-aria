# 工作项计划

## 工作项 WI-001: 后端关卡数据 API 与静态托管服务

### 身份信息
- schema_version: 1
- logical_work_item_id: WI-001
- title: 后端关卡数据 API 与静态托管服务
- kind: backend

### 目标
- summary: WHEN 服务经 createServer(options) 构建并由 start({ port }) 启动 THE SYSTEM SHALL 在同一进程内提供符合 CT-001 的 GET /api/levels 与以 staticRoot 为边界的静态页面托管。

### 非目标
- non_goals: 不实现关卡玩法逻辑与任何游戏交互行为。
- non_goals: 不引入 npm 依赖、数据库、认证或外部服务，仅使用 node:http、node:fs、node:path。
- non_goals: 不读取或依赖 web/ 目录内容，静态托管验收仅使用 tests/backend 自有夹具。

### 依赖关系
- depends_on: []

### 输入

### 输出
- contract_id: CT-001
- capabilities: GET /api/levels 返回 200，Content-Type 为 application/json; charset=utf-8，主体为恰好包含 5 个关卡的顶层数组
- capabilities: GET /api/levels 的项目包含唯一的非空字符串 id、非空字符串 name、取值集合为 {简单, 普通, 困难} 的 difficulty 以及 boolean 类型的 unlocked
- capabilities: GET /api/levels 数据失败时返回 500，主体为 JSON 格式的 error.code LEVEL_DATA_UNAVAILABLE 以及可读消息
- capabilities: API 错误响应携带 Content-Type application/json; charset=utf-8，主体为 { error: { code, message } }
- contract_id: CT-002
- capabilities: createServer(options) 接受 dataPath（默认为 data/levels.json）和 staticRoot（默认为 web/）
- capabilities: start({ port }) 解析为 { origin, port, stop }，origin 为 http://127.0.0.1:<actual port>
- capabilities: start({ port: 0 }) 绑定临时端口以进行测试隔离
- capabilities: stop() 返回终止服务器的 Promise
- contract_id: CT-003
- capabilities: GET / 从 staticRoot 提供 web/index.html 页面入口
- capabilities: 静态文件解析拒绝使用 node:path normalize 和 resolve 逃逸 staticRoot 的路径
- capabilities: staticRoot 下的静态资源以匹配的 Content-Type 提供
- capabilities: 缺失的静态资源返回 404 文本响应，区别于 API JSON 错误主体

### 任务
- task_id: TASK-001
- statement: WHEN 创建 data/levels.json THE SYSTEM SHALL 写入恰好 5 个关卡对象且字段满足 CT-001 的字段约束。
- requirement_refs: REQ-003
- done_when_refs: AC-001
- task_id: TASK-002
- statement: WHEN 编写 tests/backend 契约测试 THE SYSTEM SHALL 以 fixtures 注入 dataPath 与 staticRoot 覆盖数据成功、数据缺失、JSON 解析失败、结构校验失败、API 404 与 405、静态托管安全及 port 0 生命周期场景。
- requirement_refs: REQ-002, REQ-004, NFR-001
- done_when_refs: AC-001, AC-002, AC-003, AC-004, AC-005
- task_id: TASK-003
- statement: WHEN 实现 server/server.js THE SYSTEM SHALL 导出 createServer(options) 与 start({ port }) 并按 CT-001、CT-002、CT-003 提供路由、数据校验、静态托管与错误语义使后端测试全部通过。
- requirement_refs: REQ-002, REQ-003, REQ-004
- done_when_refs: AC-001, AC-002, AC-003, AC-004, AC-005

### 编写策略
- exclusive_scopes: server/**, data/levels.json, tests/backend/**
- forbidden_scopes: web/**, tests/frontend/**, tests/integration/**

### 验收标准
- criterion_id: AC-001
- statement: WHEN GET /api/levels 且数据文件有效 THE SYSTEM SHALL 返回 200、Content-Type application/json; charset=utf-8 及恰好 5 项且字段满足 CT-001 约束的顶层数组。
- required_evidence: non_zero_test_execution
- criterion_id: AC-002
- statement: WHEN dataPath 指向缺失文件、无法解析 JSON 或结构校验失败的夹具 THE SYSTEM SHALL 返回 500 且 JSON 体 error.code 为 LEVEL_DATA_UNAVAILABLE 且 message 面向用户可读。
- required_evidence: non_zero_test_execution
- criterion_id: AC-003
- statement: WHEN 请求未知 API 路径或非允许方法 THE SYSTEM SHALL 返回 404 或 405 的 JSON 错误体且 Content-Type 为 application/json; charset=utf-8。
- required_evidence: non_zero_test_execution
- criterion_id: AC-004
- statement: WHEN staticRoot 指向 tests/backend 夹具目录 THE SYSTEM SHALL 对 GET / 返回夹具页面、按扩展名返回匹配 Content-Type、对缺失静态资源返回 404 文本响应并拒绝解析到静态根之外的路径。
- required_evidence: non_zero_test_execution
- criterion_id: AC-005
- statement: WHEN 以 port 0 调用 start THE SYSTEM SHALL 解析出包含实际 origin、port 与 stop 的结果，且 stop() 返回 Promise 并终止监听。
- required_evidence: non_zero_test_execution

### 验证
- check_id: CHECK-001
- command: node --test tests/backend/
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true

### 交接模式 (Handoff Schema)
- required_fields: createServer(options), start({ port }), stop(), origin, port, dataPath, staticRoot
- provided_contract_refs: CT-001, CT-002, CT-003
- reviewer_check_refs: AC-001, AC-002, AC-003, AC-004, AC-005

### 阻塞项

### 可追溯性
- source_type: issue
- source_id: issue_workitem_0007#api
- requirement_id: REQ-002
- source_type: issue
- source_id: issue_workitem_0007#data
- requirement_id: REQ-003
- source_type: issue
- source_id: issue_workitem_0007#api
- requirement_id: REQ-004
- source_type: issue
- source_id: issue_workitem_0007#verify
- requirement_id: NFR-001

## 工作项 WI-002: 前端关卡选择页与 DOM 替身测试

### 身份信息
- schema_version: 1
- logical_work_item_id: WI-002
- title: 前端关卡选择页与 DOM 替身测试
- kind: frontend

### 目标
- summary: WHEN 页面加载并调用 initLevelSelect({ document, fetchImpl }) THE SYSTEM SHALL 经相对路径 /api/levels 获取数据并在 level-list 渲染名称与难度，在请求失败或响应无效时于 error-message 展示可读错误且不白屏。

### 非目标
- non_goals: 不实现关卡玩法逻辑与选中关卡后的行为。
- non_goals: 不引入浏览器自动化、npm 依赖或测试框架。
- non_goals: 不启动后端服务，不修改 server/**、data/levels.json 与 tests/backend/**。

### 依赖关系
- depends_on: WI-001

### 输入
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: GET /api/levels 返回 200，Content-Type 为 application/json; charset=utf-8，主体为恰好包含 5 个关卡的顶层数组
- required_capabilities: GET /api/levels 的项目包含唯一的非空字符串 id、非空字符串 name、取值集合为 {简单, 普通, 困难} 的 difficulty 以及 boolean 类型的 unlocked
- required_capabilities: GET /api/levels 数据失败时返回 500，主体为 JSON 格式的 error.code LEVEL_DATA_UNAVAILABLE 以及可读消息
- required_capabilities: API 错误响应携带 Content-Type application/json; charset=utf-8，主体为 { error: { code, message } }
- compatibility_policy: require_all

### 输出
- contract_id: CT-004
- capabilities: web/index.html 包含 id=loading, id=level-list, id=error-message 容器
- capabilities: web/index.html 引用 level-select.js 脚本
- capabilities: web/level-select.js 通过相对路径引用 /api/levels
- capabilities: web/level-select.js 导出 initLevelSelect({ document, fetchImpl })
- capabilities: initLevelSelect 获取 /api/levels 并将每个关卡的名称和难度渲染到 level-list 容器
- capabilities: initLevelSelect 将 unlocked=false 的关卡渲染为锁定且不可选
- capabilities: initLevelSelect 在 error-message 中显示可读的错误消息，并在获取失败、非 200 状态或响应结构无效时保持页面框架

### 任务
- task_id: TASK-004
- statement: WHEN 编写 web/index.html THE SYSTEM SHALL 提供 id=loading、id=level-list、id=error-message 三个容器并以 script 引用 level-select.js。
- requirement_refs: REQ-005
- done_when_refs: AC-006
- task_id: TASK-005
- statement: WHEN 编写 tests/frontend 测试 THE SYSTEM SHALL 以注入 document 与 fetchImpl 的最小 DOM 替身驱动 initLevelSelect 覆盖成功渲染、锁定态、结构无效与请求失败分支。
- requirement_refs: REQ-005, REQ-006, NFR-001
- done_when_refs: AC-007, AC-008
- task_id: TASK-006
- statement: WHEN 实现 web/level-select.js THE SYSTEM SHALL 导出 initLevelSelect({ document, fetchImpl }) 并按 CT-001 契约请求与校验 /api/levels 使前端测试全部通过。
- requirement_refs: REQ-005, REQ-006
- done_when_refs: AC-007, AC-008

### 编写策略
- exclusive_scopes: web/**, tests/frontend/**
- forbidden_scopes: server/**, data/levels.json, tests/backend/**, tests/integration/**

### 验收标准
- criterion_id: AC-006
- statement: WHEN 解析 web/index.html THE SYSTEM SHALL 含 id=loading、id=level-list、id=error-message 三个容器标记并以 script 引用 level-select.js。
- required_evidence: non_zero_test_execution
- criterion_id: AC-007
- statement: WHEN initLevelSelect 以 DOM 替身收到满足 CT-001 的 5 项响应 THE SYSTEM SHALL 在 level-list 渲染每项名称与难度且 unlocked=false 项呈锁定不可选状态。
- required_evidence: non_zero_test_execution
- criterion_id: AC-008
- statement: WHEN fetchImpl 请求失败、返回非 200 或结构不满足 CT-001 约束 THE SYSTEM SHALL 在 error-message 显示可读错误提示且保留页面框架不白屏。
- required_evidence: non_zero_test_execution

### 验证
- check_id: CHECK-002
- command: node --test tests/frontend/
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true

### 交接模式 (Handoff Schema)
- required_fields: web/index.html, web/level-select.js, initLevelSelect({ document, fetchImpl }), id=loading, id=level-list, id=error-message
- provided_contract_refs: CT-004
- reviewer_check_refs: AC-006, AC-007, AC-008

### 阻塞项

### 可追溯性
- source_type: issue
- source_id: issue_workitem_0007#frontend
- requirement_id: REQ-005
- source_type: issue
- source_id: issue_workitem_0007#frontend
- requirement_id: REQ-006
- source_type: issue
- source_id: issue_workitem_0007#verify
- requirement_id: NFR-001

## 工作项 WI-003: 前后端同源联调集成验证

### 身份信息
- schema_version: 1
- logical_work_item_id: WI-003
- title: 前后端同源联调集成验证
- kind: integration

### 目标
- summary: WHEN 以 start({ port: 0 }) 启动 WI-001 服务并访问其托管的页面与 API THE SYSTEM SHALL 在 tests/integration 产生页面入口同源可达、API 契约形状与错误体符合契约的可执行证据。

### 非目标
- non_goals: 不修改 server/**、web/**、data/levels.json、tests/backend/**、tests/frontend/** 内任何实现或测试。
- non_goals: 不重复运行时渲染与失败降级行为断言，该职责归属前端测试。

### 依赖关系
- depends_on: WI-001
- depends_on: WI-002

### 输入
- contract_id: CT-002
- provider_logical_work_item_id: WI-001
- required_capabilities: createServer(options) 接受 dataPath（默认为 data/levels.json）和 staticRoot（默认为 web/）
- required_capabilities: start({ port }) 解析为 { origin, port, stop }，origin 为 http://127.0.0.1:<actual port>
- required_capabilities: start({ port: 0 }) 绑定临时端口以进行测试隔离
- required_capabilities: stop() 返回终止服务器的 Promise
- compatibility_policy: require_all
- contract_id: CT-003
- provider_logical_work_item_id: WI-001
- required_capabilities: GET / 从 staticRoot 提供 web/index.html 页面入口
- required_capabilities: staticRoot 下的静态资源以匹配的 Content-Type 提供
- compatibility_policy: require_all
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: GET /api/levels 返回 200，Content-Type 为 application/json; charset=utf-8，主体为恰好包含 5 个关卡的顶层数组
- required_capabilities: GET /api/levels 的项目包含唯一的非空字符串 id、非空字符串 name、取值集合为 {简单, 普通, 困难} 的 difficulty 以及 boolean 类型的 unlocked
- required_capabilities: GET /api/levels 数据失败时返回 500，主体为 JSON 格式的 error.code LEVEL_DATA_UNAVAILABLE 以及可读消息
- required_capabilities: API 错误响应携带 Content-Type application/json; charset=utf-8，主体为 { error: { code, message } }
- compatibility_policy: require_all
- contract_id: CT-004
- provider_logical_work_item_id: WI-002
- required_capabilities: web/index.html 包含 id=loading, id=level-list, id=error-message 容器
- required_capabilities: web/index.html 引用 level-select.js 脚本
- required_capabilities: web/level-select.js 通过相对路径引用 /api/levels
- compatibility_policy: require_all

### 输出
- contract_id: CT-005
- capabilities: tests/integration 套件验证页面入口和 /api/levels 的同源可达性
- capabilities: tests/integration 套件验证注入无效 dataPath 时的 500 错误主体结构

### 任务
- task_id: TASK-007
- statement: WHEN 编写 tests/integration 测试 THE SYSTEM SHALL 以 createServer 与 start({ port: 0 }) 启动真实服务并断言 GET / 页面容器结构、/api/levels 契约、level-select.js 对 /api/levels 的引用及注入无效 dataPath 夹具时的 500 错误体。
- requirement_refs: REQ-002, REQ-004, REQ-005, NFR-001
- done_when_refs: AC-009, AC-010, AC-011

### 编写策略
- exclusive_scopes: tests/integration/**
- forbidden_scopes: server/**, web/**, data/levels.json, tests/backend/**, tests/frontend/**

### 验收标准
- criterion_id: AC-009
- statement: WHEN 以 port 0 启动服务并同源请求 GET / THE SYSTEM SHALL 返回 200 HTML 且含 id=loading、id=level-list、id=error-message 容器与 level-select.js 引用。
- required_evidence: non_zero_test_execution
- criterion_id: AC-010
- statement: WHEN 同源请求 GET /api/levels 与 level-select.js 静态脚本 THE SYSTEM SHALL 分别返回恰好 5 项且字段满足 CT-001 约束的 JSON 数组与包含 /api/levels 相对路径引用的脚本内容。
- required_evidence: non_zero_test_execution
- criterion_id: AC-011
- statement: WHEN 以指向 tests/integration 夹具的 dataPath 启动服务并请求 GET /api/levels THE SYSTEM SHALL 返回 500 且 JSON 体 error.code 为 LEVEL_DATA_UNAVAILABLE 且 Content-Type 为 application/json; charset=utf-8。
- required_evidence: non_zero_test_execution

### 验证
- check_id: CHECK-003
- command: node --test tests/integration/
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true

### 交接模式 (Handoff Schema)
- required_fields: tests/integration 测试文件，integration dataPath fixtures，同源页面入口证据，/api/levels 契约证据，500 错误主体证据
- provided_contract_refs: []
- reviewer_check_refs: AC-009, AC-010, AC-011

### 阻塞项

### 可追溯性
- source_type: issue
- source_id: issue_workitem_0007#api
- requirement_id: REQ-002
- source_type: issue
- source_id: issue_workitem_0007#api
- requirement_id: REQ-004
- source_type: issue
- source_id: issue_workitem_0007#frontend
- requirement_id: REQ-005
- source_type: issue
- source_id: issue_workitem_0007#verify
- requirement_id: NFR-001