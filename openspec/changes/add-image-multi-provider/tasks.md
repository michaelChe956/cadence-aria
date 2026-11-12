> 技术方案：`cadence/designs/2026-11-11_技术方案_图片生成多模型与双家轮转_v1.0.md`（下称「方案」；文件名沿用立项时的命名，其中的「双家轮转」已按 2026-11-12 的需求变更移除）。类型定义、字段规则表、逐文件改动与用例编号（S / T / C / E / H / F）均以方案为准。
>
> 依赖顺序：1 → 2 → 3 严格串行（后端三个工作包共享 `models.rs` / `engine.rs` / `handlers/image_create.rs`，后一个在前一个合入后开始）；4 → 5 → 6 串行，可在方案 5.5、5.8 的接口契约确认后与 2、3 并行；7 最后。每个工作包先写失败测试再写实现，各自可独立提交。
>
> **需求变更（2026-11-12）**：用户撤回原需求 3（GPT ⇄ MiniMax 自动轮转）——「你就让用户在已配置并可用的provider里选model id就行，轮转不需要」。下文各条中带「已按 2026-11-12 的需求变更移除」标注的内容不再是验收依据，现行行为见第 8 节与 delta spec。
>
> **补充（2026-11-12）**：设置弹窗的模型清单由多行文本框改为复选下拉框、候选项来自连通性探测；单家清单上限由 32 改为 256；参数面板初始选中项改为三级优先规则。下文 5.1、6.1、6.2 等条目中「一行一个」「清单的首个 id」的描述以第 10 节与 delta spec 为准。
>
> **缺陷修复（2026-11-12）**：会话历史改为按时间交织呈现，建议 prompt 块新增可选时间戳，见第 11 节与 delta spec。
>
> **体验修复（2026-11-12）**：prompt 迭代的结构化约定增加「未产出」出口，区分「模型未产出」与「结构化输出失败」并分别提示；只要本轮未产出就给出提示并排到模型说明之前，仅模型明确未产出时把模型说明默认收起，见第 12 节与 delta spec。
>
> **合入前审查修复（2026-11-12）**：连通性探测对已保存 key 的回落收紧为仅当请求的 `base_url` 与已保存的一致；API Key 输入框不再承载脱敏值；重新打开设置界面不重放探测结果；成功响应体上限 64 MiB → 128 MiB。下文 9.1 等条目中「`api_key` 为空时改用已保存的 key」的描述以第 13 节与 delta spec 为准。

## 1. 图片客户端按供应方构造请求与解析响应

- [x] 1.1 新增供应方枚举与内置推荐模型清单常量；删除硬编码模型常量，模型 id 改由调用目标传入并原样写入 `model`（方案 D2、D11、D14、5.1、5.3）；验收：方案用例 C-01、C-03、C-15 通过
- [x] 1.2 客户端入参由整份设置改为单次调用目标（供应方、`base_url`、`api_key`、模型），鉴权与错误脱敏只使用该目标的 key（方案 D12、5.3）；验收：C-11、C-13、C-16 通过
- [x] 1.3 字段过滤集中到单一函数：GPT 发 `quality`/`background`、不发 `response_format`/`area`；MiniMax 不发 `quality`/`background`、发 `response_format=b64_json`，`size=auto` 时改发 `area=1K`；JSON 与 multipart 同一规则（方案 D10、5.4）；验收：C-02、C-02b、C-04、C-05 通过
- [x] 1.4 全后端移除 `input_fidelity`（请求类型、枚举、multipart 解析分支），仍带该字段的请求被接受且字段被忽略（方案 D11）；验收：C-04、C-05、H-09 通过
- [x] 1.5 响应解析逐项检查 `data[]`：取第一个带 `b64_json` 的项；无成功项且有 `error` 项时返回逐项错误；只有 `url` 时判失败且不下载（方案 5.6、D12）；验收：C-06 至 C-10、C-12、C-14 通过
- [x] 1.6 为保持编译，引擎与处理器的调用点以现有单组凭据加内置 GPT 轮转模型构造目标，测试替身随签名调整（「轮转模型」已按 2026-11-12 的需求变更移除，调用目标的模型现为用户所选 model id）；验收：既有后端测试通过

## 2. 设置模型演进为双供应方与可配置模型清单

