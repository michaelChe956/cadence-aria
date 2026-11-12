## MODIFIED Requirements

### Requirement: 图片生成由用户显式触发且参数可配置

系统 SHALL 仅在用户显式触发（如点击生成按钮）时发起图片生成，不得由执行器自主触发。生成请求 SHALL 携带用户可配置的参数：`size`、`quality`、`background`、`output_format`、`model`、`use_previous`。系统 SHALL 仅接受预定义取值：`size` ∈ {1024x1024, 1536x1024, 1024x1536, auto}；`quality` ∈ {low, medium, high, auto}；`background` ∈ {transparent, opaque, auto}；`output_format` ∈ {png, jpeg, webp}；`model` 必须是已配置且可用的供应方模型清单中的某个 model id，缺省或去除首尾空白后为空视为非法参数；`use_previous` ∈ {true, false}，缺省视为 `false`。系统 SHALL NOT 提供 `input_fidelity` 参数，SHALL NOT 在任何出站请求中发送 `input_fidelity`。系统 SHALL 每次生成请求恰好生成一张图片（不支持多图批量）。

#### Scenario: 用户点按钮触发生成

- **WHEN** 用户确认 prompt 并点击「生成」按钮提交选定参数
- **THEN** 系统向后端发起图片生成请求，使用所提交的 prompt 与参数调用本次选定供应方的图片端点，请求恰好生成一张图片

#### Scenario: 拒绝越界参数值

- **WHEN** 前端试图提交不在预定义枚举内的参数值（如 `quality=ultra`）
- **THEN** 系统不发起该生成请求并提示参数非法

#### Scenario: 拒绝非法的 use_previous 取值

- **WHEN** 生成请求的 `use_previous` 不是 `true` 或 `false`（如 `yes`）
- **THEN** 系统不发起该生成请求并提示参数非法

#### Scenario: 缺少 model 时不发起请求并提示参数非法

- **WHEN** 生成请求未携带 `model`，或 `model` 去除首尾空白后为空
- **THEN** 系统不发起该生成请求并提示参数非法（请选择图片生成模型）

#### Scenario: 缺省 use_previous 时不携带上一张图

- **WHEN** 生成请求携带了合法的 `model` 但未携带 `use_previous`
- **THEN** 系统按 `use_previous=false` 处理，不携带上一张生成图

#### Scenario: 文生图时忽略 input_fidelity

- **WHEN** 调用方在无参考图（文生图）情况下仍提交了 `input_fidelity`
- **THEN** 系统接受该请求并忽略该字段，不在文生图请求中发送 `input_fidelity`

#### Scenario: 改图时同样不发送 input_fidelity

- **WHEN** 调用方在有参考图（改图）情况下仍提交了 `input_fidelity`
- **THEN** 系统接受该请求并忽略该字段，发往任一供应方的改图请求中都不出现 `input_fidelity`

### Requirement: 参考图改图自动选择端点

系统 SHALL 支持每次生成至多使用单张参考图，其来源为用户上传的参考图或会话上一张生成图（后者见「连续修改携带上一张生成图」）。系统 SHALL 根据本次是否存在参考图自动选择所选供应方的端点：无参考图时调用 `/v1/images/generations`（文生图）；有参考图时调用 `/v1/images/edits`（参考图改图，以 multipart 的 `image[]` 字段提交参考图并与 prompt 一并发送）。该端点选择规则 SHALL 对两个供应方一致，且对用户透明。

#### Scenario: 无参考图走文生图

- **WHEN** 用户在既未上传参考图、也未携带上一张生成图的情况下触发生成
- **THEN** 系统调用 `/v1/images/generations` 以纯 prompt 生成

#### Scenario: 有参考图走改图

- **WHEN** 用户在提供单张参考图的情况下触发生成
- **THEN** 系统调用 `/v1/images/edits`，将参考图与 prompt 一并提交，基于参考图生成

#### Scenario: 携带上一张生成图时走改图

- **WHEN** 本次生成以会话上一张生成图作为参考图
- **THEN** 系统调用 `/v1/images/edits`，以 `image[]` 提交该图片

#### Scenario: 仍只接受单张上传参考图

- **WHEN** 生成请求中包含两张上传的参考图
- **THEN** 系统拒绝该请求并提示仅支持单张参考图，不发起生成

### Requirement: 参考图输入约束

系统 SHALL 对每次生成使用的参考图施加约束，无论其来源是用户上传还是会话上一张生成图：单张；仅允许静态图像 MIME 类型（png/jpeg/webp），拒绝动图与多帧图像；最大字节数 SHALL 不超过 10 MB；解码后单边像素 SHALL 不超过 4096、总像素 SHALL 不超过 4096×4096。系统 SHALL 在向图片端点发送前由后端按**实际文件内容**校验格式、字节数与解码后尺寸（不只信任前端或声明 MIME），SHALL 拒绝不符合约束的参考图并给出明确原因。用户上传的参考图 SHALL 仅在本次生成请求生命周期内使用，SHALL NOT 在生成完成后持久化保留，SHALL NOT 写入会话记录。连续修改所携带的上一张生成图是会话中已持久化的生成结果图片文件本身；系统 SHALL NOT 为携带而另存其副本，其生命周期 SHALL 随会话记录，不属于参考图持久化。

#### Scenario: 拒绝超大或格式不符的参考图

