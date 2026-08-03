## 1. 后端图片生成链路与配置基础

- [ ] 1.1 引入 `reqwest` 依赖（json/multipart feature），建立图片客户端模块，封装按「有无参考图」选择 `/v1/images/generations` 或 `/v1/images/edits` 的调用与 `b64_json` 解析、错误归一（映射 Requirement: 参考图改图自动选择端点 / 生成结果以 base64 直接展示）
- [ ] 1.2 实现网关配置持久化：读写 `<workspace>/.aria/image_create.json`（base_url / api_key / 默认参数），`GET` 返回脱敏 key、`PUT` 写盘（映射 Requirement: 网关配置录入与脱敏存储）
- [ ] 1.3 实现配置缺失校验：未录入有效 base_url 或 api_key 时拒绝发起 image2 请求并返回明确错误（映射 Requirement: API Key 安全边界 / Scenario: 缺少有效配置时拒绝生成）

## 2. 后端图片创作会话与 prompt 迭代

- [ ] 2.1 实现图片创作会话 CRUD：独立会话存储、会话级对话历史与生成结果留存，会话删除时清理其 scratch 目录（映射 Requirement: 独立顶层图片创作入口与会话 / Scenario: 多会话相互独立）
- [ ] 2.2 接入 `StreamingProvider` 承载 prompt 迭代：在会话 scratch 目录运行执行器，首轮建立、后续轮通过 `resume_provider_session_id` 续接（映射 Requirement: 多轮 prompt 迭代由现有 CLI 执行器驱动 / Scenario: 多轮续接保留上下文）
- [ ] 2.3 用 `structured_output_contract` 让执行器产出「建议的最终 prompt」结构化结果，经 WS 流推前端（映射 Requirement: 多轮 prompt 迭代… / Scenario: 建议 prompt 可被用户编辑）
- [ ] 2.4 保证 prompt 迭代启动/续接的 CLI 子进程不接收 image2 api_key（映射 Requirement: API Key 安全边界 / Scenario: 执行器子进程不接触 key）
- [ ] 2.5 挂载 `/api/image-create/*` 路由：sessions CRUD、sessions/:id/chat（WS）、generate、settings（映射 Impact 中 API 端点）

## 3. 后端模板与参数校验

- [ ] 3.1 内置「PPT 商务配图」「业务流程图」两套模板引导词，支持自定义模板存入配置；所选模板引导词注入 prompt 迭代对话（映射 Requirement: prompt 模板机制）
- [ ] 3.2 实现图片生成参数枚举校验：拒绝 size/quality/background/output_format/n 越界值（映射 Requirement: 图片生成由用户显式触发且参数可配置 / Scenario: 拒绝越界参数值；`n` 限制 1–4）

## 4. 前端路由与页面骨架

- [ ] 4.1 新增顶层路由 `/image-create` 与导航入口，与 `/workbench` 平级，不依赖 Project/Issue（映射 Requirement: 独立顶层图片创作入口与会话 / Scenario: 用户无需项目即可开始创作）
- [ ] 4.2 实现会话列表（多会话创建/切换/删除）与 API client 封装（sessions/chat/generate/settings）

## 5. 前端对话与 prompt 区块

- [ ] 5.1 复用 chat-workspace 消息流组件呈现 prompt 迭代对话，新建图片创作专用输入栏与模板选择（创建会话时选模板/自定义）（映射 Requirement: prompt 模板机制）
- [ ] 5.2 实现「建议 prompt」可编辑区块，用户可在生成前修改（映射 Requirement: 多轮 prompt 迭代… / Scenario: 建议 prompt 可被用户编辑）
- [ ] 5.3 实现不满意循环：生成后继续在会话输入反馈回到 prompt 迭代（映射 Requirement: 不满意时可基于反馈继续迭代）

## 6. 前端参数面板、参考图与结果展示

- [ ] 6.1 实现参数面板（size/quality/background/output_format/n/input_fidelity 下拉框，受默认参数预填），仅用户点生成按钮才提交（映射 Requirement: 图片生成由用户显式触发且参数可配置 / Scenario: 用户点按钮触发生成）
- [ ] 6.2 实现单张参考图上传/预览/移除，随生成请求提交，由后端自动选端点（映射 Requirement: 参考图改图自动选择端点 / Scenario: 有参考图走改图 / 无参考图走文生图）
- [ ] 6.3 将返回 `b64_json` 以 base64 数据 URI 直接渲染展示，结果纳入会话历史供回看（映射 Requirement: 生成结果以 base64 直接展示 / Scenario: 直接展示生成图片 / 结果纳入会话历史）

## 7. 前端设置界面

- [ ] 7.1 实现设置弹窗：录入 base_url / api_key / 默认参数，展示脱敏 key，前端不缓存明文（映射 Requirement: 网关配置录入与脱敏存储 / Scenario: 录入并持久化配置 / 前端展示脱敏）

## 8. UI 视觉与打磨

- [ ] 8.1 基于 `[educational-platform]` 风格（Claymorphism + 块状 + 清晰层级 + 微交互）出视觉方案对比（融合 vs 照搬），与 Aria 现有调性协调后定稿（映射 PRD 第 9 节 UI 视觉方向；详见 `cadence/prds/2026-08-03_概要需求_图片创作agent_v1.0.md`）

## 9. 验证

- [ ] 9.1 后端单测：端点选择、配置读写脱敏、参数枚举校验、用 FakeStreamingProvider 测续接（对应 spec 各 Scenario）
- [ ] 9.2 前端单测：参数下拉绑定、参考图上传/移除、prompt 区块编辑提交、设置脱敏展示
