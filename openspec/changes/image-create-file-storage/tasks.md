## 1. 后端存储层：图片文件写入与引用模型

- [ ] 1.1 `AriaStatePaths` 新增 `image_create_images_dir()`（`.aria/image_create/images/`）与 `image_create_image_file(image_id, ext)` 辅助，含单元测试（路径推导、`.aria` 根复用）
- [ ] 1.2 `GenerationResult` 模型改引用形态：`b64: String` → `image_id: String`（保留 `media_type`、`prompt`、`params`、`ts`），serde 兼容旧字段（能反序列化含 `b64` 的旧 JSON），`cargo test --locked --lib image_create` 通过
- [ ] 1.3 实现图片文件原子写入辅助（base64 解码 → tmp + fsync + rename），含失败用例测试（目录不可写、非法 base64）

## 2. 生成链路落盘与失败边界

- [ ] 2.1 `ImageCreateEngine::finish_generation` 按 D3 顺序改造：先写图片文件、再更新会话引用；新增/调整单元测试断言写入顺序与"文件写失败不产生会话引用"
- [ ] 2.2 会话 JSON 更新失败路径复用 `pending_results`：重试只补引用不重写图片文件；测试覆盖 pending flush 后引用与文件一致
- [ ] 2.3 `generate` REST 响应改为返回 `image_id` + `media_type`（不再返回 `b64`），更新对应 handler 测试

## 3. 旧数据惰性迁移

- [ ] 3.1 会话读取路径检测内联 `b64` 记录并惰性迁移（逐张写文件 → 更新引用 → 原子重写），幂等（已有 image_id 跳过）；单元测试：旧 JSON 读取后返回引用形态且磁盘出现图片文件
- [ ] 3.2 迁移失败保留原数据：任一张写盘失败时该张保持 `b64` 原样、响应标记 legacy 占位、不丢数据、可重试；测试注入写盘失败断言行为
- [ ] 3.3 会话记录 API 序列化层强制剔除任何 `b64` 字段（含未迁移旧图），集成测试断言 `GET /api/image-create/sessions/{id}` 响应体不含 base64 长串

## 4. 图片读取端点

- [ ] 4.1 新增 `GET /api/image-create/sessions/{id}/images/{image_id}`：归属校验、image_id UUID/路径穿越校验、`Content-Type`、`ETag` + immutable 缓存头；路由注册与 handler 测试（正常/跨会话拒绝/非法 id 拒绝/文件缺失 404）
- [ ] 4.2 端点兼容服务未迁移旧图（从会话 JSON 的 b64 实时解码返回）；测试覆盖旧数据经端点可读

## 5. 会话删除清理

- [ ] 5.1 会话删除线性化流程并入该会话图片文件清理（引用列表 + 目录前缀兜底）；测试：删除后图片文件不残留、删除失败报告不静默

## 6. 前端改造

- [ ] 6.1 TS 类型与 API 层：`ImageGenerationResult` 去 `base64` 加 `image_id`，新增图片 URL 构造辅助；`pnpm tsc -b` 通过
- [ ] 6.2 `image-create-entries` 的 `generation_image` entry 改 `imageUrl`，`ChatPane` `<img>`/下载链接改 URL 渲染，`generate()` 用响应 `image_id` 即时构造 URL；组件/状态测试更新通过
- [ ] 6.3 移除 Zustand/entry 中所有 base64 持有路径；`cd web && pnpm test` 通过

## 7. 端到端验证

- [ ] 7.1 集成测试：旧版含 b64 会话 JSON → 新代码打开 → 响应无 base64、图片端点可取、磁盘文件生成；多图旧会话加载
- [ ] 7.2 手动验证：`pnpm dev` 前端连后端，生成新图、回看历史会话（含迁移后的旧会话）、删除会话后确认图片 404 与文件清理；`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`cd web && pnpm test` 全绿