- **WHEN** 用户上传超过 10 MB、不在允许 MIME 内、或解码后像素超过 4096×4096 的参考图
- **THEN** 系统拒绝该参考图并提示具体原因，不发起生成

#### Scenario: 后端按实际内容校验，拒绝伪造声明的参考图

- **WHEN** 参考图的声明 MIME 与实际文件内容不符（如声明 png 实为其他格式），或为动图/多帧图像
- **THEN** 后端在发出请求前检出并拒绝，不发起生成

#### Scenario: 参考图不被持久化

- **WHEN** 一次带用户上传参考图的生成完成
- **THEN** 系统不在会话历史或磁盘上持久化保留该上传的参考图文件，图片目录中只新增本次的生成结果

#### Scenario: 携带上一张生成图不产生副本

- **WHEN** 一次以上一张生成图为参考图的生成完成
- **THEN** 图片目录中只新增本次的生成结果，上一张生成图的文件未被复制或改写

#### Scenario: 上传参考图后的下一轮携带的是生成结果

- **WHEN** 用户先上传参考图生成了一张图，下一轮在未上传参考图且 `use_previous=true` 的情况下触发生成
- **THEN** 系统携带的是上一轮的生成结果图片，而不是当初上传的参考图

### Requirement: 生成结果按媒体类型直接展示

系统 SHALL 将图片端点返回的单张 `b64_json` 解码后按其 `output_format` 对应的媒体类型持久化为图片文件（`.aria/image_create/images/` 下，稳定 `image_id` 命名），会话历史中的生成结果 SHALL 只保存图片引用（`image_id` 与 `media_type`）与非图片元数据（供应方、模型 id、参考图来源），SHALL NOT 内联 base64 字节。前端 SHALL 通过图片读取端点按 URL 懒加载展示，SHALL NOT 依赖会话记录 API 返回图片字节，SHALL NOT 固定为某一种图片格式。系统 SHALL 将生成结果（含所用 prompt 与参数及图片引用）纳入该会话历史供回看。

#### Scenario: 按输出格式构造数据 URI

- **WHEN** 图片端点返回 `b64_json` 且本次 `output_format` 为 webp
- **THEN** 系统以 webp 媒体类型将图片字节写入 images 目录，会话历史记录 `image_id` 与 `media_type`，前端经图片端点按 URL 以 webp 媒体类型渲染该图片

#### Scenario: 打开会话不传输图片字节

- **WHEN** 用户打开一个含多张历史生成图片的会话
- **THEN** 会话记录 API 响应不包含任何图片二进制或 base64 内容，前端按需逐张加载图片

#### Scenario: 结果纳入会话历史

- **WHEN** 一次图片生成完成
- **THEN** 该结果与对应 prompt、参数及图片引用一并记录进会话历史，用户可在会话中回看

#### Scenario: 图片文件缺失时的可读错误

- **WHEN** 会话历史中的图片引用对应的图片文件在磁盘上缺失
- **THEN** 图片读取端点返回明确错误（非 2xx），前端在该图片位置展示可读的缺失提示，不影响会话其余内容的加载

### Requirement: 图片生成失败与重试边界

系统 SHALL 对图片端点的连接失败、超时、4xx、5xx、空 `data`、缺 `b64_json`、以及 HTTP 成功但 `data[]` 中的逐项 `error` 进行归一并向用户展示可读错误。系统 SHALL 逐项检查响应 `data[]`：存在带非空 `b64_json` 的项时取第一个这样的项作为结果；不存在这样的项且有项带 `error` 时，SHALL 以该 `error` 的信息作为失败原因。系统 SHALL NOT 对已发出的图片生成请求自动重试（防止重复计费），SHALL NOT 在一次生成失败后自动改用另一供应方再次请求；一次生成请求 SHALL 至多产生一次出站调用。用户可显式重新触发生成。系统 SHALL NOT 在生成失败时写入成功结果，SHALL 将失败事件（含可读错误，不含原始敏感鉴权信息）记录进会话历史。当生成已成功但会话历史持久化失败时，系统 SHALL NOT 向用户宣称历史已保存，SHALL 向用户即时返回明确错误并在进程内保留该事件，待写入恢复后重试持久化。

#### Scenario: 网关错误不自动重试

- **WHEN** 图片生成请求返回错误（如超时、429、5xx 或空 data）
- **THEN** 系统不自动重试该请求，向用户展示可读错误，并允许用户显式重新触发

#### Scenario: 生成成功但历史写盘失败

- **WHEN** 图片生成成功但将会话历史写入磁盘失败
- **THEN** 系统不宣称历史已保存，向用户即时返回明确错误，并在进程内保留该结果/事件，待写入恢复后重试持久化

#### Scenario: 失败不写入成功结果

- **WHEN** 一次图片生成失败
- **THEN** 系统不在会话历史写入成功生成结果，仅记录一条失败事件

#### Scenario: HTTP 成功但唯一一项带 error

- **WHEN** 图片端点返回 HTTP 200，且 `data[]` 的唯一一项只含 `error`
- **THEN** 系统判定本次生成失败，向用户展示该 `error` 中的信息，不写入成功结果

#### Scenario: 失败后不改用另一供应方

- **WHEN** 用户所选模型所属的供应方返回错误
- **THEN** 系统不向另一供应方发起请求，本次生成只产生过一次出站调用

#### Scenario: 响应只有 URL 时判定失败

