## 1. 后端图片生成链路与配置基础

- [ ] 1.1 引入 `reqwest` 依赖（json/multipart feature），建立图片客户端模块：按「有无参考图」选择 `/v1/images/generations` 或 `/v1/images/edits`；参考图随请求直接 multipart 传输不持久化；解析单张 `b64_json`；归一连接/超时/4xx/5xx/空 data/缺 b64 等错误；实施出站安全约束（HTTPS-only 或本地回环、跨域重定向不携带 Authorization、日志/诊断不明文记录 key）（映射 Requirement: 参考图改图自动选择端点 / 参考图输入约束 / 生成结果按媒体类型直接展示 / 图片生成失败与重试边界 / API Key 安全边界与出站目标约束）
- [ ] 1.2 实现网关配置持久化：扩展 `AriaStatePaths` 提供 image-create 配置路径；读写 `.aria/image_create.json`（base_url / api_key / 默认参数）；`GET` 返回脱敏 key、`PUT` 对 api_key 采用保留语义（空或占位则保留原值，清除需显式动作）（映射 Requirement: 网关配置录入与脱敏存储）
- [ ] 1.3 实现配置与参数校验：未录入有效 base_url 或 api_key 时拒绝生成；拒绝 size/quality/background/output_format/input_fidelity 越界值；文生图时忽略 input_fidelity；强制每次生成恰好一张（映射 Requirement: 图片生成由用户显式触发且参数可配置 / API Key 安全边界与出站目标约束）

## 2. 后端图片创作会话、prompt 迭代与并发

- [ ] 2.1 实现图片创作会话 CRUD 与独立 session 仓库/运行注册表：会话级对话历史、建议 prompt 与生成结果留存；扩展 `AriaStatePaths` 提供 scratch 路径（映射 Requirement: 独立顶层图片创作入口与会话 / 会话生命周期与资源清理）
- [ ] 2.2 实现会话生命周期与资源清理：删除会话先取消进行中操作、阻止新请求、删除 scratch 与持久化记录；删除失败上报；处理不存在会话请求（映射 Requirement: 会话生命周期与资源清理）
- [ ] 2.3 接入 `StreamingProvider` 承载 prompt 迭代：在会话 scratch 目录运行执行器，首轮建立、后续轮传 `resume_provider_session_id`；仅传不含 key 的 env_vars（映射 Requirement: 多轮 prompt 迭代由现有 CLI 执行器驱动 / API Key 安全边界与出站目标约束）
- [ ] 2.4 实现结构化建议 prompt：约定结构化产出含非空建议 prompt 字段；解析失败时保留上一轮可编辑 prompt 并提示；用 WS 流推前端（映射 Requirement: 结构化建议 prompt 的约定与降级）
- [ ] 2.5 实现执行器 session 续接失败降级：识别续接失败后以新 session 重新发起，首轮回灌上下文（模板引导词、历史输入、上一轮建议 prompt）；正常续接不重复回灌（映射 Requirement: 执行器 session 续接失败的降级）
- [ ] 2.6 实现单会话操作并发约束：每会话任一时刻最多一个进行中后端操作，忙碌时拒绝新请求并提示，不自动排队/取消（映射 Requirement: 单会话操作并发约束）
- [ ] 2.7 实现图片生成失败处理：不自动重试生图请求；失败归一为可读错误展示；失败记录一条事件（不含敏感信息）进会话历史，不写入成功结果（映射 Requirement: 图片生成失败与重试边界）
- [ ] 2.8 挂载 `/api/image-create/*` 路由：sessions（CRUD）、sessions/:id/chat（WS）、generate（POST，可选 multipart 参考图）、settings（GET/PUT）（映射 Impact 中 API 端点）

## 3. 后端模板

- [ ] 3.1 内置「PPT 商务配图」「业务流程图」两套模板引导词；支持创建会话时选择预置模板或填写一次性自定义引导词；引导词注入 prompt 迭代对话（不提供持久化 CRUD）（映射 Requirement: prompt 模板机制（一次性引导词））

## 4. 前端路由与页面骨架

- [ ] 4.1 新增顶层路由 `/image-create` 与导航入口，与 `/workbench` 平级，不依赖 Project/Issue（映射 Requirement: 独立顶层图片创作入口与会话）
- [ ] 4.2 实现会话列表（多会话创建/切换/删除）与独立 image-create chat entry 模型、API client 封装（sessions/chat/generate/settings）；复用 chat-workspace 消息流**展示组件**但使用图片域独立 entry 模型（映射 Requirement: 独立顶层图片创作入口与会话 / 会话生命周期与资源清理）

## 5. 前端对话与 prompt 区块

- [ ] 5.1 实现图片创作专用输入栏与模板选择（创建会话时选模板/一次性自定义引导词）（映射 Requirement: prompt 模板机制（一次性引导词））
- [ ] 5.2 实现「建议 prompt」可编辑区块；结构化解析失败时保留上一轮 prompt（映射 Requirement: 多轮 prompt 迭代由现有 CLI 执行器驱动 / 结构化建议 prompt 的约定与降级）
- [ ] 5.3 实现不满意循环：生成后继续在会话输入反馈回到 prompt 迭代（映射 Requirement: 不满意时可基于反馈继续迭代）
- [ ] 5.4 实现单会话忙碌态展示：进行中操作时禁止/提示新后端操作请求（映射 Requirement: 单会话操作并发约束）

## 6. 前端参数面板、参考图与结果展示

- [ ] 6.1 实现参数面板（size/quality/background/output_format/input_fidelity 下拉框，受默认参数预填），仅用户点生成按钮才提交；文生图时隐藏/忽略 input_fidelity（映射 Requirement: 图片生成由用户显式触发且参数可配置）
- [ ] 6.2 实现单张参考图上传/预览/移除与前端约束（MIME/大小），随生成请求 multipart 提交，由后端自动选端点（映射 Requirement: 参考图改图自动选择端点 / 参考图输入约束）
- [ ] 6.3 将返回 `b64_json` 按其 `output_format` 媒体类型构造 data URI 直接渲染；结果（含 prompt/参数）纳入会话历史供回看（映射 Requirement: 生成结果按媒体类型直接展示）

## 7. 前端设置界面

- [ ] 7.1 实现设置弹窗：录入 base_url（仅 HTTPS/本地回环） / api_key / 默认参数；展示脱敏 key；未重输 key 时保留（前端不缓存明文，提交保留语义）（映射 Requirement: 网关配置录入与脱敏存储 / API Key 安全边界与出站目标约束）

## 8. UI 视觉与打磨

- [ ] 8.1 基于 `[educational-platform]` 风格（Claymorphism + 块状 + 清晰层级 + 微交互）出视觉方案对比（融合 vs 照搬），与 Aria 现有调性协调后定稿（设计交付前置；详见 `cadence/prds/2026-08-03_概要需求_图片创作agent_v1.0.md` 第 9 节）

## 9. 验证

- [ ] 9.1 后端单测：端点选择（有无参考图）、出站安全约束（HTTPS、重定向不携带 Authorization、日志脱敏）、配置读写与保留语义、参数枚举校验、单会话并发互斥、会话删除取消操作并清理 scratch、生图错误不重试、用 FakeStreamingProvider 覆盖结构化解析失败与续接失败降级（对应 spec 各 Scenario）
- [ ] 9.2 前端单测：参数下拉绑定与 input_fidelity 显隐、参考图上传/移除/约束、prompt 区块编辑与解析失败保留、设置脱敏与保留语义、忙碌态展示
