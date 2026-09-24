# Work Item Plan
## Work Item WI-001: 五子棋规则模块 gomoku-rules.js 与单元测试
### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: 五子棋规则模块 gomoku-rules.js 与单元测试
- kind: backend
### Goal
- summary: WHEN 需要判定 15×15 棋盘上的五子连线结果 THE SYSTEM SHALL 由仓库根 gomoku-rules.js 导出唯一的具名判定函数 judge 供页面与测试共用。

### Non Goals
- non_goals: 不实现页面棋盘渲染、落子交互与「重新开始」控件
- non_goals: 不实现服务端静态托管路由
- non_goals: 不实现 AI 对手、联机对战、悔棋与棋谱记录
- non_goals: 不校验 judge 入参的维度与取值合法性

### Dependencies
- depends_on: []

### Inputs

### Outputs
- contract_id: CT-001
- capabilities: judge(board) 返回结构化判定结果对象
- capabilities: 四个方向连成五子及以上判定为获胜并给出获胜连线坐标
- capabilities: 满盘且无五连判定为平局
- capabilities: 被阻断的四连与不足五连判定为进行中
- capabilities: CommonJS 与浏览器全局暴露同一个 judge 函数
- capabilities: 仓库根存在可供静态托管的 gomoku-rules.js 文件

### Tasks
- task_id: TASK-001
- statement: WHEN 编写 gomoku-rules.js 的单元测试 THE SYSTEM SHALL 先经 judge 导出函数断言四方向恰好五连判胜与六连判胜与被阻断四连不判胜与空盘进行中与满盘平局并观察到测试失败。
- requirement_refs: REQ-013
- requirement_refs: REQ-007
- done_when_refs: AC-001
- done_when_refs: AC-002
- done_when_refs: AC-003
- done_when_refs: AC-004
- done_when_refs: AC-005
- task_id: TASK-002
- statement: WHEN 实现 gomoku-rules.js 的判定逻辑 THE SYSTEM SHALL 全盘扫描已落子点位并沿横竖两条斜线计数同色连子以产出 status 与 winner 与 line。
- requirement_refs: REQ-007
- done_when_refs: AC-001
- done_when_refs: AC-002
- done_when_refs: AC-003
- done_when_refs: AC-004
- done_when_refs: AC-005
- task_id: TASK-003
- statement: WHEN 导出判定实现 THE SYSTEM SHALL 以单份实现同时挂载 CommonJS 导出与浏览器全局 window.GomokuRules 且两处 judge 指向同一函数引用。
- requirement_refs: REQ-007
- requirement_refs: REQ-018
- done_when_refs: AC-006
- done_when_refs: AC-007

### Write Policy
- exclusive_scopes: gomoku-rules.js
- exclusive_scopes: test/gomoku-rules.test.js
- forbidden_scopes: gomoku.html
- forbidden_scopes: server.js
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-server.test.js

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN 经 judge 判定横向或纵向或两条斜线中任一方向恰好五连的棋盘 THE SYSTEM SHALL 返回 status 为 win 且 winner 为连子方且 line 长度不小于 5。
- required_evidence: non_zero_test_execution
- criterion_id: AC-002
- statement: WHEN 经 judge 判定任一方向六连的棋盘 THE SYSTEM SHALL 返回 status 为 win 且 winner 为连子方。
- required_evidence: non_zero_test_execution
- criterion_id: AC-003
- statement: WHEN 经 judge 判定四连两端被对方棋子阻断的棋盘 THE SYSTEM SHALL 返回 status 为 playing 且 winner 与 line 均为 null。
- required_evidence: non_zero_test_execution
- criterion_id: AC-004
- statement: WHEN 经 judge 判定全部为空的棋盘 THE SYSTEM SHALL 返回 status 为 playing 且 winner 与 line 均为 null。
- required_evidence: non_zero_test_execution
- criterion_id: AC-005
- statement: WHEN 经 judge 判定 225 个点位落满且无五连的棋盘 THE SYSTEM SHALL 返回 status 为 draw 且 winner 与 line 均为 null。
- required_evidence: non_zero_test_execution
- criterion_id: AC-006
- statement: WHEN 检查 gomoku-rules.js 的导出形态 THE SYSTEM SHALL 仅存在一份判定实现且 CommonJS 与浏览器全局暴露同一 judge 函数引用。
- required_evidence: source_diff
- criterion_id: AC-007
- statement: WHEN 在仓库根执行 node --test THE SYSTEM SHALL 规则单元测试全部通过且未引入任何测试框架依赖。
- required_evidence: non_zero_test_execution