- **WHEN** 图片端点返回 HTTP 200，且 `data[]` 的项只含 `url` 而无 `b64_json`
- **THEN** 系统判定本次生成失败并提示端点未返回图片数据，不向该 URL 发起任何请求

### Requirement: 网关配置录入与脱敏存储

系统 SHALL 提供设置界面供用户分别为两个供应方（GPT、MiniMax）录入各自的 `base_url`、`api_key` 与模型清单，以及两家共享的默认参数。系统 SHALL 将每个供应方的 `base_url` 视为主机根路径（如 `https://api.openai.com`，不含 `/v1`、`/v1/images` 等路径后缀），并在其后自动拼接 `/v1/images/generations`（文生图）或 `/v1/images/edits`（改图）构造出站地址；设置界面 SHALL 在每个供应方的 Base URL 输入框给出填写示例并说明该规则。系统 SHALL 将配置持久化于 Aria 后端 `.aria` 目录下。系统 SHALL 在前端展示每个 `api_key` 时对其脱敏（不展示明文），且前端 SHALL NOT 缓存 `api_key` 明文。设置更新 SHALL 按供应方独立采用保留语义：某个供应方未出现在更新请求中时，系统 SHALL 保留该供应方的全部配置；某个供应方出现但未提供新 `api_key`（保留动作）时，系统 SHALL 保留其原 `api_key` 而非清空；清除某个供应方的 `api_key` SHALL 需要用户针对该供应方的显式清除动作。对一个供应方的更新 SHALL NOT 改变另一个供应方的配置。任一供应方的配置校验失败时，系统 SHALL 拒绝整次更新且不改动已持久化的配置。

#### Scenario: 录入并持久化配置

- **WHEN** 用户在设置界面为某个供应方录入 `base_url`、`api_key` 并保存
- **THEN** 系统将配置持久化于后端 `.aria` 目录，后续发往该供应方的生成请求使用该配置

#### Scenario: 前端展示脱敏

- **WHEN** 配置已保存后用户再次打开设置界面
- **THEN** 每个供应方的 `api_key` 以脱敏形式展示（如 `sk-****1234`），不显示完整明文

#### Scenario: 仅改默认参数不清空 key

- **WHEN** 用户在未重新输入任何 `api_key`（展示为脱敏占位）的情况下修改默认参数并保存
- **THEN** 系统保留两个供应方的原 `api_key`，仅更新默认参数

#### Scenario: 只更新一个供应方不影响另一个

- **WHEN** 用户只替换 MiniMax 的 `api_key` 并保存
- **THEN** GPT 的 `base_url`、`api_key` 与模型清单均保持不变

#### Scenario: 清除一个供应方的 key

- **WHEN** 用户对 GPT 执行显式清除 `api_key` 的动作并保存
- **THEN** GPT 的 `api_key` 被清空，MiniMax 的 `api_key` 保持不变

#### Scenario: 校验失败时整次更新不落盘

- **WHEN** 更新请求中一个供应方的配置合法、另一个供应方的 `base_url` 不合法
- **THEN** 系统拒绝该更新并提示原因，已持久化的两家配置均未改变

#### Scenario: Base URL 只填主机根路径时系统自动拼接端点路径

- **WHEN** 用户为某个供应方录入 `base_url` 为 `https://gateway.example.com`（末尾带或不带 `/`）并保存
- **THEN** 发往该供应方的文生图请求地址为 `https://gateway.example.com/v1/images/generations`，改图请求地址为 `https://gateway.example.com/v1/images/edits`

#### Scenario: 设置界面给出 Base URL 填写示例

- **WHEN** 用户打开设置界面查看某个供应方的 Base URL 输入框
- **THEN** 输入框带有该供应方的示例地址（GPT 为 `https://api.openai.com`，MiniMax 为 `https://api.minimax.io`），并提示只填到主机根路径、不要带 `/v1` 或 `/v1/images`，系统会自动拼接图片端点路径

### Requirement: API Key 安全边界与出站目标约束

系统 SHALL 仅在 Aria 后端读取与使用各供应方的 `api_key`（用于发起图片请求的鉴权头）。系统 SHALL NOT 将任何供应方的 `api_key` 传递给 prompt 迭代所用的 CLI 执行器子进程。每个供应方的 `api_key` SHALL 仅随发往该供应方自身 `base_url` 的请求发送，SHALL NOT 出现在发往另一供应方的请求中。系统 SHALL 对用户录入的每个供应方的 `base_url` 施加出站安全约束：仅允许 HTTPS（或本地回环），在保存设置时与每次出站前均 SHALL 校验。系统 SHALL 默认禁用图片请求的自动重定向；如需允许重定向，SHALL 仅允许目标与原请求为同一 origin（scheme、host、port 三者全同），SHALL 拒绝任何 HTTPS→HTTP 的降级重定向，且 SHALL NOT 在任何跨 origin 或降级重定向中携带 Authorization 头。系统 SHALL NOT 向图片端点响应体中给出的任何 URL 发起请求。系统 SHALL NOT 在错误日志、诊断输出或测试夹具中明文记录任何 `api_key`。

#### Scenario: 执行器子进程不接触 key

- **WHEN** 系统发起一轮 prompt 迭代（启动/续接 CLI 执行器子进程）
- **THEN** 该子进程不接收任何供应方的 `api_key`，key 仅在后端用于图片生成请求

