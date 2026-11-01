# Work Item Plan
## Work Item WI-001: 规则模块 gomoku-rules.js 与单元测试
### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: 规则模块 gomoku-rules.js 与单元测试
- kind: frontend
### Goal
- summary: WHEN 页面或测试需要判定 15×15 棋盘胜负 THE SYSTEM SHALL 经唯一 judge 函数返回 status、winner 与 line。
### Non Goals
- non_goals: 不实现页面渲染、样式与用户交互
- non_goals: 不提供 HTTP 路由或静态托管
- non_goals: 不新增任何运行时或开发依赖
- non_goals: 不校验维度非法或取值越界的入参
### Dependencies
- depends_on: []
### Inputs
### Outputs
- contract_id: CT-001
- capabilities: gomoku-rules.js 落盘于仓库根
- capabilities: judge 接受 board 并返回 status winner line 三字段
- capabilities: require 与浏览器全局取得同一 judge 函数
### Tasks
- task_id: TASK-001
- statement: WHEN 规则模块尚无实现 THE SYSTEM SHALL 先落盘失败的 test/gomoku-rules.test.js 用例覆盖四方向恰好五连、六连、被阻断四连、空盘与满盘平局。
- requirement_refs: REQ-013
- done_when_refs: AC-001
- done_when_refs: AC-002
- done_when_refs: AC-003
- done_when_refs: AC-004
- task_id: TASK-002
- statement: WHEN judge 收到 board THE SYSTEM SHALL 遍历全部已落子点位并沿横竖两条斜线计数，对同色连续 ≥5 的子串返回 win 与构成胜势的连续坐标。
- requirement_refs: REQ-004
- requirement_refs: REQ-005
- requirement_refs: REQ-007
- done_when_refs: AC-001
- done_when_refs: AC-002
- done_when_refs: AC-003
- done_when_refs: AC-004
- task_id: TASK-003
- statement: WHEN gomoku-rules.js 分别被 require 与 script src 加载 THE SYSTEM SHALL 使 module.exports.judge 与 window.GomokuRules.judge 指向同一函数且不修改入参 board。
- requirement_refs: REQ-007
- requirement_refs: REQ-018
- done_when_refs: AC-005
### Write Policy
- exclusive_scopes: gomoku-rules.js
- exclusive_scopes: test/gomoku-rules.test.js
- forbidden_scopes: server.js
- forbidden_scopes: gomoku.html
- forbidden_scopes: status.html
- forbidden_scopes: package.json
- forbidden_scopes: test/status-page.test.js
- forbidden_scopes: test/status-server.test.js
- forbidden_scopes: test/status-integration.test.js
### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN 横、竖、两条斜线任一方向恰好形成五连同色子 THE SYSTEM SHALL 返回 status 为 win 且 winner 为该色。
- required_evidence: non_zero_test_execution
- criterion_id: AC-002
- statement: WHEN 任一方向形成六连或更长同色连线 THE SYSTEM SHALL 按同一条件返回 status 为 win 且 winner 为该色。
- required_evidence: non_zero_test_execution
- criterion_id: AC-003
- statement: WHEN 四连同色子两端被对方棋子阻断且无法延伸为五连 THE SYSTEM SHALL 返回 status 为 playing。
- required_evidence: non_zero_test_execution
- criterion_id: AC-004
- statement: WHEN 空盘或 225 点落满且无人连成五子 THE SYSTEM SHALL 分别返回 status 为 playing 与 draw。
- required_evidence: non_zero_test_execution
- criterion_id: AC-005
- statement: WHEN 同一 gomoku-rules.js 分别经 require 与浏览器全局加载 THE SYSTEM SHALL 暴露同一 judge 函数引用且调用后不修改入参 board。
- required_evidence: non_zero_test_execution
### Verification
- check_id: CHECK-001
- command: node --test test/gomoku-rules.test.js
- required: true
- non_zero_test_execution_required: true
### Handoff Schema
- required_fields: gomoku_rules_module_path
- required_fields: judge_signature_and_result_shape
- required_fields: dual_environment_export
- provided_contract_refs: CT-001
- reviewer_check_refs: AC-001
- reviewer_check_refs: AC-002
- reviewer_check_refs: AC-003
- reviewer_check_refs: AC-004
- reviewer_check_refs: AC-005
### Blockers
### Traceability
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-004
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-005
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-007
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-013
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-018
## Work Item WI-002: 五子棋对战页 gomoku.html
### Identity
- schema_version: 1
- logical_work_item_id: WI-002
- title: 五子棋对战页 gomoku.html
- kind: frontend
### Goal
- summary: WHEN 同一浏览器内两名玩家轮流点击交叉点 THE SYSTEM SHALL 渲染 15×15 棋盘、经 CT-001 的 judge 判定并显示回合、胜负或平局。
### Non Goals
- non_goals: 不实现服务端路由与静态托管
- non_goals: 不实现 AI 对手、联机对战、悔棋或棋谱记录
- non_goals: 不在页面内落盘判定逻辑的第二份实现
- non_goals: 不引用外部 CDN、外链字体、外链图片或构建产物
### Dependencies
- depends_on: WI-001
### Inputs
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: gomoku-rules.js 落盘于仓库根
- required_capabilities: judge 接受 board 并返回 status winner line 三字段
- required_capabilities: require 与浏览器全局取得同一 judge 函数
- compatibility_policy: require_all
### Outputs
- contract_id: CT-002
- capabilities: gomoku.html 落盘于仓库根
- capabilities: gomoku.html 以 script src 引用本地 gomoku-rules.js
### Tasks
- task_id: TASK-004
- statement: WHEN 页面尚无实现 THE SYSTEM SHALL 先落盘失败的 test/gomoku-page.test.js 用例覆盖 225 个交叉点渲染、轮流落子、占位 no-op、胜负锁定、平局与重新开始。
- requirement_refs: REQ-001
- requirement_refs: REQ-002
- requirement_refs: REQ-003
- done_when_refs: AC-006
- done_when_refs: AC-007
- done_when_refs: AC-008
- task_id: TASK-005
- statement: WHEN 页面加载完成 THE SYSTEM SHALL 用 DOM 网格渲染 15×15 个可点击交叉点并把行棋方初始化为黑方。
- requirement_refs: REQ-001
- requirement_refs: REQ-016
- done_when_refs: AC-006
- task_id: TASK-006
- statement: WHEN 玩家触发某交叉点的点击 THE SYSTEM SHALL 仅在 phase 为 playing 且该点为空时落子并切换行棋方，否则保持棋盘与回合显示不变。
- requirement_refs: REQ-001
- requirement_refs: REQ-002
- requirement_refs: REQ-003
- done_when_refs: AC-007
- done_when_refs: AC-008
- task_id: TASK-007
- statement: WHEN 落子成功后以当前 board 调用 judge THE SYSTEM SHALL 依据返回的 status、winner 与 line 显示获胜方或平局并锁定棋盘。
- requirement_refs: REQ-004
- requirement_refs: REQ-005
- requirement_refs: REQ-007
- done_when_refs: AC-009
- done_when_refs: AC-010
- task_id: TASK-008
- statement: WHEN 玩家点击「重新开始」THE SYSTEM SHALL 清空全部棋子并恢复黑方先行与 playing 状态。
- requirement_refs: REQ-006
- done_when_refs: AC-011
- task_id: TASK-009
- statement: WHEN 页面在 375px 宽视口以触屏打开 THE SYSTEM SHALL 保证 15×15 棋盘完整可见且仅以 script src 引用本地 gomoku-rules.js。
- requirement_refs: REQ-016
- requirement_refs: REQ-017
- requirement_refs: REQ-020
- done_when_refs: AC-012
- done_when_refs: AC-013
### Write Policy
- exclusive_scopes: gomoku.html
- exclusive_scopes: test/gomoku-page.test.js
- forbidden_scopes: server.js
- forbidden_scopes: gomoku-rules.js
- forbidden_scopes: status.html
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-rules.test.js
### Acceptance Criteria
- criterion_id: AC-006
- statement: WHEN 页面加载完成 THE SYSTEM SHALL 呈现 225 个可点击交叉点且回合显示为轮到黑方。
- required_evidence: non_zero_test_execution
- criterion_id: AC-007
- statement: WHEN 黑方或白方点击空交叉点 THE SYSTEM SHALL 在该点落入本方棋子并把回合显示切换为另一方。
- required_evidence: non_zero_test_execution
- criterion_id: AC-008
- statement: WHEN 点击已落子的交叉点 THE SYSTEM SHALL 保持棋盘状态与回合显示均不变化。
- required_evidence: non_zero_test_execution
- criterion_id: AC-009
- statement: WHEN 某一方形成 ≥5 连 THE SYSTEM SHALL 显示获胜方并锁定棋盘使后续点击不再生效。
- required_evidence: non_zero_test_execution
- criterion_id: AC-010
- statement: WHEN 225 个交叉点全部落满且无人连成五子 THE SYSTEM SHALL 显示平局并锁定棋盘使后续点击不再生效。
- required_evidence: non_zero_test_execution
- criterion_id: AC-011
- statement: WHEN 点击「重新开始」THE SYSTEM SHALL 移除全部棋子并恢复黑方先行。
- required_evidence: non_zero_test_execution
- criterion_id: AC-012
- statement: WHEN 校验页面内联脚本 THE SYSTEM SHALL 只存在对 GomokuRules.judge 的调用而不存在第二处判定函数定义或独立连五扫描实现。
- required_evidence: non_zero_test_execution
- criterion_id: AC-013
- statement: WHEN 校验页面结构与样式 THE SYSTEM SHALL 使用视口自适应容器尺寸使 15×15 棋盘在窄视口完整可见，且触屏与鼠标点击共用同一命中路径。
- required_evidence: source_diff
### Verification
- check_id: CHECK-002
- command: node --test test/gomoku-page.test.js
- required: true
- non_zero_test_execution_required: true
### Handoff Schema
- required_fields: page_entry_path
- required_fields: inline_script_contract
- required_fields: gomoku_rules_script_src
- provided_contract_refs: CT-002
- reviewer_check_refs: AC-006
- reviewer_check_refs: AC-007
- reviewer_check_refs: AC-008
- reviewer_check_refs: AC-009
- reviewer_check_refs: AC-010
- reviewer_check_refs: AC-011
- reviewer_check_refs: AC-012
- reviewer_check_refs: AC-013
### Blockers
### Traceability
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-001
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-002
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-003
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-004
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-005
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-006
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-007
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-016
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-017
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-020
## Work Item WI-003: server.js 静态托管路由与服务测试
### Identity
- schema_version: 1
- logical_work_item_id: WI-003
- title: server.js 静态托管路由与服务测试
- kind: backend
### Goal
- summary: WHEN 请求 GET /gomoku.html 或 GET /gomoku-rules.js THE SYSTEM SHALL 在既有服务器中返回对应静态文件与正确 Content-Type 且不改动既有行为。
### Non Goals
- non_goals: 不修改 gomoku.html 与 gomoku-rules.js 的文件内容
- non_goals: 不提供落子、对局状态查询或胜负判定等游戏 API
- non_goals: 不引入目录级静态服务、不新增任何依赖
- non_goals: 不改动既有 /api/status、/status.html 与 404 兜底行为
### Dependencies
- depends_on: WI-001
- depends_on: WI-002
### Inputs
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: gomoku-rules.js 落盘于仓库根
- compatibility_policy: require_all
- contract_id: CT-002
- provider_logical_work_item_id: WI-002
- required_capabilities: gomoku.html 落盘于仓库根
- compatibility_policy: require_all
### Outputs
- contract_id: CT-003
- capabilities: GET /gomoku.html 返回 200 且 Content-Type 为 text/html
- capabilities: GET /gomoku-rules.js 返回 200 且 Content-Type 为 text/javascript
- capabilities: 既有 /api/status 与 /status.html 行为保持不变
### Tasks
- task_id: TASK-010
- statement: WHEN 服务尚无五子棋路由 THE SYSTEM SHALL 先落盘失败的 test/gomoku-server.test.js 用例断言两条新路由的状态码与 Content-Type。
- requirement_refs: REQ-008
- requirement_refs: REQ-009
- requirement_refs: REQ-014
- done_when_refs: AC-014
- done_when_refs: AC-015
- task_id: TASK-011
- statement: WHEN 请求命中 /gomoku.html 或 /gomoku-rules.js THE SYSTEM SHALL 以 __dirname 相对固定文件名读取并返回对应 Content-Type，读取失败时复用既有 404 兜底。
- requirement_refs: REQ-008
- requirement_refs: REQ-009
- requirement_refs: REQ-010
- requirement_refs: REQ-018
- done_when_refs: AC-014
- done_when_refs: AC-015
- done_when_refs: AC-017
- task_id: TASK-012
- statement: WHEN 请求既有 /api/status、/status.html、未知路径或非 GET 方法 THE SYSTEM SHALL 保持 200 JSON、200 逐字节一致 text/html 与 404 text/plain; charset=utf-8 的既有行为。
- requirement_refs: REQ-011
- requirement_refs: REQ-012
- done_when_refs: AC-016
### Write Policy
- exclusive_scopes: server.js
- exclusive_scopes: test/gomoku-server.test.js
- forbidden_scopes: gomoku.html
- forbidden_scopes: gomoku-rules.js
- forbidden_scopes: status.html
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-rules.test.js
- forbidden_scopes: test/gomoku-page.test.js
### Acceptance Criteria
- criterion_id: AC-014
- statement: WHEN 请求 GET /gomoku.html THE SYSTEM SHALL 返回 200 且 Content-Type 以 text/html 开头。
- required_evidence: non_zero_test_execution
- criterion_id: AC-015
- statement: WHEN 请求 GET /gomoku-rules.js THE SYSTEM SHALL 返回 200、Content-Type 以 text/javascript 开头且响应体与仓库根 gomoku-rules.js 逐字节一致。
- required_evidence: non_zero_test_execution
- criterion_id: AC-016
- statement: WHEN 请求既有 /api/status、/status.html、未知路径或非 GET 方法 THE SYSTEM SHALL 保持既有 200 与 404 契约不变且不暴露任何游戏 API。
- required_evidence: non_zero_test_execution
- criterion_id: AC-017
- statement: WHEN 校验 server.js 与 package.json THE SYSTEM SHALL 仅 require node:http、node:fs、node:path 且 dependencies 与 devDependencies 均为空。
- required_evidence: source_diff
### Verification
- check_id: CHECK-003
- command: node --test test/gomoku-server.test.js
- required: true
- non_zero_test_execution_required: true
### Handoff Schema
- required_fields: gomoku_route_paths
- required_fields: content_type_mapping
- required_fields: regression_baseline
- provided_contract_refs: CT-003
- reviewer_check_refs: AC-014
- reviewer_check_refs: AC-015
- reviewer_check_refs: AC-016
- reviewer_check_refs: AC-017
### Blockers
### Traceability
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-008
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-009
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-010
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-011
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-012
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-014
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-018
## Work Item WI-004: 跨模块集成验收
### Identity
- schema_version: 1
- logical_work_item_id: WI-004
- title: 跨模块集成验收
- kind: integration
### Goal
- summary: WHEN 规则模块、页面与服务均已交付 THE SYSTEM SHALL 以集成测试与浏览器手动验收证明单一判定实现、静态托管一致性与端到端可玩性。
### Non Goals
- non_goals: 不修改 server.js、gomoku.html、gomoku-rules.js 与 status.html
- non_goals: 不修改 test/gomoku-rules.test.js、test/gomoku-page.test.js 与 test/gomoku-server.test.js
- non_goals: 不引入测试框架依赖或端到端浏览器驱动依赖
### Dependencies
- depends_on: WI-001
- depends_on: WI-002
- depends_on: WI-003
### Inputs
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: require 与浏览器全局取得同一 judge 函数
- compatibility_policy: require_all
- contract_id: CT-002
- provider_logical_work_item_id: WI-002
- required_capabilities: gomoku.html 以 script src 引用本地 gomoku-rules.js
- compatibility_policy: require_all
- contract_id: CT-003
- provider_logical_work_item_id: WI-003
- required_capabilities: GET /gomoku.html 返回 200 且 Content-Type 为 text/html
- required_capabilities: GET /gomoku-rules.js 返回 200 且 Content-Type 为 text/javascript
- compatibility_policy: require_all
### Outputs
- contract_id: CT-004
- capabilities: 提供 node --test 全绿证据与浏览器手动验收记录
### Tasks
- task_id: TASK-013
- statement: WHEN 规则模块与页面均已交付 THE SYSTEM SHALL 落盘 test/gomoku-integration.test.js 断言双环境 judge 同引用、/gomoku-rules.js 响应体与仓库根文件逐字节一致、页面无外部资源与第二份判定实现。
- requirement_refs: REQ-007
- requirement_refs: REQ-009
- requirement_refs: REQ-017
- done_when_refs: AC-018
- done_when_refs: AC-019
- task_id: TASK-014
- statement: WHEN 集成测试就绪 THE SYSTEM SHALL 在仓库根执行 node --test 并记录全绿输出作为完成证据。
- requirement_refs: REQ-013
- requirement_refs: REQ-014
- requirement_refs: REQ-018
- done_when_refs: AC-020
- task_id: TASK-015
- statement: WHEN 服务在本地启动 THE SYSTEM SHALL 在 375px 宽触屏视口完成一次黑白对局并在停服或断网后继续验证落子、判胜与重新开始。
- requirement_refs: REQ-016
- requirement_refs: REQ-019
- requirement_refs: REQ-020
- done_when_refs: AC-021
### Write Policy
- exclusive_scopes: test/gomoku-integration.test.js
- forbidden_scopes: server.js
- forbidden_scopes: gomoku.html
- forbidden_scopes: gomoku-rules.js
- forbidden_scopes: status.html
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-rules.test.js
- forbidden_scopes: test/gomoku-page.test.js
- forbidden_scopes: test/gomoku-server.test.js
### Acceptance Criteria
- criterion_id: AC-018
- statement: WHEN 同一 gomoku-rules.js 经 require 与浏览器全局分别加载 THE SYSTEM SHALL 证明两个 judge 为同一函数引用。
- required_evidence: non_zero_test_execution
- criterion_id: AC-019
- statement: WHEN 校验 gomoku.html 与 GET /gomoku-rules.js 响应 THE SYSTEM SHALL 确认响应体与仓库根文件逐字节一致且页面只引用本地 gomoku-rules.js、无外部资源与第二份判定实现。
- required_evidence: non_zero_test_execution
- criterion_id: AC-020
- statement: WHEN 在仓库根执行 node --test THE SYSTEM SHALL 全部通过且未引入任何测试框架依赖。
- required_evidence: non_zero_test_execution
- criterion_id: AC-021
- statement: WHEN 在 375px 宽触屏视口对局并在停服或断网后继续操作 THE SYSTEM SHALL 保持棋盘完整可见可落子且判定与重新开始正常。
- required_evidence: manual_check
### Verification
- check_id: CHECK-004
- command: node --test
- required: true
- non_zero_test_execution_required: true
- check_id: CHECK-005
- command: null
- manual_instruction: 在本地启动服务后在 375px 宽触屏视口完成一次黑白对局，再停止服务或断网并继续落子、触发判胜与点击「重新开始」。
- required: true
- non_zero_test_execution_required: false
### Handoff Schema
- required_fields: integration_test_path
- required_fields: test_run_evidence
- required_fields: manual_browser_findings
- provided_contract_refs: []
- reviewer_check_refs: AC-018
- reviewer_check_refs: AC-019
- reviewer_check_refs: AC-020
- reviewer_check_refs: AC-021
### Blockers
### Traceability
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-007
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-009
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-013
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-014
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-016
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-017
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-018
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-019
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-020