# Proposal: image-create-file-storage

## Why

图片创作会话把生成图片以 base64 持久化在 `.aria/image_create/sessions/<id>.json` 的 `generation_results[].b64` 中，打开会话时 `GET /api/image-create/sessions/{id}` 全量返回所有历史图片 base64（无分页、无懒加载），会话越大加载越慢；且每次生成都会原子重写整个含图片的会话 JSON，写放大严重。业界本地优先产品（Open WebUI、LibreChat、Cherry Studio、AnythingLLM、Jan）共识为图片字节落本地文件、会话数据只存引用、按 URL 懒加载。

## What Changes

- 生成图片以二进制文件写入 `.aria/image_create/images/`（UUID 命名，保留原始媒体格式）。
- 会话 JSON 的 `generation_results[]` 不再存 `b64`，改存图片引用（`image_id` + `media_type`）；旧会话中的 `b64` 保持兼容读取，首次访问时迁移落盘。
- 新增图片读取端点（如 `GET /api/image-create/sessions/{id}/images/{image_id}`），带强缓存头；前端 `<img>` 按 URL 懒加载，打开会话不再传输图片字节。
- 图片文件写入与会话 JSON 更新保持崩溃一致（先写图片文件，再更新引用）；删除会话时清理其图片文件。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `image-create-agent`: 生成结果的持久化形态从"会话内 base64"改为"文件存储 + 引用"，会话读取 API 不再内联图片字节，新增图片二进制读取端点；旧数据兼容迁移与删除清理属该能力的验收范围。

## Impact

- 后端：`src/product/image_create/models.rs`（GenerationResult 结构）、`session_store.rs`（读写/迁移/删除清理）、`engine.rs`（生成落盘路径）、`src/cross_cutting/aria_state_paths.rs`（images 目录）、`src/web/handlers/image_create.rs` + `app.rs`（新端点）。
- 前端：`web/src/api/types/image-create.ts`、`web/src/api/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/state/image-create-entries.ts`、`web/src/components/image-create/ChatPane.tsx`（data URI → URL）。
- 数据兼容：旧会话 JSON 含 `b64` 的记录必须可读且可增量迁移；迁移失败不丢数据。
- 不改生图网关协议、prompt 迭代链路与参考图上传行为。