### Verification
- check_id: CHECK-001
- command: node --test
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true
- check_id: CHECK-002
- command: null
- manual_instruction: 阅读 gomoku-rules.js，核对判定实现仅有一处且 CommonJS 导出与 window.GomokuRules 挂载的是同一 judge 函数引用。
- required: true
- non_zero_test_execution_required: false
- check_id: CHECK-003
- command: null
- manual_instruction: 核对 package.json 的 dependencies 与 devDependencies 均为空且规则模块未 require 任何模块。
- required: true
- non_zero_test_execution_required: false

### Handoff Schema
- required_fields: judge
- required_fields: JudgeResult.status
- required_fields: JudgeResult.winner
- required_fields: JudgeResult.line
- required_fields: window.GomokuRules.judge
- provided_contract_refs: CT-001
- reviewer_check_refs: AC-001
- reviewer_check_refs: AC-002
- reviewer_check_refs: AC-003
- reviewer_check_refs: AC-004
- reviewer_check_refs: AC-005
- reviewer_check_refs: AC-006
- reviewer_check_refs: AC-007

### Blockers
### Traceability
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-007
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-013
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-018

## Work Item WI-002: 五子棋双人对战页面 gomoku.html
### Identity
- schema_version: 1
- logical_work_item_id: WI-002
- title: 五子棋双人对战页面 gomoku.html
- kind: frontend
### Goal
- summary: WHEN 同一浏览器内的两名玩家打开 gomoku.html THE SYSTEM SHALL 在黑方先行的 15×15 棋盘上提供轮流落子与胜负平局显示与重新开始。

### Non Goals
- non_goals: 不实现 AI 对手与网络联机对战
- non_goals: 不实现悔棋与棋谱记录
- non_goals: 不实现服务端路由与第二份胜负判定实现
- non_goals: 不向服务端发起任何对战数据请求

### Dependencies
- depends_on: WI-001

### Inputs
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: judge(board) 返回结构化判定结果对象
- required_capabilities: 四个方向连成五子及以上判定为获胜并给出获胜连线坐标
- required_capabilities: 满盘且无五连判定为平局
- required_capabilities: 被阻断的四连与不足五连判定为进行中
- required_capabilities: CommonJS 与浏览器全局暴露同一个 judge 函数
- compatibility_policy: require_all

### Outputs
- contract_id: CT-002
- capabilities: gomoku.html 渲染 15×15 共 225 个可点击交叉点
- capabilities: 落子后按黑白轮换并显示当前行棋方
- capabilities: 已落子点位重复点击不改变棋盘与行棋方
- capabilities: 判定获胜时显示获胜方并锁定棋盘
- capabilities: 满盘无五连时显示平局并锁定棋盘
- capabilities: 「重新开始」按钮清空棋盘并恢复黑方先行
- capabilities: 页面经 script src 加载 gomoku-rules.js 并调用同一 judge 函数
- capabilities: 窄视口下棋盘完整可见且支持触屏点击落子
- capabilities: 仓库根存在可供静态托管的 gomoku.html 文件

