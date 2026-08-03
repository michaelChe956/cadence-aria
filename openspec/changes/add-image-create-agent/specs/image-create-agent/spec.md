## Purpose

独立顶层的图片创作 Agent：用户通过多轮对话迭代图片 prompt，确认后由 Aria 后端调用 gpt-image-2 网关生成单张图片并展示，支持模板、参数化生成、参考图改图与网关配置，并约束付费请求与出站安全边界。

## ADDED Requirements

### Requirement: 独立顶层图片创作入口与会话

系统 SHALL 提供与 `/workbench` 平级的独立顶层入口 `/image-create`，其存在与功能 SHALL 不依赖任何 Project、Issue 或 git 仓库。系统 SHALL 支持创建多个相互独立的图片创作会话，每个会话独立维护其对话历史、当前建议 prompt 与生成结果。

#### Scenario: 用户无需项目即可开始创作

- **WHEN** 用户在未创建任何 Project/Issue 的情况下进入 `/image-create`
- **THEN** 系统允许创建并使用图片创作会话，不提示缺少 Project/Issue/git 仓库

#### Scenario: 多会话相互独立

- **WHEN** 用户创建并切换于多个图片创作会话
- **THEN** 每个会话各自保留独立的对话历史、建议 prompt 与生成结果，互不串扰

### Requirement: 会话生命周期与资源清理

系统 SHALL 为每个会话维护独立的执行器 scratch 目录。系统 SHALL 在删除会话时取消该会话任何进行中的执行器 run 与图片生成请求，阻止该会话的新请求，删除其 scratch 目录与持久化记录。会话删除 SHALL 失败时不静默吞错，SHALL 向用户报告失败。

#### Scenario: 删除空闲会话清理资源

- **WHEN** 用户删除一个无进行中操作的会话
- **THEN** 系统删除其 scratch 目录与持久化记录，且其不再出现在会话列表

#### Scenario: 删除运行中的会话先终止操作

- **WHEN** 用户删除一个存在进行中 prompt 迭代或图片生成的会话
- **THEN** 系统先取消这些进行中操作，再执行删除与资源清理

#### Scenario: 会话不存在的请求

- **WHEN** 客户端对已删除或不存在的会话发起请求
- **THEN** 系统返回明确「会话不存在」结果，不创建副作用

### Requirement: 多轮 prompt 迭代由现有 CLI 执行器驱动

系统 SHALL 复用现有 CLI 执行器（Claude Code / Codex 等所有受支持执行器）承载图片 prompt 迭代对话。系统 SHALL 通过执行器原生 session 续接机制传递上一轮的 provider session 标识，使后续轮次在该 session 上继续。每轮迭代中，系统 SHALL 要求执行器按约定结构化产出「建议的最终 prompt」。

#### Scenario: 后续轮次携带上一轮 session 标识

- **WHEN** 用户在同一会话中发起第二轮 prompt 迭代
- **THEN** 系统向执行器提交的请求携带第一轮返回的 provider session 标识

#### Scenario: 注入的 prompt 包含模板引导词

- **WHEN** 用户选择了某个模板（含自定义引导词）并发起 prompt 迭代
- **THEN** 系统提交给执行器的 prompt 包含该模板引导词

#### Scenario: 建议 prompt 可被用户编辑

- **WHEN** 执行器按结构化约定产出一轮「建议的最终 prompt」
- **THEN** 系统将其呈现为可编辑区块，用户可在生成图片前修改其内容

### Requirement: 结构化建议 prompt 的约定与降级

系统 SHALL 与执行器约定结构化产出包含一个非空的「建议 prompt」文本字段。当执行器的产出无法解析为该结构（缺失字段、空值、非法结构）时，系统 SHALL NOT 把空 prompt 作为建议展示，SHALL 向用户提示本轮未产出可用 prompt 并保留上一轮可编辑的建议 prompt（若存在）。

#### Scenario: 结构化解析失败时保留上一轮 prompt

- **WHEN** 某轮执行器产出无法解析出非空建议 prompt
- **THEN** 系统不更新建议 prompt 区块为空，提示本轮未产出可用 prompt，并保留上一轮建议 prompt 供用户继续编辑

### Requirement: 执行器 session 续接失败的降级

系统 SHALL 在尝试续接执行器 session 失败（session 已失效或不被接受）时，SHALL NOT 静默丢弃上下文，SHALL 以新 session 重新发起本轮迭代，并在新会话首轮请求中回灌该会话已有的上下文（模板引导词、历史用户输入与建议 prompt）。系统 SHALL 仅对「续接失败」这一可识别情形降级，正常续接 SHALL 复用原 session。

#### Scenario: 续接失败后以新 session 回灌上下文

- **WHEN** 续接执行器 session 失败
- **THEN** 系统以新 session 重新发起本轮迭代，且新 session 首轮请求包含已有上下文（模板引导词、历史输入与上一轮建议 prompt）