#### Scenario: 缺少有效配置时拒绝生成

- **WHEN** 用户所选模型所属的供应方未录入 `base_url` 或 `api_key`
- **THEN** 系统不发起任何图片请求并提示用户先完成对应供应方的配置

#### Scenario: 拒绝非 HTTPS 的 base_url

- **WHEN** 用户为任一供应方录入的 `base_url` 不是 HTTPS（且非本地回环）
- **THEN** 系统拒绝该 `base_url` 并提示必须使用 HTTPS

#### Scenario: 重定向默认禁用，仅同 origin 且不降级时才跟随

- **WHEN** 图片请求被端点重定向
- **THEN** 系统默认不自动跟随；若跟随，仅当目标与原请求 scheme、host、port 全同且非 HTTPS→HTTP 降级时才携带 Authorization，否则不携带或拒绝

#### Scenario: key 只发往自己的供应方

- **WHEN** 两个供应方均已配置且系统向 MiniMax 发起一次生成请求
- **THEN** 该请求的目标为 MiniMax 的 `base_url`、鉴权头使用 MiniMax 的 `api_key`，GPT 的 `api_key` 不出现在该请求中

#### Scenario: 错误信息不含 key 明文

- **WHEN** 图片端点的错误响应体或逐项错误信息中包含本次使用的 `api_key` 明文
- **THEN** 系统展示与记录的错误信息中该明文已被替换

## ADDED Requirements

### Requirement: 模型清单可配置

系统 SHALL 为每个供应方维护一份用户可配置的模型 id 清单，并 SHALL 内置推荐默认值；用户 SHALL 可在设置中增删清单项。保存时系统 SHALL 校验清单：每个 id 长度为 1–128 且仅含字母、数字与 `. _ : / -`；同一供应方内不重复、两个供应方之间不重复；每个供应方至多 32 个。校验不通过时系统 SHALL 拒绝保存并给出原因。参数面板 SHALL 提供模型选择，选项为已配置且可用（已配置 `base_url`、`api_key` 且模型清单非空）的供应方的清单项，SHALL NOT 含自动轮转项；模型 SHALL 由用户手动选择，初始选中项为第一个可用供应方清单的首个 id；设置变更后所选模型不再属于任何可用供应方的清单时，SHALL 重置为该规则的结果。没有任何可用供应方时，所选模型 SHALL 为空，生成按钮 SHALL 禁用，并 SHALL 提示用户先在设置里配置并启用至少一个供应方。

#### Scenario: 内置推荐清单包含 8 个 gpt 模型与 1 个 MiniMax 模型

- **WHEN** 用户从未修改过模型清单并打开设置
- **THEN** GPT 清单依次恰为 `gpt-image-2.6-flare`、`gpt-image-2.6`、`gpt-image-2.6-flare-2k`、`gpt-image-2.6-flare-4k`、`gpt-image-2.6-sunburst`、`gpt-image-2.6-sunburst-2k`、`gpt-image-2.6-sunburst-4k`、`gpt-image-2.5`；MiniMax 清单恰为 `MiniMax-H3-Image`

#### Scenario: 用户新增的模型可被选择

- **WHEN** 用户在 GPT 清单中新增一个 id 并保存
- **THEN** 该 id 出现在参数面板的模型选项中，选择它生成时其值原样作为 `model` 发往 GPT 的 `base_url`

#### Scenario: 不合法的清单被拒绝

- **WHEN** 用户保存的清单含重复项、含不合法字符的 id、或同一 id 同时出现在两个供应方的清单中
- **THEN** 系统拒绝保存并提示具体原因，已持久化的配置未改变

#### Scenario: 参数面板只列出已配置且可用的供应方的模型

- **WHEN** 仅 GPT 配置了 `base_url` 与 `api_key` 且其模型清单非空
- **THEN** 参数面板的模型选项恰为 GPT 清单中的模型，不含自动轮转项，并提示 MiniMax 尚未配置

#### Scenario: 初始选中第一个可用供应方清单的首个模型

- **WHEN** 仅 GPT 已配置且可用，用户首次打开参数面板
- **THEN** 所选模型为 GPT 清单的首个 id

#### Scenario: 所选模型不再可用时重置

- **WHEN** 用户在设置中把当前所选模型从清单中删除并保存
- **THEN** 所选模型重置为第一个可用供应方清单的首个 id；已无可用供应方时重置为空

#### Scenario: 无可用供应方时生成按钮禁用并提示先配置

- **WHEN** 没有任何供应方同时配置了 `base_url`、`api_key` 且模型清单非空
- **THEN** 参数面板无模型可选，「生成」按钮禁用，并提示用户先在设置里配置并启用至少一个供应方

### Requirement: 模型 id 原样透传且供应方由清单归属决定

一个模型所属的供应方 SHALL 仅由它被配置在哪个供应方的清单中决定；系统 SHALL NOT 依据模型 id 的文本内容或 `base_url` 的主机名推断供应方或端点身份。生成请求的 `model` SHALL 与某个已配置且可用（已配置 `base_url`、`api_key` 且模型清单非空）的供应方的清单项逐字符相等，否则系统 SHALL 拒绝该请求且不发起任何出站请求：该 id 不在任何供应方清单中时返回参数错误；该 id 所属供应方未配置 `base_url` 或 `api_key` 时提示该供应方尚未配置。系统 SHALL 使用所选模型及其所属供应方，SHALL NOT 改用另一供应方。系统 SHALL 将所选模型 id 原样写入出站请求的 `model` 字段，SHALL NOT 依据模型 id 的文本内容改变任何其他请求字段，SHALL NOT 为表达分辨率档位而发送供应方官方契约之外的字段。

