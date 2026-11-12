# Proposal: add-image-multi-provider

## Why

图片创作目前只能调用一个端点、一个写死的模型（`gpt-image-2`），且每次生成互相独立。用户需要：在 GPT 与 MiniMax 两家的多个模型间手动切换；连续修改时让新一轮基于上一张生成图。

官方文档调研得到三条直接决定方案的事实：

- 用户要求的 8 个 gpt 模型 id 中只有 `gpt-image-2.6-flare` 与 `gpt-image-2.6-sunburst` 是 OpenAI 官方 id，其余 6 个（裸 `gpt-image-2.6`、裸 `gpt-image-2.5`、4 个 `-2k`/`-4k`）是否可用取决于用户所配 endpoint，因此模型清单不能硬编码为枚举。
- MiniMax `MiniMax-H3-Image` 走与 OpenAI 相同的 `/v1/images/generations`、`/v1/images/edits` 路径与 Bearer 鉴权，但字段集不同（无 `quality`/`background`/`input_fidelity`，默认返回 URL，HTTP 200 时 `data[]` 可逐项带 `error`）。
- 现有客户端有三处与官方契约不符：模型 id 硬编码、对 2.x 模型仍发送 `input_fidelity`、响应解析不认识逐项 `error`。

## What Changes

- **模型清单可配置**：每个供应方（GPT / MiniMax）在设置中各有一份模型 id 清单（每家至多 256 个）；内置 8 个 gpt id（`gpt-image-2.6-flare` 打头）与 `MiniMax-H3-Image` 作为推荐默认值；生成时由用户在已配置且可用的供应方清单中手动选择模型，id 原样作为 `model` 发送，客户端不对 id 做任何解析或分辨率映射。
- **双供应方独立凭据**：`.aria/image_create.json` 由单组 `base_url`/`api_key` 演进为每家各一组；旧文件读入时原凭据归入 GPT，下次保存写为新形态。设置界面按家分区，API Key 的「保留 / 替换 / 清除」按家独立。**BREAKING**：设置更新接口的请求体与设置读取接口的响应体改为按家分组的形态；新形态配置文件不能被旧版本读取。
- **模型必填且手动选择**：生成请求新增**必填** `model` 参数（已配置且可用供应方的 model id），缺省或为空视为参数错误。所选模型不在任何供应方清单中、或其所属供应方未配置凭据时，在出站前明确失败且不换家；运行期失败不换家、不重试。参数面板的初始选中项取自第一个可用供应方的清单：优先内置推荐清单的首个 id，其次第一个 id 含 `image` 的，最后清单首项；无可用供应方时生成按钮禁用并提示先完成配置。
- **携带上一张生成图**：生成请求新增 `use_previous` 参数（缺省 `false`，前端开关默认打开）。未上传参考图且该参数为真时，后端取会话最后一张生成图作为参考图走 `/v1/images/edits`；上一张图不可用时在出站前明确失败。
- **按官方契约构造请求与解析响应**：GPT 侧发送 `quality`/`background`、不发送 `response_format`；MiniMax 侧不发送 `quality`/`background`、显式发送 `response_format=b64_json`，`size` 为 `auto` 时改发 `area=1K`；响应逐项检查 `error`；不下载响应中的 URL。
- **移除 `input_fidelity`**：该参数自生成接口、参数面板与出站请求中移除。**BREAKING**：生成接口不再识别 `input_fidelity`（仍带该字段的请求被接受，字段被忽略）。
- **结果可观测**：每条生成结果记录并展示供应方、模型与参考图来源；旧记录无这些字段时照常展示。
- **默认模型变更**：未做任何配置的升级用户，实际使用的模型由 `gpt-image-2` 变为 `gpt-image-2.6-flare`（内置 GPT 清单的首项，即参数面板的初始选中项）。
- **供应方连通性探测与 Base URL 填写说明**：设置界面每个供应方新增「测试连通」，用当前填写的值（未重新输入 key 时由后端改用已保存的 key，且仅当所填 Base URL 与已保存的一致；不一致时拒绝并提示先填写 API Key，不出站）依次请求 `GET /v1/models` 与一个故意不含 `prompt` 的 `POST /v1/images/generations`，给出「地址通不通、key 是否有效、清单中哪些模型未被网关列出」；探测不生成图片、不计费、不落盘，`base_url` 走与生成出站相同的校验。新增接口 `POST /api/image-create/settings/connectivity`。Base URL 输入框给出示例，并说明只填到主机根路径（系统自动拼 `/v1/images/{generations|edits}`）。
- **模型清单以连通性探测结果为来源，设置里改为复选下拉框**：探测响应新增 `available_models`（网关 `/v1/models` 返回的全部 model id，去重并保持网关返回顺序；该步未成功时为空），其余字段不变。设置界面的模型清单由多行文本框改为复选下拉框，勾选的即要保存的清单。探测取得非空的网关模型列表时，清单**更新为网关返回的模型**（至多 256 个；已被另一供应方勾选的跳过并提示），更新前清单里有而网关未返回的项移出清单、留在候选项中并标注「网关未列出」；入口旁说明「成功后会用网关返回的模型列表更新模型清单」。用户可在此基础上取消勾选或添加网关未列出的自定义 id。单家清单上限由 32 调整为 256。参数面板的初始选中项改为三级优先规则，避免清单首项是文本模型时默认选中非图片模型。探测第 2 步的模型 id 改为优先取清单中第一个 id 含 `image` 的项。不新增接口、不新增出站请求。
- **会话历史按时间顺序交织呈现**（缺陷修复）：重新打开会话时，对话消息、建议 prompt、生成结果、事件此前按类别分四段首尾相接，图与它的 prompt 被拆到两头、报错掉到最后。根因是会话记录里只有建议 prompt 块没有时间戳。建议 prompt 块新增可选时间戳 `ts`（写入时记录，会话记录读取接口随之返回）；前端把四类条目合并后按时间戳升序排序，时间戳相同的依次为对话消息、建议 prompt、生成结果、事件；没有时间戳的旧建议 prompt 以同序号用户消息的时间定位，再退回会话创建时间。不新增接口、不新增出站请求；旧会话记录照常可读。
- **模型未产出与结构化输出失败分开处理**（体验修复）：prompt 迭代时模型正常跑完、但拒绝给出图片 prompt 且不按结构化约定输出，此前前端只显示一大段可读文本，「本轮未产出新的建议 prompt，已保留上一版」排在长文本之后被淹没，用户以为执行器断了；首轮未产出时甚至没有任何提示。本次调整四处：① **结构化约定改为双出口**——不给 prompt 时仍输出约定的 JSON 块、把 `suggested_prompt` 置为空字符串；原首句里的「不要输出其他内容」改为「除上面的 JSON 块外不要输出其他内容」，并写明可在 JSON 块之外用一段文字说明原因。② **两种降级可判定**——迭代事件新增 `prompt_declined`（结构化输出解析成功但建议 prompt 为空或缺失时为真），据此区分「模型未产出」与「结构化输出失败」。③ **提示文案条件化**——只要本轮未产出建议 prompt 就给出恰好一条提示（不再要求存在上一版），排在模型说明之前；主句按是否明确未产出取「本轮模型未产出可用 prompt」或「本轮未产出新的建议 prompt」，明确未产出且模型给了说明时接「（下方是它给出的说明）」，存在上一版时末尾接「，已保留上一版」。④ **模型说明折叠**——仅模型明确未产出时把它的说明收进「查看模型说明」（默认收起）；结构化输出失败时说明照常直接显示。不新增接口、不新增出站请求、不改会话记录形态；旧前端忽略新字段，新前端在字段缺失时按结构化输出失败处理。
- **合入前审查修复**（安全与缺陷修复，2026-11-12）：① 连通性探测未提供 key 时，已保存的 key 只在所填 Base URL 与已保存的一致时使用（两侧去首尾空白与末尾 `/` 后逐字符比较），否则拒绝并提示「Base URL 与已保存配置不同，请填写 API Key 后再测试连通性」、不出站——原实现违反了既有的「`api_key` 仅随发往该供应方自身 `base_url` 的请求发送」，本次是让实现回归既有契约。② 设置界面的 API Key 输入框不再把脱敏值当作可编辑内容（输入框恒为空，脱敏值只作占位提示与状态文案；输入非空即替换，删空即保留），避免改一个字就把真 key 覆盖成脱敏串。③ 只有用户主动点「测试连通」才用网关模型列表更新清单；关闭后重新打开设置界面不重放上一次的探测结果，不把已保存的精简清单还原成网关全量清单。④ 图片端点成功响应体的读取上限由 64 MiB 提到 128 MiB（4K 档大图的合法响应不再被拒），错误体与模型列表响应体的上限不变。不新增接口，请求与响应形态不变。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `image-create-agent`：生成参数增加必填的模型选择与携带上一张图、移除 `input_fidelity`；配置由单端点演进为双供应方并支持可配置模型清单；新增按供应方官方契约构造请求、逐项错误处理与结果的供应方 / 模型记录；「参考图不被持久化」的约束收窄为仅针对用户上传的参考图；新增供应方连通性探测，并明确 Base URL 只填主机根路径；模型清单以连通性探测返回的网关模型列表更新、以复选下拉框勾选，并新增模型初始选中项规则；会话历史按时间顺序交织呈现，建议 prompt 记录时间戳；prompt 迭代的结构化约定增加「未产出」出口，区分模型未产出与结构化输出失败，提示文案按情形拼接，仅模型明确未产出时折叠模型说明；合入前审查修复收紧了探测对已保存 key 的使用（仅当 Base URL 与已保存的一致）、API Key 输入框不再承载脱敏值、重新打开设置界面不重放探测结果。delta 共 **8 条 MODIFIED（49 个 Scenario）+ 17 条 ADDED（106 个 Scenario），155 个 Scenario**。