### Tasks
- task_id: TASK-004
- statement: WHEN 构建 gomoku.html 的棋盘结构 THE SYSTEM SHALL 渲染 15×15 共 225 个可点击交叉点并保持黑方先行。
- requirement_refs: REQ-001
- done_when_refs: AC-008
- task_id: TASK-005
- statement: WHEN 玩家点击空交叉点 THE SYSTEM SHALL 落入当前行棋方棋子并切换行棋方且立即更新回合显示。
- requirement_refs: REQ-002
- requirement_refs: REQ-003
- done_when_refs: AC-009
- done_when_refs: AC-010
- task_id: TASK-006
- statement: WHEN 点击已有棋子的交叉点或处于锁定状态的棋盘 THE SYSTEM SHALL 不改变棋盘与行棋方与任何显示。
- requirement_refs: REQ-002
- done_when_refs: AC-009
- task_id: TASK-007
- statement: WHEN 每次成功落子后 THE SYSTEM SHALL 调用 gomoku-rules.js 的 judge 并依返回结果展示胜方或平局并锁定棋盘。
- requirement_refs: REQ-004
- requirement_refs: REQ-005
- requirement_refs: REQ-007
- requirement_refs: REQ-009
- done_when_refs: AC-011
- done_when_refs: AC-012
- done_when_refs: AC-014
- task_id: TASK-008
- statement: WHEN 玩家点击「重新开始」 THE SYSTEM SHALL 清空全部棋子并恢复黑方先行。
- requirement_refs: REQ-006
- done_when_refs: AC-013
- task_id: TASK-009
- statement: WHEN 页面在窄视口或触屏设备加载 THE SYSTEM SHALL 保证 15×15 棋盘完整可见且触屏点击可落子。
- requirement_refs: REQ-016
- done_when_refs: AC-015
- task_id: TASK-010
- statement: WHEN 页面加载完成后运行 THE SYSTEM SHALL 仅引用本地 gomoku-rules.js 且不发起任何对战网络请求。
- requirement_refs: REQ-017
- requirement_refs: REQ-019
- requirement_refs: REQ-011
- requirement_refs: REQ-020
- done_when_refs: AC-016
- done_when_refs: AC-017
- done_when_refs: AC-018

### Write Policy
- exclusive_scopes: gomoku.html
- forbidden_scopes: gomoku-rules.js
- forbidden_scopes: server.js
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-rules.test.js
- forbidden_scopes: test/gomoku-server.test.js

### Acceptance Criteria
- criterion_id: AC-008
- statement: WHEN 打开 gomoku.html 并点击空交叉点 THE SYSTEM SHALL 显示 225 个交叉点且黑方先行落子后轮到白方。
- required_evidence: manual_check
- criterion_id: AC-009
- statement: WHEN 点击已落子的交叉点 THE SYSTEM SHALL 保持棋盘与回合显示不变。
- required_evidence: manual_check
- criterion_id: AC-010
- statement: WHEN 任一方完成一次落子 THE SYSTEM SHALL 立即显示当前轮到的一方。
- required_evidence: manual_check
- criterion_id: AC-011
- statement: WHEN 任一方向连成五子及以上 THE SYSTEM SHALL 显示获胜方并锁定棋盘且后续点击不再生效。
- required_evidence: manual_check
- criterion_id: AC-012
- statement: WHEN 225 个交叉点落满且无人连成五子 THE SYSTEM SHALL 显示平局并锁定棋盘且后续点击不再生效。
- required_evidence: manual_check
- criterion_id: AC-013
- statement: WHEN 点击「重新开始」 THE SYSTEM SHALL 清空棋盘无残留棋子并恢复黑方先行。
- required_evidence: manual_check
- criterion_id: AC-014
- statement: WHEN 检查 gomoku.html 的内联脚本 THE SYSTEM SHALL 仅以调用方式使用 gomoku-rules.js 的 judge 且不存在第二份判定实现或独立连五扫描。
- required_evidence: source_diff
- criterion_id: AC-015
- statement: WHEN 在约 375px 宽视口以触屏操作 THE SYSTEM SHALL 棋盘完整可见不溢出且点击空交叉点可落子并轮转。
- required_evidence: manual_check
- criterion_id: AC-016
- statement: WHEN 检查 gomoku.html 引用的外部资源 THE SYSTEM SHALL 仅存在指向本地 gomoku-rules.js 的引用且无框架与构建产物。
- required_evidence: source_diff
- criterion_id: AC-017
- statement: WHEN 检查 gomoku.html 的文档头与文案 THE SYSTEM SHALL 声明 lang="zh-CN" 与 meta viewport 并使用中文界面文案。
- required_evidence: source_diff
- criterion_id: AC-018
- statement: WHEN 停止服务或断网后继续对局 THE SYSTEM SHALL 落子与判胜与「重新开始」仍正常工作且不产生对战数据请求。
- required_evidence: manual_check