#### Scenario: 显式选择的模型原样出现在出站请求中

- **WHEN** 用户在参数面板选择 `gpt-image-2.6-sunburst-2k` 并触发生成
- **THEN** 出站请求发往 GPT 的 `base_url`，其 `model` 字段的值恰为 `gpt-image-2.6-sunburst-2k`

#### Scenario: 带分辨率后缀的模型不改变其他请求字段

- **WHEN** 在其余参数完全相同的情况下，分别以 `gpt-image-2.6-flare` 与 `gpt-image-2.6-flare-4k` 触发生成
- **THEN** 两次出站请求体除 `model` 的值之外完全相同，且都不含 `resolution`、`area` 等用于表达分辨率档位的额外字段

#### Scenario: 清单外的模型被拒绝且不出站

- **WHEN** 生成请求的 `model` 不与任何供应方的清单项逐字符相等（包括仅大小写不同）
- **THEN** 系统返回参数错误并指出该模型不在已配置且可用的供应方清单中，不发起任何出站请求

#### Scenario: 供应方由清单归属决定

- **WHEN** 用户把一个任意命名的 id 配置在 MiniMax 清单中并显式选择它生成
- **THEN** 系统按 MiniMax 的请求规则构造请求并发往 MiniMax 的 `base_url`

#### Scenario: 所选模型的供应方未配置时不换家

- **WHEN** 生成请求的 `model` 在 MiniMax 清单中，而 MiniMax 未配置 `api_key`、GPT 可用
- **THEN** 系统提示 MiniMax 尚未配置 Base URL 或 API Key，不发起出站请求，也不改用 GPT

### Requirement: 连续修改携带上一张生成图

当生成请求未包含用户上传的参考图、`use_previous` 为 `true`、且会话已有成功的生成结果时，系统 SHALL 由后端读取会话最后一条成功生成结果的图片，将其作为本次的单张参考图。用户上传了参考图时，系统 SHALL 使用上传的参考图并忽略 `use_previous`，SHALL NOT 同时发送两张图。`use_previous` 为 `false` 或会话尚无生成结果时，系统 SHALL NOT 携带任何图片。被携带的图片 SHALL 经过与上传参考图相同的输入约束校验。上一张生成图无法读取或未通过校验时，系统 SHALL 使本次生成失败并给出可读原因，SHALL NOT 发起出站请求，SHALL NOT 静默退化为文生图，SHALL NOT 改用更早的生成结果。携带 SHALL 不受供应方限制：上一张图由哪个供应方生成不影响它被携带给本次选定的供应方。前端 SHALL 提供「基于上一张图继续修改」开关，默认打开，用户可关闭。

#### Scenario: 携带上一张生成图

- **WHEN** 会话已有一张生成图，用户未上传参考图并以 `use_previous=true` 触发生成
- **THEN** 出站请求发往 `/v1/images/edits`，其 `image[]` 的内容与上一张生成图的文件内容相同

#### Scenario: 会话尚无生成结果时直接文生图

- **WHEN** 会话尚无生成结果，用户以 `use_previous=true` 触发生成
- **THEN** 出站请求发往 `/v1/images/generations`，不含图片

#### Scenario: 上传的参考图优先

- **WHEN** 会话已有生成图，用户上传了参考图并以 `use_previous=true` 触发生成
- **THEN** 出站请求的 `image[]` 为用户上传的参考图，且只含这一张图

#### Scenario: 关闭携带时不带图

- **WHEN** 会话已有生成图，用户未上传参考图并以 `use_previous=false` 触发生成
- **THEN** 出站请求发往 `/v1/images/generations`，不含图片

#### Scenario: 上一张图文件缺失时明确失败

- **WHEN** 会话最后一条生成结果的图片文件已不存在且无可用的内联图片数据，用户以 `use_previous=true` 触发生成
- **THEN** 系统返回错误说明上一张图片不可用，并提示可关闭该开关或上传参考图；不发起任何出站请求

#### Scenario: 上一张图超过输入约束时明确失败

- **WHEN** 会话最后一条生成结果的图片超过参考图输入约束（如超过 10 MB），用户以 `use_previous=true` 触发生成
- **THEN** 系统返回错误说明上一张图片不可用及具体校验原因；不发起任何出站请求

#### Scenario: 旧会话内联图片可被携带

- **WHEN** 会话最后一条生成结果是尚未迁移为文件、仍内联 `b64` 的旧记录，用户以 `use_previous=true` 触发生成
- **THEN** 系统解码该内联图片并将其作为参考图发送

#### Scenario: 跨供应方携带

- **WHEN** 两个供应方均可用，上一张生成图由 GPT 的模型生成，用户随后选择 MiniMax 清单中的模型并以 `use_previous=true` 触发生成
- **THEN** 请求发往 MiniMax 的 `/v1/images/edits`，其 `image[]` 为 GPT 生成的那张图

#### Scenario: 前端开关默认打开且可关闭

- **WHEN** 用户首次进入图片创作会话
- **THEN** 「基于上一张图继续修改」开关处于打开状态，用户关闭后触发的生成请求以 `use_previous=false` 提交

