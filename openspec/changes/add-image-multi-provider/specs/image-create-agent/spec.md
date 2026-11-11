## MODIFIED Requirements

### Requirement: 图片生成由用户显式触发且参数可配置

系统 SHALL 仅在用户显式触发（如点击生成按钮）时发起图片生成，不得由执行器自主触发。生成请求 SHALL 携带用户可配置的参数：`size`、`quality`、`background`、`output_format`、`model`、`use_previous`。系统 SHALL 仅接受预定义取值：`size` ∈ {1024x1024, 1536x1024, 1024x1536, auto}；`quality` ∈ {low, medium, high, auto}；`background` ∈ {transparent, opaque, auto}；`output_format` ∈ {png, jpeg, webp}；`model` 为 `auto` 或已配置模型清单中的某个模型 id，缺省视为 `auto`；`use_previous` ∈ {true, false}，缺省视为 `false`。系统 SHALL NOT 提供 `input_fidelity` 参数，SHALL NOT 在任何出站请求中发送 `input_fidelity`。系统 SHALL 每次生成请求恰好生成一张图片（不支持多图批量）。

#### Scenario: 用户点按钮触发生成

- **WHEN** 用户确认 prompt 并点击「生成」按钮提交选定参数
- **THEN** 系统向后端发起图片生成请求，使用所提交的 prompt 与参数调用本次选定供应方的图片端点，请求恰好生成一张图片

#### Scenario: 拒绝越界参数值

- **WHEN** 前端试图提交不在预定义枚举内的参数值（如 `quality=ultra`）
- **THEN** 系统不发起该生成请求并提示参数非法

#### Scenario: 拒绝非法的 use_previous 取值

- **WHEN** 生成请求的 `use_previous` 不是 `true` 或 `false`（如 `yes`）
- **THEN** 系统不发起该生成请求并提示参数非法

#### Scenario: 缺省 model 与 use_previous 时保持既有行为

- **WHEN** 生成请求未携带 `model` 与 `use_previous`
- **THEN** 系统按 `model=auto`、`use_previous=false` 处理，不携带上一张生成图

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

- **WHEN** 自动轮转选中的供应方返回错误
- **THEN** 系统不向另一供应方发起请求，本次生成只产生过一次出站调用

#### Scenario: 响应只有 URL 时判定失败

- **WHEN** 图片端点返回 HTTP 200，且 `data[]` 的项只含 `url` 而无 `b64_json`
- **THEN** 系统判定本次生成失败并提示端点未返回图片数据，不向该 URL 发起任何请求

### Requirement: 网关配置录入与脱敏存储

系统 SHALL 提供设置界面供用户分别为两个供应方（GPT、MiniMax）录入各自的 `base_url`、`api_key`、模型清单与轮转模型，以及两家共享的默认参数。系统 SHALL 将配置持久化于 Aria 后端 `.aria` 目录下。系统 SHALL 在前端展示每个 `api_key` 时对其脱敏（不展示明文），且前端 SHALL NOT 缓存 `api_key` 明文。设置更新 SHALL 按供应方独立采用保留语义：某个供应方未出现在更新请求中时，系统 SHALL 保留该供应方的全部配置；某个供应方出现但未提供新 `api_key`（保留动作）时，系统 SHALL 保留其原 `api_key` 而非清空；清除某个供应方的 `api_key` SHALL 需要用户针对该供应方的显式清除动作。对一个供应方的更新 SHALL NOT 改变另一个供应方的配置。任一供应方的配置校验失败时，系统 SHALL 拒绝整次更新且不改动已持久化的配置。

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
- **THEN** GPT 的 `base_url`、`api_key`、模型清单与轮转模型均保持不变

#### Scenario: 清除一个供应方的 key

- **WHEN** 用户对 GPT 执行显式清除 `api_key` 的动作并保存
- **THEN** GPT 的 `api_key` 被清空，MiniMax 的 `api_key` 保持不变

#### Scenario: 校验失败时整次更新不落盘