- [x] 2.1 设置结构改为按供应方分组（每家 `base_url`、`api_key`、模型清单、轮转模型——其中「轮转模型」已按 2026-11-12 的需求变更移除），反序列化兼容旧的单组形态并归入 GPT，序列化只写新形态；配置结构的调试输出不含 key 明文；三个 wire 反序列化结构（`ImageCreateSettingsWire` / `VendorsWire` / `VendorWire`）**不 derive `Debug`**（编译期保证无 `{:?}` 可泄漏 key）（方案 D3、D12.7、5.1）；验收：S-01 至 S-07、S-14 通过
- [x] 2.2 设置更新请求与内部更新意图改为按供应方分组，三态 key 语义按家独立，未出现的供应方整体保留（方案 D4、5.2）；验收：S-08、S-09、H-06 通过
- [x] 2.3 以「合并后整份校验」取代「只校验提交的 base_url」：每家非空 `base_url` 的出站约束 + 模型清单规则（长度与字符集、保留值、家内与跨家去重、数量上限、轮转模型归属——其中「保留值」与「轮转模型归属」两项已按 2026-11-12 的需求变更移除），失败时整次更新不落盘（方案 D1、5.2）；验收：S-10 至 S-12、H-07 通过
- [x] 2.4 脱敏视图按供应方输出，并带出内置推荐清单与可用标记（方案 D4）；验收：S-13、H-05 通过
- [x] 2.5 实现目标解析纯函数（显式 / 自动轮转 / 一家不可用时的规则——其中「自动轮转」分支已按 2026-11-12 的需求变更移除，现只按所选 model id 解析其归属供应方）与模型选择解析；配置缺失错误带具体说明（方案 D5、D6、5.1）；验收：T-01 至 T-12 通过
- [x] 2.6 为保持编译，引擎调用点改读 GPT 供应方配置，处理器的设置更新流程与错误状态映射同步调整；验收：既有后端测试通过

## 3. 生成链路接入模型选择、携带上一张图与结果记录

- [x] 3.1 生成入口改为命令结构（参数、模型选择、是否携带上一张图、上传的参考图）；处理器解析 `model` 与 `use_previous` 并校验取值（方案 5.5）；验收：H-01 至 H-03、H-08 通过
- [x] 3.2 生成流程在出站前依次完成：读取设置 → 取会话最后一条成功结果 → 解析目标 → 确定参考图来源（上传优先，其次上一张图）→ 校验参考图 → 构造只含目标供应方凭据的调用目标（方案 D5 至 D8、5.7）；验收：E-01 至 E-05、E-07 至 E-09、E-12、E-13 通过
- [x] 3.3 新增「读取一条生成结果的图片字节」的单一实现（文件优先，回退内联 `b64`），携带上一张图与图片读取端点共用它；上一张图不可用或未过校验时在出站前失败并给出原因（方案 D7、D8、5.5）；验收：E-06、E-10、E-11、E-15、H-10 通过
- [x] 3.4 成功结果与会话读取 DTO 记录供应方、模型、选择方式、参考图来源；生成接口响应带出这四项（其中「选择方式」已按 2026-11-12 的需求变更移除，现为供应方、模型、参考图来源三项）；客户端错误与失败事件带「供应方 · 模型」前缀（方案 D13）；验收：E-14、E-16、H-04 通过

## 4. 前端类型与 API 层

- [x] 4.1 按方案 5.8 更新类型：供应方、模型选择保留值、选择方式、参考图来源（其中「模型选择保留值」与「选择方式」已按 2026-11-12 的需求变更移除）；参数与生成请求增加 `model`、`use_previous` 并移除 `input_fidelity`；设置读取与更新类型改为按供应方分组；生成响应与会话结果增加四个字段（移除「选择方式」后为三个）；**同时迁移 `image-create-store.ts` 中 `saveSettings()` 的前置 base_url 校验**——新请求类型已无顶层 `base_url`，改为对请求中出现的每家的非空 `base_url` 逐家调 `validateImageCreateBaseUrl`（方案 5.9「状态」）；验收：类型检查通过
- [x] 4.2 API 客户端在生成表单中追加 `model`、`use_previous`，不再追加 `input_fidelity`；验收：F-01、F-02 通过
- [x] 4.3 全前端移除 `input_fidelity`：参数面板的保真度控件、参考图上传时的写入逻辑、状态初值与各测试夹具；设置弹窗暂只读写 GPT 供应方的 base_url 与 key 以保持可用，**共享的「默认参数」分区维持现状读写**（不得因此丢失 defaults 编辑能力）；验收：F-16 与既有前端测试通过

## 5. 前端状态