### Verification
- check_id: CHECK-004
- command: null
- manual_instruction: 在浏览器打开 gomoku.html，逐项核对 AC-008 至 AC-013 的落子与回合与胜负平局与重开表现并记录结果。
- required: true
- non_zero_test_execution_required: false
- check_id: CHECK-005
- command: null
- manual_instruction: 在约 375px 宽视口与触屏设备下核对棋盘完整可见且触屏点击空交叉点可落子并正常轮转（AC-015）。
- required: true
- non_zero_test_execution_required: false
- check_id: CHECK-006
- command: null
- manual_instruction: 检索 gomoku.html 核对无第二份判定实现、无外部资源引用、lang 与 viewport 声明符合既有页面约定（AC-014、AC-016、AC-017）。
- required: true
- non_zero_test_execution_required: false
- check_id: CHECK-007
- command: null
- manual_instruction: 停止服务后继续落子与判胜与重开，核对功能不依赖网络且页面无对战数据请求（AC-018）。
- required: true
- non_zero_test_execution_required: false

### Handoff Schema
- required_fields: gomoku.html
- required_fields: 棋盘容器元素标识
- required_fields: window.GomokuRules.judge 调用点
- provided_contract_refs: CT-002
- reviewer_check_refs: AC-008
- reviewer_check_refs: AC-009
- reviewer_check_refs: AC-010
- reviewer_check_refs: AC-011
- reviewer_check_refs: AC-012
- reviewer_check_refs: AC-013
- reviewer_check_refs: AC-014
- reviewer_check_refs: AC-015
- reviewer_check_refs: AC-016
- reviewer_check_refs: AC-017
- reviewer_check_refs: AC-018

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
- requirement_id: REQ-009
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-011
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-016
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-017
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-019
- source_type: design_spec
- source_id: design_spec_0001
- requirement_id: REQ-020

## Work Item WI-003: 服务端静态托管路由与服务测试
### Identity
- schema_version: 1
- logical_work_item_id: WI-003
- title: 服务端静态托管路由与服务测试
- kind: backend
### Goal
- summary: WHEN 浏览器请求 GET /gomoku.html 或 GET /gomoku-rules.js THE SYSTEM SHALL 由既有 server.js 返回对应文件内容且既有路由行为保持不变。

### Non Goals
- non_goals: 不为五子棋提供落子或对局状态或胜负判定接口
- non_goals: 不实现目录级静态服务与路径参数化托管
- non_goals: 不修改规则模块判定实现与页面交互实现
- non_goals: 不新增任何运行时或开发依赖

### Dependencies
- depends_on: WI-001
- depends_on: WI-002

### Inputs
- contract_id: CT-001
- provider_logical_work_item_id: WI-001
- required_capabilities: 仓库根存在可供静态托管的 gomoku-rules.js 文件
- compatibility_policy: require_all
- contract_id: CT-002
- provider_logical_work_item_id: WI-002
- required_capabilities: 仓库根存在可供静态托管的 gomoku.html 文件
- compatibility_policy: require_all

### Outputs
- contract_id: CT-003
- capabilities: GET /gomoku.html 返回 200 且 Content-Type 以 text/html 开头
- capabilities: GET /gomoku-rules.js 返回 200 且 Content-Type 以 text/javascript 开头
- capabilities: 既有 GET /api/status 与 GET /status.html 与未知路径及非 GET 请求的响应保持不变
- capabilities: 服务与规则模块仅使用 node:http 与 node:fs 与 node:path 与 process