- **WHEN** 更新请求中一个供应方的配置合法、另一个供应方的 `base_url` 不合法
- **THEN** 系统拒绝该更新并提示原因，已持久化的两家配置均未改变

### Requirement: API Key 安全边界与出站目标约束

系统 SHALL 仅在 Aria 后端读取与使用各供应方的 `api_key`（用于发起图片请求的鉴权头）。系统 SHALL NOT 将任何供应方的 `api_key` 传递给 prompt 迭代所用的 CLI 执行器子进程。每个供应方的 `api_key` SHALL 仅随发往该供应方自身 `base_url` 的请求发送，SHALL NOT 出现在发往另一供应方的请求中。系统 SHALL 对用户录入的每个供应方的 `base_url` 施加出站安全约束：仅允许 HTTPS（或本地回环），在保存设置时与每次出站前均 SHALL 校验。系统 SHALL 默认禁用图片请求的自动重定向；如需允许重定向，SHALL 仅允许目标与原请求为同一 origin（scheme、host、port 三者全同），SHALL 拒绝任何 HTTPS→HTTP 的降级重定向，且 SHALL NOT 在任何跨 origin 或降级重定向中携带 Authorization 头。系统 SHALL NOT 向图片端点响应体中给出的任何 URL 发起请求。系统 SHALL NOT 在错误日志、诊断输出或测试夹具中明文记录任何 `api_key`。

#### Scenario: 执行器子进程不接触 key

- **WHEN** 系统发起一轮 prompt 迭代（启动/续接 CLI 执行器子进程）
- **THEN** 该子进程不接收任何供应方的 `api_key`，key 仅在后端用于图片生成请求

#### Scenario: 缺少有效配置时拒绝生成

- **WHEN** 用户显式选择的模型所属供应方未录入 `base_url` 或 `api_key`，或在自动轮转下没有任何可用的供应方
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

系统 SHALL 为每个供应方维护一份用户可配置的模型 id 清单与一个轮转模型（自动轮转选中该供应方时使用的模型），并 SHALL 内置推荐默认值；用户 SHALL 可在设置中增删清单项并更改轮转模型。保存时系统 SHALL 校验清单：每个 id 长度为 1–128 且仅含字母、数字与 `. _ : / -`；id 不得为保留值 `auto`（不区分大小写）；同一供应方内不重复、两个供应方之间不重复；每个供应方至多 32 个；清单非空时轮转模型必须在清单内。校验不通过时系统 SHALL 拒绝保存并给出原因。参数面板 SHALL 提供模型选择，选项为「自动轮转」与已配置凭据的供应方的清单项。

#### Scenario: 内置推荐清单包含 8 个 gpt 模型与 1 个 MiniMax 模型

- **WHEN** 用户从未修改过模型清单并打开设置
- **THEN** GPT 清单恰为 `gpt-image-2.6`、`gpt-image-2.6-flare`、`gpt-image-2.6-flare-2k`、`gpt-image-2.6-flare-4k`、`gpt-image-2.6-sunburst`、`gpt-image-2.6-sunburst-2k`、`gpt-image-2.6-sunburst-4k`、`gpt-image-2.5`，其轮转模型为 `gpt-image-2.6-flare`；MiniMax 清单恰为 `MiniMax-H3-Image`，其轮转模型为 `MiniMax-H3-Image`

#### Scenario: 用户新增的模型可被选择

- **WHEN** 用户在 GPT 清单中新增一个 id 并保存
- **THEN** 该 id 出现在参数面板的模型选项中，选择它生成时其值原样作为 `model` 发往 GPT 的 `base_url`

#### Scenario: 不合法的清单被拒绝

- **WHEN** 用户保存的清单含重复项、含 `auto`、同一 id 同时出现在两个供应方的清单中、或轮转模型不在清单内
- **THEN** 系统拒绝保存并提示具体原因，已持久化的配置未改变

#### Scenario: 参数面板只列出已配置凭据的供应方的模型

- **WHEN** 仅 GPT 配置了 `base_url` 与 `api_key`
- **THEN** 参数面板的模型选项为「自动轮转」与 GPT 清单中的模型，并提示 MiniMax 尚未配置