- [x] 5.1 参数初值为自动轮转且默认携带上一张图；设置加载或保存后，若所选模型已不在任何已配置凭据的供应方清单中则重置为自动轮转（「自动轮转」作为初值与重置目标已按 2026-11-12 的需求变更移除：现为第一个可用供应方清单的首个 id，无可用供应方时为空）（方案 5.9）；验收：F-03、F-05 通过
- [x] 5.2 生成成功后把供应方、模型、选择方式、参考图来源写入生成图条目（其中「选择方式」已按 2026-11-12 的需求变更移除）；从会话记录恢复条目时带出这些可选字段，旧记录不报错（方案 5.9）；验收：F-04、F-06 通过

## 6. 前端组件

- [x] 6.1 参数面板：新增模型下拉（自动轮转 + 已配置凭据的供应方的清单项，未配置的供应方给出提示；其中「自动轮转」选项已按 2026-11-12 的需求变更移除，选项现仅为已配置且可用的供应方的清单项，无可用供应方时生成按钮禁用并提示先配置）；新增「基于上一张图继续修改」开关及其置灰规则；质量与背景在 MiniMax 下的置灰与提示；等待提示去掉具体模型名（方案 5.9）；验收：F-07 至 F-10 通过
- [x] 6.2 设置弹窗：GPT 与 MiniMax 两个分区，各含 Base URL、API Key（三态按家独立）、模型清单（一行一个，可恢复推荐清单）、轮转模型（「轮转模型」已按 2026-11-12 的需求变更移除）；未改动的供应方不进入更新请求；后端校验错误原样展示（方案 5.9）；验收：F-11 至 F-14 通过
- [x] 6.3 对话区：生成图下方展示「供应方 · 模型」及自动轮转、参考图来源标示（「自动轮转」标示已按 2026-11-12 的需求变更移除）；旧记录不展示（方案 5.9）；验收：F-15 通过

## 7. 联调、确认与收尾

- [x] 7.1 全量检查：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`cd web && pnpm tsc -b && pnpm test` 全部通过 —— **已完成（Task 9 执行，结论：四项全绿；`cargo fmt --check` 零差异、`clippy -D warnings` 零告警、`cargo test --locked` lib 6363 passed / 0 failed 且全部集成 target 通过、`pnpm tsc -b` 零错误、`pnpm test` 281 files / 2914 tests 全通过）**
  - 已知事项：既有 flaky 测试 `plan_start_tests::plan_coding_delete_sweeps_residue_without_legacy_repos_json` 在全量并行跑 lib 测试时会因 git worktree 清理 panic，单独重跑通过；与本变更无关（本变更不触及 coding / plan_start 模块）。
  - 已知事项：中间提交区间 `79059838..44788aed`（Task 1 终点 → Task 2 的临时适配提交）之间整树处于编译红状态（`InputFidelity` 枚举删除后的连带编译错误）；验收以分支终点为准，不要用该区间做 bisect 判据。
- [ ] 7.2 真实调用确认方案 8.2 的假设 A1：向 MiniMax 各发一次「不带 `size`、带 `area=1K`」的文生图与改图请求，核对 HTTP 200 与返回图片像素；不成立时停止并回到方案 D10 重新决策，不在实现中自行调整字段规则 —— **未完成（原因：需要真实 MiniMax API Key 与可出图的 endpoint，才能核对 HTTP 200 与返回图片像素；本环境无该凭据，无法执行。方案 8.2 的假设 A1 因此未获真实端点证实，字段规则按已确认的 D10 决策实现，不在实现中自行调整）**
- [ ] 7.3 手动验证需求：选择模型后请求中的 `model` 与所选一致；同一会话内手动切换 GPT / MiniMax 清单中的模型连续生成三次，第二、三次基于上一张图；仅配置 GPT 的旧配置文件升级后无需重新录入即可生成（原「默认参数下连续三次生成依次为 GPT、MiniMax、GPT」的自动轮转验证项已按 2026-11-12 的需求变更移除） —— **未完成（原因：需要真实可用的 GPT 与 MiniMax 端点做手动验证——选择模型后请求 `model` 与所选一致、同会话手动切换两家模型连续生成且第二、三次基于上一张图、旧配置文件升级后可直接生成；本环境无真实端点凭据，无法执行）**
- [x] 7.4 在 `CHANGELOG.md` 记录：两项新能力（原第三项「双家自动轮转」已按 2026-11-12 的需求变更移除；Task 18 起「供应方连通性探测」为第三项，见第 9 节，`CHANGELOG.md` 现行小节标题为「三项新能力」）；默认模型由 `gpt-image-2` 变为 `gpt-image-2.6-flare` 及加回方法；新形态配置文件不可被旧版本读取；`input_fidelity` 参数移除
- [x] 7.5 `openspec validate add-image-multi-provider --strict` 通过
- [ ] 7.6 归档本变更时同步改写主 spec `openspec/specs/image-create-agent/spec.md` 的 **Purpose 段**：其中「调用 gpt-image-2 网关」的措辞在双供应方下已过时，改为「调用所选供应方的图片端点」；delta 只作用于 Requirements，Purpose 不会被自动同步，必须手工改（评审发现项 4） —— **未完成（原因：归档需用户确认后再执行，不在本次实施范围内；本条为归档时的手工改写动作，须在 `openspec archive` 时一并完成）**
- [ ] 7.7 归档前逐条复核 delta 的 MODIFIED/ADDED 与主 spec 的对应 Requirement：确认「生成结果按媒体类型直接展示」等被 MODIFIED 的条目在归档后不与「生成结果记录供应方与模型」自相矛盾（评审发现项 1，已在 delta 补 MODIFIED） —— **未完成（原因：本条是归档前的复核步骤，须在 `openspec archive` 时随归档一并执行；本次只做归档准备，未执行归档）**

