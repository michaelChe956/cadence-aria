# Design: image-create-file-storage

## Context

现状链路（见 proposal）：`ImageCreateEngine::finish_generation` 把 `outcome.b64` 放进 `GenerationResult.b64`，`SessionStore::append_generation_result` 原子重写整个 `.aria/image_create/sessions/<id>.json`；`GET /api/image-create/sessions/{id}` 全量返回含所有图片 base64 的 `SessionRecord`，前端 `buildEntries` 原样复制 base64 并构造 data URI。会话存储为单 JSON 文件（非 SQLite），后端为 Rust axum + RustEmbed，前端 Zustand。仓库无既有附件/图片文件服务设施；静态文件服务仅服务 `web/dist`。

## Goals / Non-Goals

**Goals:**
- 会话记录 API 响应不再包含图片字节；打开会话的响应体积与会话文本成正比，与图片数量无关。
- 图片字节恰好持久化一份（文件），崩溃一致性有明确顺序约束。
- 旧内联 `b64` 数据无损迁移，迁移可失败重试。
- 图片端点具备归属校验与路径安全。

**Non-Goals:**
- 缩略图/WebP 二级资产（方案 B，本期不做）。
- 生图网关协议、prompt 迭代链路、参考图上传行为的任何变化。
- 跨会话图片库/复用、对象存储、多实例共享卷。
- 全量一次性批量迁移命令（只做首次访问的惰性迁移）。

## Decisions

### D1: 图片落盘位置与命名
图片写入 `AriaStatePaths` 新增的 `image_create_images_dir()` = `.aria/image_create/images/`，文件名为 `<image_id>.<扩展名>`；`image_id` 为 UUIDv4（与仓库其他 id 生成方式一致），扩展名由 `media_type` 映射（png/jpeg/webp）。
- 备选：按会话分子目录 `images/<session_id>/`。否决：删除会话时递归清理更简单，但孤儿清理与迁移要去重跨会话引用时更复杂；本设计图片与会话一一归属，Flat 目录 + 记录归属即可，删除按会话过滤前缀清理。
- 备选：内容哈希命名以去重。否决：同一图重复生成概率低，去重引入引用计数复杂度，YAGNI。

### D2: 会话记录中的引用形态
`GenerationResult` 的 `b64: String` 字段替换为 `image_id: String`（`media_type` 保留）。serde 用 `#[serde(default)]` + 新增旧字段兼容：反序列化时若存在 `b64` 且无 `image_id`，视为待迁移旧记录（读取路径处理，见 D4）。
- 备选：保留 `Option<String> b64` 字段长期共存。否决：双字段长期并存会让"响应不含 base64"的目标失守，序列化时必须强制剔除旧字段。

### D3: 写入顺序与失败边界
`finish_generation` 顺序：(1) base64 解码 → (2) 图片文件 `write_json_atomic` 同款 tmp+fsync+rename 原子写入 → (3) `append_generation_result` 更新会话 JSON。(2) 失败按既有 `ImageClient/Store` 错误边界暴露（生成失败事件），不进入会话历史引用；(3) 失败走既有 `pending_results` 进程内保留重试路径（图片文件已写好，重试只补会话 JSON；由此产生的"有文件无引用"孤儿由 D6 清理）。删除会话的图片清理并入既有 tombstone 线性化删除流程，在删 scratch 目录同一步骤删除该会话图片文件。

### D4: 旧数据惰性迁移
`SessionStore::get`（或 engine 读取入口）检测到含内联 `b64` 的记录时：逐张解码写文件 → 更新该记录为引用形态 → 原子重写会话 JSON → 返回引用形态。任一张迁移失败：该张及之后的保留 `b64` 原样返回（前端兼容旧 data URI 展示路径），已成功的张目不回滚（幂等：已有 `image_id` 的跳过）。响应序列化层保证：无论迁移成功与否，会话记录 API 的 JSON 输出永不包含 `b64` 字段——未迁移成功的旧图在响应中标记为 `legacy_pending: true` 之类的占位字段，由图片端点兼容读取（见 D5）。**取舍**：未迁移旧图通过端点实时解码 base64 提供，避免"迁移失败就会话打不开"。
- 备选：启动时全量扫描迁移。否决：启动延迟与不可控失败面；惰性迁移把代价摊到首次访问。

### D5: 图片读取端点
`GET /api/image-create/sessions/{session_id}/images/{image_id}`，axum `Path` 提取后先 `validate_session_id`/UUID 校验 `image_id`（拒绝路径穿越），再在会话记录中确认归属；命中文件则以 `mime_guess`/media_type 映射返回字节 + `ETag`（image_id 内容寻址不可变，可 `Cache-Control: private, max-age=31536000, immutable`）。旧图未迁移时端点从会话 JSON 中的 `b64` 解码返回（同 ETag 策略，迁移完成后自然切换到文件）。
- 备选：全局图片端点 `/api/image-create/images/{id}`（无会话归属）。否决：spec 要求会话隔离校验，且删除会话后图片必须不可达。

### D6: 孤儿清理
两处来源：(a) D3 的"文件已写、会话 JSON 更新失败"；(b) 历史遗留文件。清理策略：会话删除时删除其全部图片（按会话记录引用 + 会话前缀兜底）；不做全局 GC 任务（Non-Goal，文件系统孤儿不阻塞正确性，仅占磁盘）。

### D7: 前端改造
`GenerationResult` TS 类型去掉 `base64`、加 `image_id`；`image-create-entries` 的 `generation_image` entry 由 `base64` 改为 `imageUrl`（由会话 id + image_id 构造）；`ChatPane` `<img src>` 与下载链接直接用该 URL（下载 `download` 属性带扩展名文件名）。`generate()` 响应仍即时返回 `media_type` + 图片字节信息（新增返回 `image_id`，前端即时用端点 URL 展示，无需等待重新拉会话）。Zustand 不再持有任何 base64 字符串。

## Risks / Trade-offs

- [崩溃窗口：图片文件写好但会话 JSON 更新失败/进程崩溃] → 孤儿文件只占磁盘不影响正确性；`pending_results` 重试只补引用；会话删除兜底清理。
- [旧会话迁移失败导致响应仍需兼容旧图] → 响应永不内联 base64（D4 强制），未迁移图经端点实时解码服务，迁移可重试，原数据不删。
- [多进程/并发访问同一会话（单后端假设）] → 沿用现有单写者约定与 `generation` 乐观并发；迁移写入与既有 `append_*` 同一互斥路径。
- [`.aria` 目录被用户同步/备份工具跳过] → 图片与会话 JSON 同在 `.aria/image_create/` 下，风险与现状一致，无新增差异。
- [浏览器缓存与不可变性] → image_id 为一次性 UUID，内容不可变，`immutable` 缓存安全；删除会话后缓存失效由同源 404 体现。

## Migration Plan

1. 合入后旧版本数据无需预迁移：新代码读旧 JSON 兼容，首次访问惰性迁移。
2. 回滚：新代码写出的会话 JSON（引用形态）旧代码无法展示图片（旧代码读不到 `b64`）——回滚窗口内已迁移会话的图片在旧版本下显示缺失，但数据无损（文件在磁盘），再次升级即恢复。接受该单向兼容取舍，不做双写。

## Open Questions

- 无（缩略图方案 B 留待未来 change）。
