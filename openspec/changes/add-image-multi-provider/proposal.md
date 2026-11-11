# Proposal: add-image-multi-provider

## Why

图片创作目前只能调用一个端点、一个写死的模型（`gpt-image-2`），且每次生成互相独立。用户需要：在多个模型间切换；连续修改时让新一轮基于上一张生成图；在 GPT 与 MiniMax 两家之间自动轮转出图以便对比。

官方文档调研得到三条直接决定方案的事实：

- 用户要求的 8 个 gpt 模型 id 中只有 `gpt-image-2.6-flare` 与 `gpt-image-2.6-sunburst` 是 OpenAI 官方 id，其余 6 个（裸 `gpt-image-2.6`、裸 `gpt-image-2.5`、4 个 `-2k`/`-4k`）是否可用取决于用户所配 endpoint，因此模型清单不能硬编码为枚举。
- MiniMax `MiniMax-H3-Image` 走与 OpenAI 相同的 `/v1/images/generations`、`/v1/images/edits` 路径与 Bearer 鉴权，但字段集不同（无 `quality`/`background`/`input_fidelity`，默认返回 URL，HTTP 200 时 `data[]` 可逐项带 `error`）。
- 现有客户端有三处与官方契约不符：模型 id 硬编码、对 2.x 模型仍发送 `input_fidelity`、响应解析不认识逐项 `error`。

## What Changes

- **模型清单可配置**：每个供应方（GPT / MiniMax）在设置中各有一份模型 id 清单与一个轮转模型；内置 8 个 gpt id 与 `MiniMax-H3-Image` 作为推荐默认值；生成时可显式选择清单中的模型，id 原样作为 `model` 发送，客户端不对 id 做任何解析或分辨率映射。
- **双供应方独立凭据**：`.aria/image_create.json` 由单组 `base_url`/`api_key` 演进为每家各一组；旧文件读入时原凭据归入 GPT，下次保存写为新形态。设置界面按家分区，API Key 的「保留 / 替换 / 清除」按家独立。**BREAKING**：设置更新接口的请求体与设置读取接口的响应体改为按家分组的形态；新形态配置文件不能被旧版本读取。
- **自动轮转**：生成请求新增 `model` 参数（`auto` 或模型 id，缺省 `auto`）。`auto` 时取会话上一条成功结果所属供应方的另一家，首次为 GPT；仅一家可用时恒用该家；运行期失败不换家、不重试。
- **携带上一张生成图**：生成请求新增 `use_previous` 参数（缺省 `false`，前端开关默认打开）。未上传参考图且该参数为真时，后端取会话最后一张生成图作为参考图走 `/v1/images/edits`；上一张图不可用时在出站前明确失败。
- **按官方契约构造请求与解析响应**：GPT 侧发送 `quality`/`background`、不发送 `response_format`；MiniMax 侧不发送 `quality`/`background`、显式发送 `response_format=b64_json`，`size` 为 `auto` 时改发 `area=1K`；响应逐项检查 `error`；不下载响应中的 URL。
- **移除 `input_fidelity`**：该参数自生成接口、参数面板与出站请求中移除。**BREAKING**：生成接口不再识别 `input_fidelity`（仍带该字段的请求被接受，字段被忽略）。
- **结果可观测**：每条生成结果记录并展示供应方、模型、选择方式（自动 / 显式）与参考图来源；旧记录无这些字段时照常展示。
- **默认模型变更**：未做任何配置的升级用户，实际使用的模型由 `gpt-image-2` 变为 `gpt-image-2.6-flare`。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `image-create-agent`：生成参数增加模型选择与携带上一张图、移除 `input_fidelity`；配置由单端点演进为双供应方并支持可配置模型清单；新增自动轮转、按供应方官方契约构造请求、逐项错误处理与结果的供应方 / 模型记录；「参考图不被持久化」的约束收窄为仅针对用户上传的参考图。delta 共 **7 条 MODIFIED + 8 条 ADDED，79 个 Scenario**。

## Impact

- 后端：`src/cross_cutting/image_client.rs`（请求构造、响应解析、trait 签名）、`src/product/image_create/models.rs`（设置与结果模型、错误变体、`SettingsStoreApi`）、`settings_store.rs`（按家合并、校验、脱敏）、`engine.rs`（目标解析、参考图来源、结果字段）、`image_files.rs`（结果图片读取）、`src/web/handlers/image_create.rs`（生成与设置接口）；`session_store.rs` 的测试与 `src/web/handlers/image_create_endpoint_tests.rs` 仅随结果结构新增字段调整测试夹具。
- 前端：`web/src/api/types/image-create.ts`、`web/src/api/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/state/image-create-entries.ts`、`web/src/components/image-create/{ParamsPanel,SettingsDialog,ChatPane,ReferenceImageUpload}.tsx` 及对应测试。
- 数据兼容：旧 `.aria/image_create.json` 必须可读且不丢凭据；旧会话记录（无供应方 / 模型字段、或仍内联 `b64`）必须可读、可参与轮转判定、可作为上一张图。
- 不改动：会话存储的非测试实现（`session_store.rs`）、参考图内容校验规则、图片文件路径规则、prompt 迭代链路、会话删除流程。
- 发布说明：`CHANGELOG.md` 记录默认模型变更与配置文件不可降级。
- 技术方案：`cadence/designs/2026-11-11_技术方案_图片生成多模型与双家轮转_v1.0.md`（类型定义、字段规则表、时序、逐文件改动、测试用例清单）。
- 待真实调用确认的假设：MiniMax 接受「不带 `size`、带 `area=1K`」的请求（技术方案 8.2 节 A1），在联调工作包中确认。**证据等级分侧**：`H3ImageGenerationReq` 侧已由官方字段表直接确认；`H3ImageEditReq` 侧官方未逐项列出可选字段集，改图端点接受 `area` 属推定，A1 的联调必须覆盖 generations 与 edits 两侧。
- 设计评审（2026-11-12）复核结论：三条需求与四条硬约束全部满足；`area=1K` 决策保留。评审发现的 3 条需修正（delta 漏改基线「生成结果按媒体类型直接展示」、前端 `saveSettings()` 前置 base_url 校验的编译级破坏点、D10 对 `area` 证据等级的越界）与 3 条建议优化已全部吸收，处置记录见技术方案 10.1 节。
- 归档时需手工同步主 spec 的 **Purpose 段**（delta 只作用于 Requirements），见 tasks 7.6。