### Requirement: 模型 id 原样透传且供应方由清单归属决定

一个模型所属的供应方 SHALL 仅由它被配置在哪个供应方的清单中决定；系统 SHALL NOT 依据模型 id 的文本内容或 `base_url` 的主机名推断供应方或端点身份。生成时显式指定的模型 SHALL 与某个清单项逐字符相等，否则系统 SHALL 拒绝该请求且不发起任何出站请求。系统 SHALL 将所选模型 id 原样写入出站请求的 `model` 字段，SHALL NOT 依据模型 id 的文本内容改变任何其他请求字段，SHALL NOT 为表达分辨率档位而发送供应方官方契约之外的字段。

#### Scenario: 显式选择的模型原样出现在出站请求中

- **WHEN** 用户在参数面板选择 `gpt-image-2.6-sunburst-2k` 并触发生成
- **THEN** 出站请求发往 GPT 的 `base_url`，其 `model` 字段的值恰为 `gpt-image-2.6-sunburst-2k`

#### Scenario: 带分辨率后缀的模型不改变其他请求字段

- **WHEN** 在其余参数完全相同的情况下，分别以 `gpt-image-2.6-flare` 与 `gpt-image-2.6-flare-4k` 触发生成
- **THEN** 两次出站请求体除 `model` 的值之外完全相同，且都不含 `resolution`、`area` 等用于表达分辨率档位的额外字段

#### Scenario: 清单外的模型被拒绝且不出站

- **WHEN** 生成请求的 `model` 既不是 `auto` 也不与任何清单项逐字符相等（包括仅大小写不同）
- **THEN** 系统返回参数错误并指出该模型不在已配置的模型清单中，不发起任何出站请求

#### Scenario: 供应方由清单归属决定

- **WHEN** 用户把一个任意命名的 id 配置在 MiniMax 清单中并显式选择它生成
- **THEN** 系统按 MiniMax 的请求规则构造请求并发往 MiniMax 的 `base_url`

### Requirement: 双供应方自动轮转

当生成请求的 `model` 为 `auto` 时，系统 SHALL 按如下规则选定供应方：会话尚无成功的生成结果时首选 GPT；否则首选会话最后一条成功生成结果所属供应方的另一家，其中未记录供应方的旧结果视为 GPT。首选供应方可用（已配置 `base_url`、`api_key` 且模型清单非空）时使用它；不可用而另一家可用时使用另一家；两家都不可用时系统 SHALL 拒绝该请求且不发起出站请求。被选定供应方使用其轮转模型。轮转判定 SHALL 只依据会话已持久化的成功生成结果序列：失败的生成 SHALL NOT 改变下一次的选择。

当 `model` 为显式模型 id 时，系统 SHALL 使用该模型及其所属供应方，SHALL NOT 改用另一供应方；该供应方未配置 `base_url` 或 `api_key` 时系统 SHALL 拒绝该请求且不发起出站请求。显式选择产生的成功结果同样进入序列，参与此后的自动轮转判定。

#### Scenario: 首次自动生成使用 GPT

- **WHEN** 两个供应方均可用，会话尚无生成结果，用户以 `model=auto` 触发生成
- **THEN** 请求发往 GPT，使用 GPT 的轮转模型

#### Scenario: 连续自动生成依次为 GPT、MiniMax、GPT

- **WHEN** 两个供应方均可用，用户在同一会话中以 `model=auto` 连续成功生成三次
- **THEN** 三次请求依次发往 GPT、MiniMax、GPT

#### Scenario: 失败不消耗轮次

- **WHEN** 第一次自动生成（GPT）成功，第二次自动生成（MiniMax）失败，用户再次以 `model=auto` 触发生成
- **THEN** 第三次请求仍发往 MiniMax

#### Scenario: 显式选择之后的自动生成选另一家

- **WHEN** 会话最后一条成功结果是用户显式选择 GPT 模型生成的，用户随后以 `model=auto` 触发生成
- **THEN** 请求发往 MiniMax

#### Scenario: 旧记录视为 GPT