#### Scenario: 正常续接复用原 session

- **WHEN** 续接执行器 session 成功
- **THEN** 系统复用原 session，不新建 session、不重复回灌上下文

### Requirement: prompt 模板机制（一次性引导词）

系统 SHALL 提供预置模板，并允许用户在创建会话时选择一个预置模板或填写一段自定义引导词。预置集合 SHALL 至少包含「PPT 商务配图」与「业务流程图」两套模板。所选模板或自定义引导词 SHALL 作为该会话 prompt 迭代的注入引导词。本变更 SHALL NOT 提供模板的持久化保存、列表、编辑或删除（自定义引导词为一次性会话引导词）。

#### Scenario: 选择预置模板

- **WHEN** 用户创建会话时选择「业务流程图」模板
- **THEN** 系统将该模板引导词作为该会话 prompt 迭代的注入引导词

#### Scenario: 自定义一次性引导词

- **WHEN** 用户创建会话时选择「自定义」并填写一段引导词
- **THEN** 系统将该引导词作为该会话 prompt 迭代的注入引导词，且不持久化为可复用模板

### Requirement: 单会话操作并发约束

系统 SHALL 限制每个会话在任一时刻最多存在一个进行中的后端操作（prompt 迭代或图片生成）。当该会话已有进行中操作时，系统 SHALL 拒绝新的后端操作请求并向用户明确提示忙碌，SHALL NOT 自动排队或自动取消已有操作。

#### Scenario: 已有进行中操作时拒绝新操作

- **WHEN** 某会话已有一个进行中的后端操作，用户再发起新的后端操作请求
- **THEN** 系统拒绝新请求并提示该会话忙碌，不创建新操作

### Requirement: 图片生成由用户显式触发且参数可配置

系统 SHALL 仅在用户显式触发（如点击生成按钮）时发起图片生成，不得由执行器自主触发。生成请求 SHALL 携带用户可配置的参数：`size`、`quality`、`background`、`output_format`、`input_fidelity`（仅参考图改图时）。系统 SHALL 仅接受预定义枚举值：`size` ∈ {1024x1024, 1536x1024, 1024x1536, auto}；`quality` ∈ {low, medium, high, auto}；`background` ∈ {transparent, opaque, auto}；`output_format` ∈ {png, jpeg, webp}；`input_fidelity` ∈ {low, high}。本变更 SHALL 每次生成请求恰好生成一张图片（不支持多图批量）。

#### Scenario: 用户点按钮触发生成

- **WHEN** 用户确认 prompt 并点击「生成」按钮提交选定参数
- **THEN** 系统向后端发起图片生成请求，使用所提交的 prompt 与参数调用 image2 网关，请求恰好生成一张图片

#### Scenario: 拒绝越界参数值

- **WHEN** 前端试图提交不在预定义枚举内的参数值（如 `quality=ultra`）
- **THEN** 系统不发起该生成请求并提示参数非法

#### Scenario: 文生图时忽略 input_fidelity

- **WHEN** 用户在无参考图（文生图）情况下提交了 `input_fidelity`
- **THEN** 系统不在文生图请求中发送 `input_fidelity`

### Requirement: 参考图改图自动选择端点

系统 SHALL 支持用户在生成时可选地提供单张参考图。系统 SHALL 根据是否提供参考图自动选择 image2 网关端点：无参考图时调用 `/v1/images/generations`（文生图）；有参考图时调用 `/v1/images/edits`（参考图改图，将参考图与 prompt 一并提交）。该端点选择对用户透明。

#### Scenario: 无参考图走文生图

- **WHEN** 用户在无参考图的情况下触发生成
- **THEN** 系统调用 `/v1/images/generations` 以纯 prompt 生成

#### Scenario: 有参考图走改图

- **WHEN** 用户在提供单张参考图的情况下触发生成
- **THEN** 系统调用 `/v1/images/edits`，将参考图与 prompt 一并提交，基于参考图生成

### Requirement: 参考图输入约束

系统 SHALL 对参考图施加约束：单张；限定允许的图像 MIME 类型（png/jpeg/webp）；限定最大字节数与图像尺寸上限。系统 SHALL 拒绝不符合约束的参考图并给出明确原因。参考图 SHALL 仅在本次生成请求生命周期内使用，SHALL NOT 在生成完成后持久化保留。

#### Scenario: 拒绝超大或格式不符的参考图

- **WHEN** 用户上传超过最大字节数或不在允许 MIME 内的参考图
- **THEN** 系统拒绝该参考图并提示具体原因，不发起生成

#### Scenario: 参考图不被持久化

- **WHEN** 一次带参考图的生成完成
- **THEN** 系统不在会话历史或磁盘上持久化保留该参考图文件

### Requirement: 生成结果按媒体类型直接展示