### Requirement: GPT 请求按官方契约构造

系统 SHALL 仅依据本次选定的供应方决定出站请求的字段集，对文生图与改图两种端点 SHALL 应用同一套字段规则。发往 GPT 的请求 SHALL 包含 `model`、`prompt`、`size`、`quality`、`background`、`output_format`（改图另含 `image[]`），SHALL NOT 包含 `input_fidelity`、`response_format`、`area`、`aspect_ratio`、`n`、`stream`。系统 SHALL 以响应 `data[]` 中的 `b64_json` 作为图片数据来源，图片媒体类型 SHALL 由请求的 `output_format` 决定。

#### Scenario: GPT 文生图请求字段

- **WHEN** 系统向 GPT 发起文生图请求
- **THEN** 请求体的字段恰为 `model`、`prompt`、`size`、`quality`、`background`、`output_format`

#### Scenario: GPT 在 size 为 auto 时原样发送

- **WHEN** 系统向 GPT 发起文生图请求且用户选择的 `size` 为 `auto`
- **THEN** 请求体含 `size=auto`，不含 `area`

#### Scenario: GPT 改图请求字段

- **WHEN** 系统向 GPT 发起参考图改图请求
- **THEN** multipart 表单含 `model`、`prompt`、`size`、`quality`、`background`、`output_format`、`image[]`，不含 `input_fidelity`、`response_format`、`area`

### Requirement: MiniMax 请求按官方契约构造

发往 MiniMax 的请求 SHALL 包含 `model`、`prompt`、`output_format` 与 `response_format=b64_json`（改图另含 `image[]`），SHALL NOT 包含 `quality`、`background`、`input_fidelity`、`aspect_ratio`、`n`、`stream`。当用户选择的 `size` 为具体宽高时，请求 SHALL 包含该 `size` 且不包含 `area`；当用户选择的 `size` 为 `auto` 时，请求 SHALL 包含 `area=1K` 且不包含 `size`。文生图与改图两种端点 SHALL 应用同一套字段规则。系统 SHALL 以响应 `data[]` 中的 `b64_json` 作为图片数据来源，图片媒体类型 SHALL 由请求的 `output_format` 决定。

#### Scenario: MiniMax 文生图请求字段（具体尺寸）

- **WHEN** 系统向 MiniMax 发起文生图请求且用户选择的 `size` 为 `1536x1024`
- **THEN** 请求体的字段恰为 `model`、`prompt`、`size`、`output_format`、`response_format`，其中 `response_format` 为 `b64_json`

#### Scenario: MiniMax 在 size 为 auto 时改发 area

- **WHEN** 系统向 MiniMax 发起文生图请求且用户选择的 `size` 为 `auto`
- **THEN** 请求体的字段恰为 `model`、`prompt`、`area`、`output_format`、`response_format`，其中 `area` 为 `1K`，请求体不含 `size`

#### Scenario: MiniMax 改图请求不含 GPT 专有字段

- **WHEN** 系统向 MiniMax 发起参考图改图请求
- **THEN** multipart 表单含 `model`、`prompt`、`output_format`、`response_format`、`image[]` 以及 `size` 与 `area` 二者之一，不含 `quality`、`background`、`input_fidelity`

#### Scenario: 手动切换供应方时的携带链可持续

- **WHEN** 两个供应方均可用，用户保持 `size=auto`、`use_previous=true`，在同一会话中依次选择 GPT、MiniMax、GPT 清单中的模型连续触发三次生成，且各次请求均成功
- **THEN** 第二次发往 MiniMax 的请求含 `area=1K`，第三次生成能以第二次的生成结果作为参考图发出，不因上一张图超过输入约束而失败

### Requirement: 生成结果记录供应方与模型

系统 SHALL 在每条成功的生成结果中记录：实际使用的供应方、实际发送的模型 id、参考图来源（无、用户上传、上一张生成图）。生成接口的成功响应与会话记录读取接口 SHALL 返回这些信息。前端 SHALL 在生成图处展示供应方与模型，并标示参考图来源。对升级前产生、未记录这些信息的旧结果，系统 SHALL 正常读取与展示图片，SHALL NOT 报错，前端 SHALL NOT 为其展示供应方与模型标注。图片端点返回的错误（含写入会话历史的失败事件）SHALL 带有本次所用的供应方与模型，SHALL NOT 含 `api_key` 明文。

#### Scenario: 成功结果记录供应方与模型

- **WHEN** 用户选择 MiniMax 清单中的某个模型并以上一张生成图为参考图的生成成功
- **THEN** 该结果记录供应方为 MiniMax、模型为所选的模型 id、参考图来源为上一张生成图，且生成接口响应包含同样的信息

#### Scenario: 前端展示供应方与模型

- **WHEN** 用户查看一条升级后产生的生成图
- **THEN** 该图片处显示其供应方与模型 id；携带了上一张图或使用了上传参考图的带有相应标示

#### Scenario: 重新打开会话后信息仍在

- **WHEN** 用户重新打开一个包含升级后生成结果的会话
- **THEN** 会话记录读取接口返回每条结果的供应方、模型与参考图来源，前端据此展示

#### Scenario: 旧结果照常展示

- **WHEN** 用户打开一个只含升级前生成结果的会话
- **THEN** 图片照常加载展示，不显示供应方与模型标注，不出现错误

