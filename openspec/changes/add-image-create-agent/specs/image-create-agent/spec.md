## Purpose

独立顶层的图片创作 Agent：用户通过多轮对话迭代图片 prompt，确认后由 Aria 后端调用 gpt-image-2 网关生成图片并展示，支持模板、参数化生成、参考图改图与网关配置。

## ADDED Requirements

### Requirement: 独立顶层图片创作入口与会话

系统 SHALL 提供与 `/workbench` 平级的独立顶层入口 `/image-create`，其存在与功能 SHALL 不依赖任何 Project、Issue 或 git 仓库。系统 SHALL 支持创建多个相互独立的图片创作会话，每个会话独立维护其对话历史、当前 prompt 与生成结果。

#### Scenario: 用户无需项目即可开始创作

- **WHEN** 用户在未创建任何 Project/Issue 的情况下进入 `/image-create`
- **THEN** 系统允许创建并使用图片创作会话，不提示缺少 Project/Issue/git 仓库

#### Scenario: 多会话相互独立

- **WHEN** 用户创建并切换于多个图片创作会话
- **THEN** 每个会话各自保留独立的对话历史、prompt 区块与生成结果，互不串扰

### Requirement: 多轮 prompt 迭代由现有 CLI 执行器驱动

系统 SHALL 复用现有 CLI 执行器（Claude Code / Codex 等所有受支持执行器）承载图片 prompt 迭代对话。系统 SHALL 通过执行器原生 session 续接机制（`resume_provider_session_id`）实现同一会话的多轮上下文连续。每轮迭代中，执行器 SHALL 把「建议的最终 prompt」作为结构化产出，供前端呈现为可编辑区块。

#### Scenario: 多轮续接保留上下文

- **WHEN** 用户在同一会话中发起第二轮 prompt 迭代
- **THEN** 系统复用上一轮的执行器原生 session，使执行器记得前文并基于已有上下文更新建议的 prompt

#### Scenario: 建议 prompt 可被用户编辑

- **WHEN** 执行器产出一轮「建议的最终 prompt」
- **THEN** 系统将其呈现为可编辑区块，用户可在生成图片前修改其内容

### Requirement: prompt 模板机制

系统 SHALL 提供预置模板，并允许用户选择模板或自定义引导词。预置集合 SHALL 至少包含「PPT 商务配图」与「业务流程图」两套模板。所选模板的引导词 SHALL 注入该会话的 prompt 迭代对话以约束生成方向。

#### Scenario: 选择预置模板

- **WHEN** 用户创建会话时选择「业务流程图」模板
- **THEN** 该会话后续 prompt 迭代注入「业务流程图」模板引导词，使建议 prompt 倾向于流程图风格

#### Scenario: 自定义模板

- **WHEN** 用户选择「自定义」并填写一段引导词创建会话
- **THEN** 系统将该自定义引导词注入 prompt 迭代对话，行为与预置模板一致

### Requirement: 图片生成由用户显式触发且参数可配置

系统 SHALL 仅在用户显式触发（如点击生成按钮）时发起图片生成，不得由执行器自主触发。生成请求 SHALL 携带用户可配置的参数：`size`、`quality`、`background`、`output_format`、`n`。系统 SHALL 仅接受预定义的枚举值作为这些参数：`size` ∈ {1024x1024, 1536x1024, 1024x1536, auto}；`quality` ∈ {low, medium, high, auto}；`background` ∈ {transparent, opaque, auto}；`output_format` ∈ {png, jpeg, webp}。

#### Scenario: 用户点按钮触发生成

- **WHEN** 用户确认 prompt 并点击「生成」按钮提交选定参数
- **THEN** 系统向后端发起图片生成请求，使用所提交的 prompt 与参数调用 image2 网关

#### Scenario: 拒绝越界参数值

- **WHEN** 前端试图提交不在预定义枚举内的参数值（如 `quality=ultra`）
- **THEN** 系统不发起该生成请求并提示参数非法

### Requirement: 参考图改图自动选择端点

系统 SHALL 支持用户在生成时可选地提供单张参考图。系统 SHALL 根据是否提供参考图自动选择 image2 网关端点：无参考图时调用 `/v1/images/generations`（文生图）；有参考图时调用 `/v1/images/edits`（参考图改图，将参考图作为图片输入与 prompt 一并提交）。该端点选择对用户透明。

#### Scenario: 无参考图走文生图

- **WHEN** 用户在无参考图的情况下触发生成
- **THEN** 系统调用 `/v1/images/generations` 以纯 prompt 生成

#### Scenario: 有参考图走改图

- **WHEN** 用户在提供单张参考图的情况下触发生成
- **THEN** 系统调用 `/v1/images/edits`，将参考图与 prompt 一并提交，基于参考图生成

### Requirement: 生成结果以 base64 直接展示

系统 SHALL 将 image2 网关返回的 `b64_json` 直接在前端以图片形式展示，SHALL NOT 要求将图片写入本地磁盘作为展示前提。系统 SHALL 将生成结果纳入该会话历史，供用户回看。

#### Scenario: 直接展示生成图片

- **WHEN** image2 网关返回 `b64_json`
- **THEN** 系统在前端直接以 base64 数据 URI 渲染图片，无需先落盘本地

#### Scenario: 结果纳入会话历史

- **WHEN** 一次图片生成完成
- **THEN** 该结果与对应 prompt 一并记录进会话历史，用户可在会话中回看

### Requirement: 不满意时可基于反馈继续迭代

系统 SHALL 允许用户在看到生成结果后继续在会话中输入反馈，回到 prompt 迭代阶段由执行器更新建议 prompt，再次由用户显式触发生成。

#### Scenario: 基于不满意反馈重新迭代

- **WHEN** 用户对生成结果不满意并在会话中输入修改诉求
- **THEN** 系统复用同一执行器 session 继续 prompt 迭代、更新建议 prompt 区块，供用户再次确认并触发生成

### Requirement: 网关配置录入与脱敏存储

系统 SHALL 提供设置界面供用户录入 image2 网关的 `base_url` 与 `api_key` 及默认参数。系统 SHALL 将配置持久化于 Aria 后端 `.aria` 目录下。系统 SHALL 在前端展示 `api_key` 时对其脱敏（不展示明文），且前端 SHALL NOT 缓存 `api_key` 明文。

#### Scenario: 录入并持久化配置

- **WHEN** 用户在设置界面录入 `base_url`、`api_key` 与默认参数并保存
- **THEN** 系统将配置持久化于后端 `.aria` 目录，后续生成请求使用该配置

#### Scenario: 前端展示脱敏

- **WHEN** 配置已保存后用户再次打开设置界面
- **THEN** `api_key` 以脱敏形式展示（如 `sk-****1234`），不显示完整明文

### Requirement: API Key 安全边界

系统 SHALL 仅在 Aria 后端读取与使用 image2 `api_key`（用于发起 image2 请求的鉴权头）。系统 SHALL NOT 将 `api_key` 传递给 prompt 迭代所用的 CLI 执行器子进程。

#### Scenario: 执行器子进程不接触 key

- **WHEN** 系统发起一轮 prompt 迭代（启动/续接 CLI 执行器子进程）
- **THEN** 该子进程不接收 image2 `api_key`，key 仅在后端用于图片生成请求

#### Scenario: 缺少有效配置时拒绝生成

- **WHEN** 用户在未录入有效 `base_url` 或 `api_key` 时触发生成
- **THEN** 系统不发起 image2 请求并提示用户先完成网关配置
