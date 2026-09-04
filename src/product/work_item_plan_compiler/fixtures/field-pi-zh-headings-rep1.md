# 工作项计划
## 工作项 WI-001: Hello API
### 身份 (Identity)
- schema_version: 1
- logical_work_item_id: WI-001
- title: Hello API
- kind: backend
### 目标 (Goal)
- summary: WHEN 联调冒烟需要后端问候接口 THE SYSTEM SHALL 提供仅依赖 Node 内置模块的 GET /api/hello 最小 HTTP 服务并返回 {"message":"hello"}。
### 非目标 (Non Goals)
- non_goals: 不引入任何第三方依赖，不新增数据库、认证、外部服务或前端页面
- non_goals: 不新增 test/hello.test.js 之外的其他测试，不使用非 Node 内置测试运行器
### 依赖 (Dependencies)
- depends_on: []
### 输入 (Inputs)
### 输出 (Outputs)
- contract_id: CT-001
- capabilities: node:http 服务监听 127.0.0.1:3000
- capabilities: GET /api/hello 返回 200 且响应体恰为 {"message":"hello"}
- capabilities: 内部错误时返回 500 且响应体为 {"error":"internal"}
### 任务 (Tasks)
- task_id: TASK-001
- statement: WHEN 运行 node --test THE SYSTEM SHALL 通过 test/hello.test.js 唯一测试启动 server.js 服务、请求 GET /api/hello 并断言状态码为 200 且响应体恰为 {"message":"hello"}。
- requirement_refs: design_requirement_placeholder
- done_when_refs: AC-001
- task_id: TASK-002
- statement: WHEN server.js 以 node:http 监听 127.0.0.1:3000 并收到 GET /api/hello 请求 THE SYSTEM SHALL 返回 200 状态码且响应体恰为 {"message":"hello"}。
- requirement_refs: design_requirement_placeholder
- done_when_refs: AC-001
### 写入策略 (Write Policy)
- exclusive_scopes: server.js
- exclusive_scopes: test/hello.test.js
- forbidden_scopes: 除 server.js 与 test/hello.test.js 外的仓库全部既有文件与目录
### 验收标准 (Acceptance Criteria)
- criterion_id: AC-001
- statement: WHEN 测试客户端向运行中的服务发起 GET /api/hello 请求 THE SYSTEM SHALL 收到 200 状态码且响应体恰为 {"message":"hello"}。
- required_evidence: non_zero_test_execution
### 验证 (Verification)
- check_id: CHECK-001
- command: node --test test/hello.test.js
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true
### 交接模式 (Handoff Schema)
- required_fields: []
- provided_contract_refs: []
- reviewer_check_refs: AC-001
### 阻塞项 (Blockers)
### 可追溯性 (Traceability)
- source_type: issue
- source_id: issue_workitem_0001#api
- requirement_id: design_requirement_placeholder