## Impact

- 后端：`src/cross_cutting/image_client.rs`（请求构造、响应解析、trait 签名）、`src/product/image_create/models.rs`（设置与结果模型、错误变体、`SettingsStoreApi`）、`settings_store.rs`（按家合并、校验、脱敏）、`engine.rs`（目标解析、参考图来源、结果字段）、`image_files.rs`（结果图片读取）、`src/web/handlers/image_create.rs`（生成与设置接口）；`session_store.rs` 的测试与 `src/web/handlers/image_create_endpoint_tests.rs` 仅随结果结构新增字段调整测试夹具。
- 前端：`web/src/api/types/image-create.ts`、`web/src/api/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/state/image-create-entries.ts`、`web/src/components/image-create/{ParamsPanel,SettingsDialog,ChatPane,ReferenceImageUpload}.tsx` 及对应测试。
- 连通性探测（2026-11-12 补充）：后端 `src/cross_cutting/image_client.rs`（探测实现）、`src/web/handlers/image_create.rs`（接口）、`src/web/app.rs`（路由）；前端 `web/src/api/types/image-create.ts`、`web/src/api/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/components/image-create/SettingsDialog.tsx` 及对应测试。不新增依赖，生成链路不变。
- 模型清单来源与复选下拉框（2026-11-12 补充）：后端 `src/cross_cutting/image_client.rs`（探测结果新增 `available_models`）、`src/product/image_create/models.rs`（单家清单上限 32 → 256）；前端 `web/src/api/types/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/components/image-create/SettingsDialog.tsx`、新增 `web/src/components/image-create/ModelMultiSelect.tsx` 与 `ModelListField.tsx` 及对应测试。不新增依赖、接口与出站请求，配置文件形态不变。
- 会话历史时间交织（2026-11-12 补充）：后端 `src/product/image_create/models.rs`（`PromptBlock` 新增可选 `ts`）、`engine.rs`（迭代落盘时写入）及对应测试；前端 `web/src/api/types/image-create.ts`、`web/src/state/image-create-store.ts`（`buildEntries`）与新增测试 `web/src/state/image-create-entry-order.test.ts`。会话读取 DTO 原样带出该字段，`src/web/handlers/image_create.rs` 不改。不新增依赖、接口与出站请求。
- 模型未产出与结构化输出失败（2026-11-12 补充）：后端 `src/product/image_create/prompt_iteration.rs`（提示契约双出口、`IterationOutcome.prompt_declined`）、`engine.rs`（迭代事件带出该标记）、`models.rs`（`IterationEvent` 新增 `prompt_declined`）、`src/web/handlers/image_create.rs`（仅错误事件的结构体字面量补该字段，推送逻辑不变）及对应测试；前端 `web/src/api/types/image-create.ts`、`web/src/state/image-create-store.ts`、`web/src/state/image-create-entries.ts`、`web/src/components/image-create/ChatPane.tsx` 与新增测试 `web/src/state/image-create-store.entries.test.ts`。不新增依赖、接口与出站请求，会话记录形态不变。
- 数据兼容：旧 `.aria/image_create.json` 必须可读且不丢凭据；旧会话记录（无供应方 / 模型字段、或仍内联 `b64`）必须可读、可作为上一张图；旧会话记录中没有时间戳的建议 prompt 必须可读，呈现时按同序号的用户消息定位。
- 不改动：会话存储的非测试实现（`session_store.rs`）、参考图内容校验规则、图片文件路径规则、prompt 迭代的执行器调用与 session 续接（其结构化约定与迭代事件自 2026-11-12 起有改动，见上文「模型未产出与结构化输出失败」一条）、会话删除流程。
- 发布说明：`CHANGELOG.md` 记录默认模型变更与配置文件不可降级。
- 技术方案：`cadence/designs/2026-11-11_技术方案_图片生成多模型与双家轮转_v1.0.md`（文件名沿用立项时的命名，其中的「双家轮转」已按 2026-11-12 的需求变更移除；内容为类型定义、字段规则表、时序、逐文件改动、测试用例清单）。
- 待真实调用确认的假设：MiniMax 接受「不带 `size`、带 `area=1K`」的请求（技术方案 8.2 节 A1），在联调工作包中确认。**证据等级分侧**：`H3ImageGenerationReq` 侧已由官方字段表直接确认；`H3ImageEditReq` 侧官方未逐项列出可选字段集，改图端点接受 `area` 属推定，A1 的联调必须覆盖 generations 与 edits 两侧。
- 设计评审（2026-11-12）复核结论：三条需求与四条硬约束全部满足（该评审早于同日的需求变更，其中原需求 3「自动轮转」随后已按用户要求移除）；`area=1K` 决策保留。评审发现的 3 条需修正（delta 漏改基线「生成结果按媒体类型直接展示」、前端 `saveSettings()` 前置 base_url 校验的编译级破坏点、D10 对 `area` 证据等级的越界）与 3 条建议优化已全部吸收，处置记录见技术方案 10.1 节。
- 归档时需手工同步主 spec 的 **Purpose 段**（delta 只作用于 Requirements），见 tasks 7.6。
- 需求变更（2026-11-12）：自动轮转已按用户 2026-11-12 要求撤回（「你就让用户在已配置并可用的provider里选model id就行，轮转不需要」）。模型改为用户手动选择，生成请求的 `model` 必填且不再有保留值，结果不再记录选择方式；内置 GPT 清单改为 `gpt-image-2.6-flare` 打头，使参数面板的初始选中项是 OpenAI 官方 id。
- 补充（2026-11-12）：按用户要求新增供应方连通性探测与 Base URL 填写说明（「base url 中给出案例，否则用户不知道是配到 v1 还是配到 /v1/images 还是配到 /v1/images/generations」「模型配置的时候提供一个是否连通测试的功能」），契约见技术方案 5.5、5.9、5.11。待真实调用确认的假设 A2（技术方案 8.2）：两家图片端点对「只有 `model`、缺 `prompt`」的请求返回 HTTP 400；不成立时探测结论偏保守（把连通的端点判为失败），不会误出图。
- 补充（2026-11-12）：按用户要求把模型清单的来源改为连通性探测并以复选下拉框呈现（「模型清单默认从测试连通性按钮获取,并渲染,用户可以自己删除部分或添加自定义的部分.做个复选下拉框这样的东西吧」），契约见技术方案 D1、5.5、5.9、5.11。主控裁决：单家上限 32 → 256；跨家去重保留，更新清单时跳过已被另一家勾选的 id；探测取得网关模型列表即以它**替换**清单（初版为「只扩充候选项、不改已有勾选」，因内置清单非空会使用户原话落空，同日修正为替换）；是否更新以 `available_models` 非空为准、不看 `ok`；参数面板初始选中项改为三级优先规则；探测第 2 步的模型 id 优先取含 `image` 的项。已知取舍：取消勾选过的模型再次探测会被重新勾上，网关未返回的自定义 id 再次探测后被移出清单。
- 缺陷修复（2026-11-12）：「图片创作对话过程顺序不对」——会话历史的四类条目此前首尾相接，未按时间交织。根因在数据层（建议 prompt 块无时间戳），契约见技术方案 5.1、5.8、5.12，用例见 7.6。已知取舍：旧会话中某一轮未产出建议 prompt 时，其后的旧建议 prompt 会对到更早一轮的用户消息上（旧数据没有可还原时刻的信息，技术方案 8.3 的 R11）。
- 体验修复（2026-11-12）：「模型拒答时用户以为 provider 断了」——模型正常跑完但不给结构化输出时，提示被埋在长文本之后。根因分两层：判定层把「解析成功但建议 prompt 为空」与「没解析出来」都折叠成无 prompt；呈现层把提示追加在 fallback 文本之后，且只在存在上一版时才提示。契约见技术方案 5.13，用例见 7.7。delta 本轮新增 1 条 MODIFIED（「结构化建议 prompt 的约定与降级」，6 个 Scenario）与 2 条 ADDED（「未产出建议 prompt 时的提示与模型说明呈现」7 个 Scenario、「未产出建议 prompt 时的提示文案」8 个 Scenario）。主控裁决（2026-11-12）：① 只要本轮未产出就提示，没有上一版时文案不带「，已保留上一版」；② 仅模型明确未产出时折叠模型说明，结构化输出失败时直接显示；③ 约定首句里的「不要输出其他内容」改为「除上面的 JSON 块外不要输出其他内容」并允许在 JSON 块之外说明原因，模型未附说明时提示不带「（下方是它给出的说明）」；④ `IterationEvent` 定义在 `models.rs`、`handlers/image_create.rs` 有它的结构体字面量，后端线获准触及这两个文件（只加字段与更新字面量）。假设 A3（技术方案 8.2）「真实执行器在拒答时会遵循新增的出口」已经真实调用验证成立（2026-11-12）：`claude_code` 以内容安全理由拒答时输出了约定的 JSON 块且 `suggested_prompt` 为空字符串，后端判定为模型未产出（`prompt_declined` 为真）；界面在模型说明之前显示「本轮模型未产出可用 prompt（下方是它给出的说明），已保留上一版」，模型说明收进默认收起的「查看模型说明」。实测记录见技术方案 10.6。`codex` 等其他执行器未单独实测；个别轮次或其他执行器不遵守时落入「结构化输出失败」，提示仍出现并排在前面、说明直接显示，文案为结构化输出失败的文案。
- 合入前审查修复（2026-11-12）：整分支代码审查与安全审查的结论为 0 个 Critical、3 个 Important、若干 Minor，逐条记录见 tasks 第 13 节，契约见技术方案 5.5、5.6、5.9、5.11，核对记录见 10.7。涉及文件：后端 `src/web/handlers/image_create.rs`（探测回落前比较 `base_url`）、`src/product/image_create/engine.rs`（迭代失败事件脱敏）、`src/cross_cutting/image_client.rs`（成功响应体上限）及对应测试；前端 `web/src/components/image-create/SettingsDialog.tsx` 及对应测试。迭代失败事件脱敏进入契约（API Key 安全边界新增 1 个 Scenario）。不新增依赖、接口与出站请求。delta 本轮不新增 Requirement，新增 6 个 Scenario：「网关配置录入与脱敏存储」2 个（API Key 输入框）、「连通性探测的取值来源与安全边界」1 个、「连通性探测的界面交互」1 个、「模型清单以连通性探测结果为来源」1 个、「API Key 安全边界与出站目标约束」1 个（迭代失败的事件不携带 key 明文）；改写这四条与「API Key 安全边界与出站目标约束」的正文；并把「未产出建议 prompt 时的提示与模型说明呈现」「未产出建议 prompt 时的提示文案」两条下以「有无上一版」为前提的 Scenario 统一为「提示词编辑框非空 / 为空」，与 Requirement 正文及技术方案 8.3 的 R14 一致。现为 8 条 MODIFIED（49 个 Scenario）+ 17 条 ADDED（106 个 Scenario），共 155 个 Scenario。
