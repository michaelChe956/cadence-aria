# 关卡列表与关卡选择页需求（上游 Story Spec fixture）

source id: issue_workitem_0007#api
source id: issue_workitem_0007#frontend
source id: issue_workitem_0007#data
source id: issue_workitem_0007#verify

## 用户故事

- [REQ-001] 作为玩家，我希望打开游戏后看到关卡选择页，列出所有可选关卡及难度，以便选择要玩的关卡。
  - source id: issue_workitem_0007#frontend

## 功能需求

- [REQ-002] 系统 SHALL 提供 GET /api/levels 接口，返回关卡列表，每项含 id、name、difficulty、unlocked 字段。
  - source id: issue_workitem_0007#api
- [REQ-003] 关卡数据 SHALL 来自静态 JSON 数据文件，包含恰好 5 个关卡。
  - source id: issue_workitem_0007#data
- [REQ-004] 后端 SHALL 使用 Node 内置模块实现（node:http、node:fs），不引入任何 npm 依赖，并托管前端静态页面。
  - source id: issue_workitem_0007#api
- [REQ-005] 关卡选择页 SHALL 调用 GET /api/levels 渲染关卡列表，展示名称与难度。
  - source id: issue_workitem_0007#frontend
- [REQ-006] API 请求失败时，页面 SHALL 展示可读错误提示，不得白屏。
  - source id: issue_workitem_0007#frontend

## 非功能需求

- [NFR-001] 测试 SHALL 只使用 Node 内置测试运行器（node --test），不引入测试框架。
  - source id: issue_workitem_0007#verify

## 成功标准

- [AC-001] 启动服务后，GET /api/levels 返回 5 个关卡且字段完整。覆盖 REQ-002、REQ-003。
  - source id: issue_workitem_0007#api
- [AC-002] 打开页面可见 5 个关卡的名称与难度。覆盖 REQ-005。
  - source id: issue_workitem_0007#frontend
- [AC-003] 断开 API 后页面显示错误提示而非白屏。覆盖 REQ-006。
  - source id: issue_workitem_0007#frontend
- [AC-004] node --test 全部通过。覆盖 NFR-001。
  - source id: issue_workitem_0007#verify

## 范围

- 本 Story 只约定功能行为需求；API 契约、服务结构与验证方案的详细设计由下游 Design 产物（形态 07-fullstack-levels）完成。
- 不实现关卡实际玩法逻辑，选中关卡后的行为不在本期范围。
  - source id: issue_workitem_0007#frontend