## 8. 需求变更：移除自动轮转（2026-11-12）

- [x] 8.1 移除自动轮转：模型改由用户在已配置且可用的供应方清单中手动选择；生成请求的 `model` 必填且不再有保留值；目标解析只返回供应方与模型，所选模型不可用时报错、不换家；结果不再记录选择方式；无可用供应方时生成按钮禁用并提示先配置（已由 Task 16 后端 `25842681`、Task 17 前端 `ef713dea` + `2bfaee26` 落地）
- [x] 8.2 契约与文档同步：delta spec 删除「双供应方自动轮转」Requirement 并改写相关条目（当时为 7 条 MODIFIED + 7 条 ADDED，76 个 Scenario；现行计数见 12.4）；`proposal.md`、本文件与 `CHANGELOG.md` 同步；`openspec validate add-image-multi-provider --strict` 通过
- [x] 8.3 内置 GPT 推荐清单改为 `gpt-image-2.6-flare` 打头（其余 7 个保持原顺序），使参数面板的初始选中项是 OpenAI 官方 id —— **已完成（后端由 Task 16 落地：`src/product/image_create/models.rs` 的 `DEFAULT_GPT_MODELS` 已调整为 `gpt-image-2.6-flare` 打头；参数面板的初始选中项因此是 OpenAI 官方 id `gpt-image-2.6-flare`）**

## 9. 补充：供应方连通性探测与 Base URL 填写说明（2026-11-12）

- [x] 9.1 契约：`POST /api/image-create/settings/connectivity` 的请求 / 响应形态；两步探测策略（`GET /v1/models` → 故意不含 `prompt` 的 `POST /v1/images/generations`）与各状态的判定、文案；`api_key` 为空时后端改用该供应方已保存的 key，两者皆无 → 400 且不出站；安全边界（与生成出站共用 `validate_base_url`、禁重定向、key 不进结果与日志、不落盘、连接 10s / 总计 30s）；Base URL 只填主机根路径（方案 5.5、5.9、5.11）
- [x] 9.2 后端：探测实现（`src/cross_cutting/image_client.rs`）与接口（`src/web/handlers/image_create.rs`，路由挂载于 `src/web/app.rs`）；验收：方案 7.4 的 P-01 至 P-10、PH-01 至 PH-06 通过，`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked` 通过 —— **已完成（控制方独立复验：`cargo fmt --check` 通过、clippy `-D warnings` 零告警、`cargo test --locked` lib 6381 passed / 0 failed、it_web 631 / 0、其余 target 全过；探测用例 14 条先红后绿）**
- [x] 9.3 前端：探测类型与 API、状态动作、设置弹窗的 Base URL 示例与说明、「测试连通」按钮与内联结果；验收：方案 7.4 的 PF-01 至 PF-07 通过，`cd web && pnpm tsc -b && pnpm vitest run` 通过 —— **已完成（控制方独立复验：`pnpm tsc -b` 零错误、`pnpm vitest run` 282 files / 2949 tests 全通过；前端用例 16 条先红后绿）**
- [x] 9.4 契约文档同步：方案新增 5.11「连通性探测」并补 5.5、5.7、5.8、5.9、7.3、7.4、8.2、8.3、9、10.3；delta spec 在「网关配置录入与脱敏存储」补 Base URL 只填主机根路径（新增 2 个 Scenario），新增 4 条 ADDED Requirement（「供应方连通性探测」「连通性探测的逐步判定规则」「连通性探测的取值来源与安全边界」「连通性探测的界面交互」，共 18 个 Scenario；拆成 4 条是因为 strict 校验要求单条 Requirement 正文不超过 500 字符）；当时为 7 条 MODIFIED + 11 条 ADDED，96 个 Scenario（现行计数见 12.4）；`proposal.md`、本文件与 `CHANGELOG.md` 同步；`openspec validate add-image-multi-provider --strict` 通过