- **WHEN** 会话最后一条成功结果是升级前产生的、未记录供应方的旧结果，用户以 `model=auto` 触发生成
- **THEN** 请求发往 MiniMax

#### Scenario: 仅一家可用时自动轮转恒用该家

- **WHEN** 仅 GPT 可用，用户在同一会话中以 `model=auto` 连续成功生成两次
- **THEN** 两次请求都发往 GPT，均不报配置缺失

#### Scenario: 两家都不可用时拒绝

- **WHEN** 两个供应方都未配置，用户以 `model=auto` 触发生成
- **THEN** 系统提示尚未配置任何可用的图片生成端点，不发起出站请求

#### Scenario: 显式选择的供应方未配置时不换家

- **WHEN** 用户显式选择 MiniMax 清单中的模型，而 MiniMax 未配置 `api_key`、GPT 可用
- **THEN** 系统提示 MiniMax 尚未配置，不发起出站请求，也不改用 GPT

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

- **WHEN** 两个供应方均可用，第一次自动生成由 GPT 完成，用户随后以 `model=auto`、`use_previous=true` 触发生成
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

#### Scenario: 默认参数下的轮转与携带链可持续

- **WHEN** 两个供应方均可用，用户保持默认参数（`size=auto`、`model=auto`、`use_previous=true`）在同一会话中连续触发三次生成，且各次请求均成功
- **THEN** 第二次发往 MiniMax 的请求含 `area=1K`，第三次生成能以第二次的生成结果作为参考图发出，不因上一张图超过输入约束而失败

### Requirement: 生成结果记录供应方与模型

系统 SHALL 在每条成功的生成结果中记录：实际使用的供应方、实际发送的模型 id、选择方式（自动轮转或显式选择）、参考图来源（无、用户上传、上一张生成图）。生成接口的成功响应与会话记录读取接口 SHALL 返回这些信息。前端 SHALL 在生成图处展示供应方与模型，并标示自动轮转与参考图来源。对升级前产生、未记录这些信息的旧结果，系统 SHALL 正常读取与展示图片，SHALL NOT 报错，前端 SHALL NOT 为其展示供应方与模型标注。图片端点返回的错误（含写入会话历史的失败事件）SHALL 带有本次所用的供应方与模型，SHALL NOT 含 `api_key` 明文。

#### Scenario: 成功结果记录供应方与模型

- **WHEN** 一次自动轮转选中 MiniMax 并以上一张生成图为参考图的生成成功
- **THEN** 该结果记录供应方为 MiniMax、模型为 MiniMax 的轮转模型、选择方式为自动、参考图来源为上一张生成图，且生成接口响应包含同样的信息

#### Scenario: 前端展示供应方与模型

- **WHEN** 用户查看一条升级后产生的生成图
- **THEN** 该图片处显示其供应方与模型 id；自动轮转选出的带有自动轮转标示，携带了上一张图的带有相应标示

#### Scenario: 重新打开会话后信息仍在

- **WHEN** 用户重新打开一个包含升级后生成结果的会话
- **THEN** 会话记录读取接口返回每条结果的供应方、模型、选择方式与参考图来源，前端据此展示

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

- **WHEN** 仅有旧配置文件的用户升级后以 `model=auto` 触发生成
- **THEN** 请求使用旧文件中的 `base_url` 与 `api_key` 发往 GPT，`model` 为 GPT 的内置轮转模型

#### Scenario: 保存后写为新形态

- **WHEN** 系统读入旧配置文件后用户保存了一次设置
- **THEN** 配置文件为按供应方分组的形态，原 `api_key` 保留在 GPT 供应方下，再次读取得到相同的配置

#### Scenario: 缺少模型清单时补默认值

- **WHEN** 新形态配置文件中某个供应方没有模型清单字段
- **THEN** 系统读入后该供应方的模型清单为内置推荐默认值

#### Scenario: 显式清空的清单保持为空

- **WHEN** 新形态配置文件中某个供应方的模型清单为显式空列表
- **THEN** 系统读入后该清单仍为空，该供应方被视为不可用