#### Scenario: 失败信息带供应方与模型

- **WHEN** 发往某个供应方的生成请求失败
- **THEN** 展示给用户的错误与写入会话历史的失败事件均标明本次所用的供应方与模型 id

### Requirement: 配置文件向后兼容

系统 SHALL 能读取升级前的配置文件形态（单组 `base_url`、`api_key` 与默认参数），读入时 SHALL 将该组 `base_url` 与 `api_key` 归入 GPT 供应方、MiniMax 供应方的凭据为空、两个供应方的模型清单取内置推荐默认值、默认参数保持不变，SHALL NOT 丢失原有 `api_key`。读取 SHALL NOT 改写配置文件；用户下一次保存设置时系统 SHALL 以按供应方分组的新形态写回。读取新形态文件时，某个供应方缺少模型清单的，系统 SHALL 以内置推荐默认值补齐；模型清单为显式空列表的，系统 SHALL 保持为空（视为用户主动清空，该供应方不可用）。

#### Scenario: 旧配置文件的凭据归入 GPT

- **WHEN** 系统读取一个升级前保存的、只含单组 `base_url` 与 `api_key` 的配置文件
- **THEN** GPT 的 `base_url` 与 `api_key` 等于旧值，MiniMax 的凭据为空，GPT 清单为内置的 8 个 id，原默认参数不变

#### Scenario: 升级后无需重新配置即可生成

- **WHEN** 仅有旧配置文件的用户升级后选择 GPT 内置清单中的某个模型触发生成
- **THEN** 请求使用旧文件中的 `base_url` 与 `api_key` 发往 GPT，`model` 为所选的模型 id

#### Scenario: 保存后写为新形态

- **WHEN** 系统读入旧配置文件后用户保存了一次设置
- **THEN** 配置文件为按供应方分组的形态，原 `api_key` 保留在 GPT 供应方下，再次读取得到相同的配置

#### Scenario: 缺少模型清单时补默认值

- **WHEN** 新形态配置文件中某个供应方没有模型清单字段
- **THEN** 系统读入后该供应方的模型清单为内置推荐默认值

#### Scenario: 显式清空的清单保持为空

- **WHEN** 新形态配置文件中某个供应方的模型清单为显式空列表
- **THEN** 系统读入后该清单仍为空，该供应方被视为不可用

### Requirement: 供应方连通性探测

系统 SHALL 提供供应方连通性探测，供用户在设置界面检查单个供应方的 `base_url`、`api_key` 与模型清单是否可用。探测 SHALL 依次发起至多两次出站请求：① 以 Bearer 鉴权 `GET {base_url}/v1/models`；② `POST {base_url}/v1/images/generations`，请求体只含 `model`、不含 `prompt`。每步 SHALL 至多请求一次。探测 SHALL NOT 携带 `prompt`，SHALL NOT 调用 `/v1/images/edits`，因而不生成图片、不产生费用。探测结果 SHALL 给出总体结论、每一步的状态（通过、跳过或失败）与中文说明，以及配置了但未被网关模型列表列出的模型 id。当且仅当没有任何一步失败时，结论 SHALL 为连通；存在未被列出的模型 SHALL NOT 使结论变为不连通。

#### Scenario: 连通且 key 有效

- **WHEN** 用户对某个供应方发起连通性探测，网关的 `/v1/models` 返回 200 且列出了清单中的全部模型，不含 `prompt` 的 `/v1/images/generations` 请求返回 400
- **THEN** 探测结论为连通，两步均标记为通过，未被列出的模型为空

#### Scenario: API Key 无效或无权限

- **WHEN** 网关的 `/v1/models` 返回 401 或 403
- **THEN** 探测结论为不连通并说明 API Key 无效或无权限，系统不再向 `/v1/images/generations` 发起请求

#### Scenario: 路径不对时提示只填主机根路径

- **WHEN** 用户把 `base_url` 填成带 `/v1` 或 `/v1/images` 后缀的地址，网关未在拼接后的地址提供模型列表，且 `/v1/images/generations` 请求返回 404
- **THEN** 探测结论为不连通，并提示系统会自动拼接 `/v1/images/generations`、`base_url` 只填到主机根路径

#### Scenario: 网关未列出部分配置模型

- **WHEN** 网关的 `/v1/models` 返回 200，但其模型列表不含清单中的某些模型 id，且图片端点一步通过
- **THEN** 探测结果逐个列出这些未被网关列出的模型 id，结论仍为连通

#### Scenario: 探测不生成图片不计费

- **WHEN** 用户对任一供应方发起连通性探测
- **THEN** 系统至多发出一次 `GET /v1/models` 与一次不含 `prompt` 的 `POST /v1/images/generations`，不发出任何带 `prompt` 的请求，也不调用 `/v1/images/edits`；图片目录与会话记录均无新增

### Requirement: 连通性探测的逐步判定规则