系统 SHALL 将 image2 网关返回的单张 `b64_json` 按其 `output_format` 对应的媒体类型构造数据 URI 在前端直接展示，SHALL NOT 固定为某一种图片格式，SHALL NOT 要求将图片写入本地磁盘作为展示前提。系统 SHALL 将生成结果（含所用 prompt 与参数）纳入该会话历史供回看。

#### Scenario: 按输出格式构造数据 URI

- **WHEN** image2 网关返回 `b64_json` 且本次 `output_format` 为 webp
- **THEN** 系统以 webp 媒体类型构造数据 URI 渲染图片

#### Scenario: 结果纳入会话历史

- **WHEN** 一次图片生成完成
- **THEN** 该结果与对应 prompt、参数一并记录进会话历史，用户可在会话中回看

### Requirement: 不满意时可基于反馈继续迭代

系统 SHALL 允许用户在看到生成结果后继续在会话中输入反馈，回到 prompt 迭代阶段更新建议 prompt，再次由用户显式触发生成。

#### Scenario: 基于不满意反馈重新迭代

- **WHEN** 用户对生成结果不满意并在会话中输入修改诉求
- **THEN** 系统回到 prompt 迭代阶段更新建议 prompt 区块，供用户再次确认并触发生成

### Requirement: 图片生成失败与重试边界

系统 SHALL 对 image2 网关的连接失败、超时、4xx、5xx、空 `data`、缺 `b64_json` 等错误进行归一并向用户展示可读错误。系统 SHALL NOT 对已发出的图片生成请求自动重试（防止重复计费）；用户可显式重新触发生成。系统 SHALL NOT 在生成失败时写入成功结果，SHALL 将失败事件（含可读错误，不含原始敏感鉴权信息）记录进会话历史。

#### Scenario: 网关错误不自动重试

- **WHEN** 图片生成请求返回错误（如超时、429、5xx 或空 data）
- **THEN** 系统不自动重试该请求，向用户展示可读错误，并允许用户显式重新触发

#### Scenario: 失败不写入成功结果

- **WHEN** 一次图片生成失败
- **THEN** 系统不在会话历史写入成功生成结果，仅记录一条失败事件

### Requirement: 网关配置录入与脱敏存储

系统 SHALL 提供设置界面供用户录入 image2 网关的 `base_url` 与 `api_key` 及默认参数。系统 SHALL 将配置持久化于 Aria 后端 `.aria` 目录下。系统 SHALL 在前端展示 `api_key` 时对其脱敏（不展示明文），且前端 SHALL NOT 缓存 `api_key` 明文。设置更新 SHALL 采用保留语义：当用户未提供新 `api_key`（值为空或为脱敏占位）时，系统 SHALL 保留原 `api_key` 而非清空；清除 `api_key` SHALL 需要用户显式清除动作。

#### Scenario: 录入并持久化配置

- **WHEN** 用户在设置界面录入 `base_url`、`api_key` 与默认参数并保存
- **THEN** 系统将配置持久化于后端 `.aria` 目录，后续生成请求使用该配置

#### Scenario: 前端展示脱敏

- **WHEN** 配置已保存后用户再次打开设置界面
- **THEN** `api_key` 以脱敏形式展示（如 `sk-****1234`），不显示完整明文

#### Scenario: 仅改默认参数不清空 key

- **WHEN** 用户在未重新输入 `api_key`（展示为脱敏占位）的情况下修改默认参数并保存
- **THEN** 系统保留原 `api_key`，仅更新默认参数

### Requirement: API Key 安全边界与出站目标约束

系统 SHALL 仅在 Aria 后端读取与使用 image2 `api_key`（用于发起 image2 请求的鉴权头）。系统 SHALL NOT 将 `api_key` 传递给 prompt 迭代所用的 CLI 执行器子进程。系统 SHALL 对用户录入的 `base_url` 施加出站安全约束：仅允许 HTTPS（或本地回环）；禁止在跨域 HTTP 重定向时携带 Authorization 头。系统 SHALL NOT 在错误日志、诊断输出或测试夹具中明文记录 `api_key`。

#### Scenario: 执行器子进程不接触 key

- **WHEN** 系统发起一轮 prompt 迭代（启动/续接 CLI 执行器子进程）
- **THEN** 该子进程不接收 image2 `api_key`，key 仅在后端用于图片生成请求

#### Scenario: 缺少有效配置时拒绝生成

- **WHEN** 用户在未录入有效 `base_url` 或 `api_key` 时触发生成
- **THEN** 系统不发起 image2 请求并提示用户先完成网关配置

#### Scenario: 拒绝非 HTTPS 的 base_url

- **WHEN** 用户录入的 `base_url` 不是 HTTPS（且非本地回环）
- **THEN** 系统拒绝该 `base_url` 并提示必须使用 HTTPS

#### Scenario: 跨域重定向不携带鉴权

- **WHEN** image2 请求被网关重定向到与原 host 不同的目标
- **THEN** 系统不在该重定向请求中携带 Authorization 头