### Tasks
- task_id: TASK-011
- statement: WHEN 编写服务测试 THE SYSTEM SHALL 先断言 GET /gomoku.html 返回 200 且 Content-Type 以 text/html 开头并观察到测试失败。
- requirement_refs: REQ-008
- requirement_refs: REQ-014
- done_when_refs: AC-019
- task_id: TASK-012
- statement: WHEN 编写 gomoku-rules.js 的托管测试 THE SYSTEM SHALL 断言 GET /gomoku-rules.js 返回 200 且响应体与仓库根文件逐字节一致。
- requirement_refs: REQ-009
- done_when_refs: AC-020
- task_id: TASK-013
- statement: WHEN 在既有 server.js 请求分派中新增静态路由 THE SYSTEM SHALL 以固定白名单与 __dirname 相对解析返回页面与规则脚本。
- requirement_refs: REQ-008
- requirement_refs: REQ-009
- requirement_refs: REQ-010
- requirement_refs: REQ-018
- done_when_refs: AC-019
- done_when_refs: AC-021
- task_id: TASK-014
- statement: WHEN 服务在新增路由后运行 THE SYSTEM SHALL 保持既有状态接口与既有页面与非 GET 请求的 404 文本响应不变。
- requirement_refs: REQ-012
- done_when_refs: AC-023
- task_id: TASK-015
- statement: WHEN 枚举服务暴露的路由 THE SYSTEM SHALL 确认不存在任何落子或对局状态或胜负判定接口。
- requirement_refs: REQ-011
- done_when_refs: AC-022

### Write Policy
- exclusive_scopes: server.js
- exclusive_scopes: test/gomoku-server.test.js
- forbidden_scopes: gomoku.html
- forbidden_scopes: gomoku-rules.js
- forbidden_scopes: package.json
- forbidden_scopes: test/gomoku-rules.test.js

### Acceptance Criteria
- criterion_id: AC-019
- statement: WHEN 请求 GET /gomoku.html THE SYSTEM SHALL 返回 200 且 Content-Type 以 text/html 开头且响应体与仓库根 gomoku.html 内容一致。
- required_evidence: non_zero_test_execution
- criterion_id: AC-020
- statement: WHEN 请求 GET /gomoku-rules.js THE SYSTEM SHALL 返回 200 且 Content-Type 以 text/javascript 开头且响应体与仓库根 gomoku-rules.js 逐字节一致。
- required_evidence: non_zero_test_execution
- criterion_id: AC-021
- statement: WHEN 检查服务与规则模块源码的 require 调用 THE SYSTEM SHALL 目标集合恰为 node:http 与 node:fs 与 node:path 且 package.json 的 dependencies 与 devDependencies 为空。
- required_evidence: source_diff
- criterion_id: AC-022
- statement: WHEN 枚举服务暴露的路由 THE SYSTEM SHALL 不含任何落子或对局状态查询或服务端胜负判定接口。
- required_evidence: manual_check
- criterion_id: AC-023
- statement: WHEN 请求 GET /api/status 与 GET /status.html 与未知路径及非 GET 请求 THE SYSTEM SHALL 分别返回既有 200 JSON 与既有 200 text/html 与 404 text/plain; charset=utf-8。
- required_evidence: non_zero_test_execution
- criterion_id: AC-024
- statement: WHEN 在仓库根执行 node --test THE SYSTEM SHALL 服务测试全部通过且未引入任何测试框架依赖。
- required_evidence: non_zero_test_execution

### Verification
- check_id: CHECK-008
- command: node --test
- manual_instruction: null
- required: true
- non_zero_test_execution_required: true
- check_id: CHECK-009
- command: null
- manual_instruction: 检索 server.js 与 gomoku-rules.js 的 require 调用并核对 package.json 依赖为空（AC-021）。
- required: true
- non_zero_test_execution_required: false
- check_id: CHECK-010
- command: null
- manual_instruction: 枚举服务路由清单并核对不存在落子与对局状态与胜负判定接口，静态资源路由不计入游戏 API（AC-022）。
- required: true
- non_zero_test_execution_required: false

### Handoff Schema
- required_fields: GET /gomoku.html
- required_fields: GET /gomoku-rules.js
- required_fields: createServer
- provided_contract_refs: []
- reviewer_check_refs: AC-019
- reviewer_check_refs: AC-020
- reviewer_check_refs: AC-021
- reviewer_check_refs: AC-022
- reviewer_check_refs: AC-023
- reviewer_check_refs: AC-024

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