第 ① 步（模型列表）返回 200 时，系统 SHALL 判定该步通过，并将请求中的模型清单与网关返回的模型列表逐字符比对，列出未被列出的模型 id；返回 401 或 403 时 SHALL 判定 API Key 无效或无权限并结束探测，不再发起第 ② 步；其他情况（含网络错误）SHALL 视为网关未提供模型列表，跳过该步并继续第 ② 步，此时未被列出的模型为空。第 ② 步（图片端点）返回 400 时 SHALL 判定图片端点连通且鉴权通过；返回 401 或 403 时 SHALL 判定 API Key 无效或无权限；返回 404 时 SHALL 判定路径不对并提示 `base_url` 只填到主机根路径；其他情况 SHALL 判定失败并给出状态码与脱敏后的响应摘要。请求的模型清单为空时，第 ② 步 SHALL 使用通用占位模型 id，且对 404 的说明 SHALL 区分「模型 id 未指定」与「路径不对」。

#### Scenario: 网关未提供模型列表时改用图片端点验证

- **WHEN** 网关的 `/v1/models` 返回 404、405、5xx 或请求出现网络错误，而不含 `prompt` 的 `/v1/images/generations` 请求返回 400
- **THEN** 模型列表一步标记为已跳过，探测结论为连通，未被列出的模型为空

#### Scenario: 图片端点鉴权失败

- **WHEN** 模型列表一步被跳过，`/v1/images/generations` 请求返回 401 或 403
- **THEN** 图片端点一步标记为失败并说明 API Key 无效或无权限，探测结论为不连通

#### Scenario: 图片端点返回其他状态

- **WHEN** `/v1/images/generations` 请求返回 400、401、403、404 之外的状态（如 500）
- **THEN** 图片端点一步标记为失败，说明中包含该状态码与脱敏后的响应摘要

#### Scenario: 模型清单为空时区分路径不对与模型未指定

- **WHEN** 探测请求的模型清单为空，系统以通用占位模型 id 发起第 ② 步且返回 404
- **THEN** 探测结论为不连通，说明中同时指出「模型 id 未指定」与「路径不对」两种可能，而不是直接断言路径不对

### Requirement: 连通性探测的取值来源与安全边界

探测 SHALL 使用请求给出的 `base_url`、`api_key` 与模型清单，即设置界面当前填写的值，无需先保存。请求未提供 `api_key`（缺省或为空）时，系统 SHALL 改用该供应方已保存的 `api_key`；两者皆无时 SHALL 拒绝该请求。探测前系统 SHALL 以与图片生成出站相同的规则校验 `base_url`（仅允许 HTTPS 或本地回环），不合规时 SHALL 拒绝该请求。被拒绝的请求 SHALL NOT 发起任何出站请求。探测请求 SHALL 只发往被探测的 `base_url`，SHALL 禁用自动重定向，SHALL NOT 携带另一供应方的 `api_key`。探测 SHALL NOT 改动已持久化的配置，SHALL NOT 写入会话记录或图片文件。探测结果、错误与日志 SHALL NOT 含 `api_key` 明文。

#### Scenario: 用界面当前填写的值探测

- **WHEN** 用户在设置界面修改了某个供应方的 `base_url` 与 `api_key` 但尚未保存，随即发起连通性探测
- **THEN** 探测请求发往界面上当前填写的 `base_url`，鉴权使用当前填写的 `api_key`，而非已保存的值

#### Scenario: 未重新输入 key 时用已保存的 key 探测

- **WHEN** 某个供应方已保存过 `api_key`，探测请求的 `api_key` 为空
- **THEN** 系统使用该供应方已保存的 `api_key` 完成探测

#### Scenario: 既未填写也未保存过 key 时拒绝探测

- **WHEN** 探测请求的 `api_key` 为空，且该供应方没有已保存的 `api_key`
- **THEN** 系统拒绝该请求并提示未配置 API Key、无法测试连通性，不发起任何出站请求

#### Scenario: base_url 不合规时拒绝探测

- **WHEN** 探测请求的 `base_url` 不是 HTTPS（且非本地回环）或不是合法 URL
- **THEN** 系统拒绝该请求并提示原因，不发起任何出站请求

#### Scenario: 探测结果不含 key 明文

- **WHEN** 网关的错误响应体中包含本次探测使用的 `api_key` 明文
- **THEN** 探测结果的结论与各步说明中该明文已被替换

#### Scenario: 探测不改动已保存的配置

- **WHEN** 一次连通性探测结束（无论结论是否连通）
- **THEN** 已持久化的配置与探测前相同，会话记录与图片目录均无新增

### Requirement: 连通性探测的界面交互

设置界面 SHALL 在每个供应方分区的 Base URL 附近提供「测试连通」入口，SHALL 以该分区当前填写的值发起探测；用户未重新输入 `api_key` 时，前端 SHALL 提交空的 `api_key`，SHALL NOT 提交脱敏值。探测结果 SHALL 在该供应方分区内联展示：结论、每一步的说明，以及单独高亮的未被网关列出的模型。探测进行期间该供应方的入口 SHALL 显示进行中状态并禁用。探测失败 SHALL NOT 阻止用户保存设置。

#### Scenario: 内联展示探测结果

- **WHEN** 某个供应方的连通性探测返回，且有模型未被网关列出
- **THEN** 该供应方分区内展示结论与每一步的说明，并以单独一行高亮列出未被网关列出的模型 id

#### Scenario: 探测进行中入口禁用

- **WHEN** 某个供应方的连通性探测尚未返回
- **THEN** 该供应方的「测试连通」按钮显示进行中状态并禁用，另一供应方的按钮不受影响

#### Scenario: 探测失败不阻断保存

- **WHEN** 某个供应方的连通性探测结论为不连通
- **THEN** 用户仍可保存当前填写的设置