## 10. 补充：模型清单以连通性探测结果为来源与复选下拉框（2026-11-12）

- [x] 10.1 契约：探测响应新增 `available_models`（`/v1/models` 一步成功时为网关返回的全部 model id，去重保序；否则为空数组），其余字段语义不变；设置弹窗的模型清单改为复选下拉框——探测取得非空的 `available_models` 时（不看 `ok`）草稿清单**替换**为网关返回的模型（去重、保持网关顺序，减去已被另一家勾选的 id 并提示，至多 256 个并提示），候选池 = 网关模型（网关顺序在前）∪ 探测前草稿清单已有项（追加在后，未勾选并标注「网关未列出」）；`available_models` 为空时清单与候选池不变；「测试连通」按钮下说明「成功后会用网关返回的模型列表更新模型清单」；可添加自定义 id；单家清单上限 32 → 256；参数面板初始选中项改为三级优先规则；探测第 2 步的模型 id 取请求 `models` 中第一个 id 含 `image` 的项，否则首项（方案 D1、D2、5.5、5.9、5.11）。裁决沿革：初版为「探测只扩充候选池、不改已有勾选」，同日由主控修正为替换语义
- [x] 10.2 后端：`ProbeReport` 新增 `available_models`（`src/cross_cutting/image_client.rs`，接口响应随之带出）；探测第 2 步的模型 id 改为取请求 `models` 中第一个 id 含 `image`（忽略大小写）的项，否则首项；`MAX_MODELS_PER_VENDOR` 由 32 改为 256（`src/product/image_create/models.rs`）；验收：方案 7.5 的 PA-01 至 PA-06 与 7.1 的 S-10（以 256 为界）通过，`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked` 通过 —— **已完成（控制方独立复验：`cargo fmt --check` 通过、clippy `-D warnings` 零告警、`cargo test --locked` lib 6386 passed / 0 failed、it_web 631 / 0、其余 target 全过、`large_file_guard` 通过）**
- [x] 10.3 前端：`ConnectivityProbeResult.available_models`；状态层的 `modelPools` 与候选池动作、初始选中项三级规则；新增 `ModelMultiSelect` 与 `ModelListField`；设置弹窗的模型清单由 `textarea` 换成复选下拉框、探测取得网关模型列表后以它替换草稿清单、「测试连通」按钮下的更新说明、「恢复推荐清单」改为并入候选池并勾选；验收：方案 7.5 的 MP / MS / MD 用例通过，`cd web && pnpm tsc -b && pnpm vitest run` 通过 —— **已完成（控制方独立复验：`pnpm tsc -b` 零错误、`pnpm vitest run` 284 files / 2978 tests 全通过）**
- [x] 10.4 契约文档同步：方案补 D1（上限 256）、D2（边界说明）、5.5（`available_models`）、5.8、5.9（「设置弹窗的模型清单」与初始选中项三级规则）、5.11（「模型清单以探测结果为来源」与探测模型 id 的选取）、7.2、7.3、7.4，新增 7.5、8.3 的 R8 至 R10、第 9 节 Task 19 行与 10.4；delta spec 改写「模型清单可配置」（候选项来源、上限 256，新增 4 个 Scenario、移出 1 个）、「供应方连通性探测」（`available_models`，新增 1 个 Scenario）、「连通性探测的逐步判定规则」（探测模型 id 的选取，新增 2 个 Scenario）、「连通性探测的界面交互」（探测后更新清单与入口旁说明，新增 2 个 Scenario），新增 2 条 ADDED Requirement（「模型初始选中项」4 个 Scenario、「模型清单以连通性探测结果为来源」12 个 Scenario，替换语义）；当时为 7 条 MODIFIED + 13 条 ADDED，120 个 Scenario（现行计数见 12.4）；`proposal.md`、本文件与 `CHANGELOG.md` 同步；`openspec validate add-image-multi-provider --strict` 通过

## 11. 缺陷修复：会话历史按时间交织呈现（2026-11-12）

- [x] 11.1 契约：会话记录的四类条目（对话消息、建议 prompt、生成结果、事件）合并为一条时间线，按时间戳升序呈现；时间戳相同的依次为对话消息、建议 prompt、生成结果、事件，类别也相同的保持会话记录中的先后；建议 prompt 块新增可选时间戳 `ts`（写入时记录，读取接口返回，旧记录缺该字段时读为空）；没有时间戳的旧建议 prompt 以同序号用户消息的时间定位，再退回会话创建时间；历史条目的 id 按排序后的最终位置编号（方案 5.1、5.8、5.12）
- [x] 11.2 后端：`PromptBlock` 新增 `ts: Option<DateTime<Utc>>`（缺省读为空、为空时不写出）；`persist_iteration` 追加提示词块时写入当前时刻；会话读取 DTO 原样带出；验收：方案 7.6 的 OB-01 至 OB-05 通过，`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked` 通过，`large_file_guard` 通过 —— **已完成（控制方独立复验：`cargo fmt --check` 通过、clippy `-D warnings` 零告警、`cargo test --locked` lib 6391 passed / 0 failed、it_web 631 / 0、其余 target 全过、`large_file_guard` 通过）**
- [x] 11.3 前端：`ImageCreatePromptBlock` 新增 `ts?`；`buildEntries` 改为合并后按时间戳升序排序、同时间戳按固定次序、旧记录兜底、按最终位置重新编号；验收：方案 7.6 的 OF-01 至 OF-07 通过，`cd web && pnpm tsc -b && pnpm vitest run` 通过 —— **已完成（控制方独立复验：`pnpm tsc -b` 零错误、`pnpm vitest run` 285 files / 2989 tests 全通过）**
- [x] 11.4 契约文档同步：方案新增 5.12「会话历史的时间交织」、7.6、10.5，并补 5.1（`PromptBlock`）、5.8（`ImageCreatePromptBlock`）、5.9、5.10、7.3、8.3 的 R11 至 R13、第 9 节 Task 20 行；delta spec 新增 2 条 ADDED Requirement（「会话历史按时间顺序交织呈现」4 个 Scenario、「建议 prompt 记录时间戳且旧记录可定位」4 个 Scenario；分成 2 条是因为呈现顺序与数据字段可分别验收）；当时为 7 条 MODIFIED + 15 条 ADDED，128 个 Scenario（现行计数见 12.4）；`proposal.md`、本文件与 `CHANGELOG.md` 同步；`openspec validate add-image-multi-provider --strict` 通过

## 12. 体验修复：模型未产出与结构化输出失败分开提示（2026-11-12）

- [x] 12.1 契约：结构化提示契约改为双出口——原首句里的「（不要输出其他内容）」移到 JSON 块之后并改为「除上面的 JSON 块外不要输出其他内容。」，新增「无法或不宜给出图片 prompt 时仍必须输出 JSON 块、把 `suggested_prompt` 置为空字符串，并可在 JSON 块之外用一段文字说明原因」；迭代结果与迭代事件新增 `prompt_declined`，仅在「结构化输出解析成功且 `suggested_prompt` 为空或缺失」时为真，结构化输出没解析出来时为假；两种降级都不改动当前建议 prompt；只要本轮未产出就给出恰好一条提示（不要求存在上一版），排在模型说明之前；提示文案 = 主句（模型未产出为「本轮模型未产出可用 prompt」，结构化输出失败为「本轮未产出新的建议 prompt」）+ 模型未产出且有说明时的「（下方是它给出的说明）」+ 存在上一版时的「，已保留上一版」；仅模型未产出时把模型说明折叠为默认收起的「查看模型说明」，结构化输出失败时直接显示；完成事件未带该标记时以文本事件的标记为准，均未带时按结构化输出失败处理；事件推送顺序不变（方案 5.8、5.9、5.10、5.13）。裁决沿革：初版为「存在上一版才提示、未产出时说明一律折叠、既有契约一字不改只追加」，同日由主控裁决修正
- [x] 12.2 后端：`prompt_iteration.rs` 的提示契约按 12.1 改写、`IterationOutcome` 新增 `prompt_declined` 并按 12.1 判定；`IterationEvent` 新增 `prompt_declined`（`#[serde(default)]`），`text`、`prompt`、`done` 事件带出本轮的值，`error` 事件为假；验收：方案 7.7 的 DB-01 至 DB-08 通过（DB-06 至 DB-08 的用例由提交 `24e92c23` 补齐），`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked` 通过，`large_file_guard` 通过 —— **已完成(控制方独立复验:`cargo fmt --check` 通过、clippy `-D warnings` 零告警、`cargo test --locked` lib 6403 passed / 0 failed、it_core 221 / 0、it_web 631 / 0、其余 target 全 0 failed、`large_file_guard` 2 passed / 0 failed)**
- [x] 12.3 前端：`IterationEvent.prompt_declined?`；fallback 文本条目仅在 `prompt_declined` 为真时标记 `collapsed`；完成事件只要本轮未产出就生成提示，文案按 12.1 拼接，并把提示插到 fallback 文本之前；`ChatPane` 把 `collapsed` 的模型文本渲染为默认收起的「查看模型说明」；验收：方案 7.7 的 DF-01 至 DF-14 通过，`cd web && pnpm tsc -b && pnpm vitest run` 通过，`image-create-store.ts` 不超过 900 行 —— **已完成(控制方独立复验:`pnpm tsc -b` 零错误、`pnpm vitest run` 286 files / 3004 tests 全通过、`image-create-store.ts` 833 行 ≤ 900;浏览器真实 provider 拒答实测:提示排在模型说明之前、说明折叠为默认收起的「查看模型说明」)**
- [x] 12.4 契约文档同步：方案新增 5.13「模型未产出与结构化输出失败的区别」、7.7、10.6，并补 5.8（`IterationEvent`）、5.9、5.10、5.12（`buildEntries` 的所在文件）、7.3、8.2 的 A3、8.3 的 R14 至 R18、第 9 节 Task 21 行；delta spec 新增 1 条 MODIFIED Requirement（「结构化建议 prompt 的约定与降级」，6 个 Scenario）与 2 条 ADDED Requirement（「未产出建议 prompt 时的提示与模型说明呈现」7 个 Scenario、「未产出建议 prompt 时的提示文案」8 个 Scenario；分成 2 条是因为呈现规则与文案可分别验收，且 strict 校验对单条 Requirement 正文有长度上限）；当时为 8 条 MODIFIED + 17 条 ADDED，149 个 Scenario（现行计数见 13.8；前文各处「现行计数见 12.4」同此）；`proposal.md`、本文件与 `CHANGELOG.md` 同步；`openspec validate add-image-multi-provider --strict` 通过

## 13. 合入前审查与修复（2026-11-12）

> 合入前对整分支做了代码审查与安全审查，结论：**0 个 Critical、3 个 Important、若干 Minor**。下列条目逐条记录问题、影响、修法与提交号；后端提交号已补入（13.9）。复验（13.10）由主控执行，本节不代勾。

- [x] 13.1 **Important · 后端 · 安全：探测回落已保存 key 时不看 `base_url`**。问题：连通性探测请求未提供 `api_key` 时，后端无条件改用该供应方已保存的 `api_key`，而探测地址取自请求里的 `base_url`。影响：调用探测接口的一方（包括用户在设置里改了 Base URL、没填 key 就点「测试连通」）可以让已保存的 key 被发往未保存的地址，违反既有 Requirement「API Key 安全边界与出站目标约束」的「每个供应方的 `api_key` SHALL 仅随发往该供应方自身 `base_url` 的请求发送」。修法：请求未提供 `api_key` 时，仅当请求的 `base_url` 与该供应方已保存的 `base_url` 一致（两侧去首尾空白并去掉末尾的 `/` 后逐字符相等，不做其他归一化）才改用已保存的 key；不一致 → 400「Base URL 与已保存配置不同，请填写 API Key 后再测试连通性」且不发起任何出站请求；显式提供 key 时照旧用它；从未保存过 key 时仍是「未配置 API Key，无法测试连通性」。本修复是让实现回归既有契约。提交：后端 `75603c2e`
- [x] 13.2 **Important · 后端 · 安全：迭代失败事件未脱敏**。问题：prompt 迭代失败时，执行器的失败信息未经脱敏就作为 `error` 事件推给前端，而成功路径的文本与建议 prompt 已做脱敏。影响：迭代执行器的工作目录在 `.aria/` 之下，能读到设置文件里的明文 key；key 一旦出现在失败信息里，就会显示在对话区。修法：失败路径复用成功路径的同一脱敏实现，按两个供应方已保存的 key 替换后再推送 `error` 事件（落在 `engine.rs` 的 `send_iteration_error`；该失败信息不落进会话记录）。提交：后端 `75603c2e`
- [x] 13.3 **Important · 前端：重新打开设置弹窗会重放上一次的探测结果**。问题：探测结果保存在状态层、跨弹窗开关保留，弹窗每次打开都把上一次的结果当成新结果处理。影响：用户探测后取消勾选部分模型并保存，关闭再打开弹窗，草稿清单被还原成网关全量清单；此时只改别的字段、点保存，就会把精简掉的模型写回。修法：只有用户主动点「测试连通」才用网关模型列表更新清单；弹窗打开时把已存在的探测结果标记为已处理，结果仍可内联展示但不触发清单替换。提交：前端 `98a70a76`
- [x] 13.4 **Minor · 前端 · 安全：API Key 输入框回显可编辑的脱敏串**。问题：输入框的 value 是已保存 key 的脱敏串（如 `sk-****1234`），动作以「值是否等于脱敏串」推导。影响：用户改一个字就得到 `sk-****1234X` 并以替换动作保存，真 key 被这串内容覆盖。修法：输入框 value 恒为空，脱敏串只作 placeholder 与状态文案「已保存 sk-****1234，留空保持不变」；输入非空 → `replace`，输入后删空 → `retain`；「清除 key」按钮与「保存后将清除」placeholder 不变；前端的探测与保存请求都不提交脱敏值。提交：前端 `9be360f3`
- [x] 13.5 **Minor · 后端：成功响应体 64 MiB 上限会拦下 4K 档的合法大图**。问题：图片端点成功响应体的读取上限为 64 MiB，超限被定性为恶意 / 异常端点。影响：内置清单含 `gpt-image-2.6-flare-4k` / `gpt-image-2.6-sunburst-4k`，4096² PNG 的 b64 载荷最坏会逼近该上限，合法响应会被拒。修法：上限提到 128 MiB，超限文案改为「图片端点响应体超过大小上限（128 MiB）」并不再定性为恶意端点；错误体 8 KiB 与探测模型列表 1 MiB 两个上限不变。提交：后端 `75603c2e`
- [x] 13.6 **Minor · 文档：三处精度修正**。① delta spec「未产出建议 prompt 时的提示文案」把「存在上一版」定义为提示词编辑框非空，而同批 Scenario 的前提写的是「会话尚无建议 prompt」，用户首轮就在编辑框里手打了内容时两者分叉 → 相关 Scenario 的前提统一为「提示词编辑框非空 / 为空」，与 Requirement 正文及方案 8.3 的 R14 一致。② 本文件 7.4 的「两项新能力」计数过时 → 补注连通性探测为第三项。③ 实施计划 Task 15 的 Step 4b / 6 / 7 未勾选但实质已由后续 Task 16–21 完成 → 核对后补勾并更新状态行。提交：见契约文档同步的提交（13.8）
- [x] 13.7 **Minor · 后端加固**。① 设置文件保存的权限窗口：问题——含明文 key 的设置文件在写入临时文件到改名、以及改名到收紧权限之间可能以宽权限存在；影响——该窗口内（或中途崩溃后）明文 key 文件对同机其他用户可读；修法——临时文件创建即 0600 的 owner-only 原子写，收紧权限失败时删除已落盘文件再返回错误。② 结构化输出块剥离不彻底：问题——执行器输出含多处完整的结构化输出块时只剥离首个；影响——其余块残留进可读文本并显示给用户；修法——结构化输出块的剥离改为循环，剥掉输出里全部完整配对。提交：后端 `85b5e90c` + `2a28ece0`（owner-only 原子写）、`7595b530`（复现用例）+ `91dbf2f3`（剥离循环）
- [x] 13.8 契约文档同步：delta spec 改写「连通性探测的取值来源与安全边界」「连通性探测的界面交互」「模型清单以连通性探测结果为来源」「网关配置录入与脱敏存储」「API Key 安全边界与出站目标约束」五条的正文，新增 **6** 个 Scenario（「网关配置录入与脱敏存储」2 个、「连通性探测的取值来源与安全边界」1 个、「连通性探测的界面交互」1 个、「模型清单以连通性探测结果为来源」1 个、「API Key 安全边界与出站目标约束」1 个——迭代失败的事件不携带 key 明文），不新增 Requirement，并按 13.6 ① 统一前提口径；方案补 D12 第 9 点、5.5、5.6（响应体读取上限与 `OversizedBody`）、5.8、5.9、5.11、7.3、7.4 的 PH-04、8.3 的 R19 与 R20、第 9 节 Task 22 行，新增 10.7；实施计划补勾 Task 15 的三个收尾项并追加 Task 22；`proposal.md`、本文件与 `CHANGELOG.md` 同步；现为 **8 条 MODIFIED（49 个 Scenario）+ 17 条 ADDED（106 个 Scenario），155 个 Scenario**；`openspec validate add-image-multi-provider --strict` 通过
- [x] 13.9 已补入 13.1、13.2、13.5、13.7 的后端提交号（`75603c2e`、`85b5e90c` + `2a28ece0`、`7595b530` + `91dbf2f3`）并勾选对应条目，方案第 9 节 Task 22 行同步
- [ ] 13.10 复验（主控执行）：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`cd web && pnpm tsc -b && pnpm vitest run`、`openspec validate add-image-multi-provider --strict`
