# Tasks

## 1. 后端底座：知识库 operation 模型与落盘

- [x] 1.1 建 `src/product/logical_codebase/knowledge_base/` 模块骨架（operation / store / command / coordinator / templates / change_package / manifest / files），并在 `mod.rs` 导出；验证：`cargo check --locked` 通过且模块可被 `src/product/logical_codebase/mod.rs` 引用
- [x] 1.2 定义 `KnowledgeBaseOperation`（kind = bootstrap|update、input 含 provider 与变更包路径、steps、provider_selection、error、时间戳）与状态枚举；验证：新增单元测试断言「bootstrap 只有一个 provider 步骤」「update 只有一个 apply_update 步骤」通过
- [x] 1.3 实现 durable store：落 `lc_scope_root(...)/knowledge-base-operations/`，`operation_id` 派生含 `lc_id`，支持 create/get/latest/finish_failed/cancel；验证：单元测试覆盖「两个 LC 传同一 idempotency_key 不撞 id」「latest 返回当前 LC 最新 operation」
- [x] 1.4 实现命令文本生成：复用 `RepositoryInitializationStepKind::skill_command_prefix`，bootstrap = 前缀 + `knowledge-base-bootstrap`，update = 前缀 + `knowledge-base-update <变更包相对目录>`；验证：单元测试逐 provider 断言命令文本逐字相等（Claude `/`、Pi `/skill:`、Codex `$cadence-init:`），且不新增第二份前缀映射

## 2. 前置检查与门禁

- [x] 2.1 实现 skills 可用性检查：按被选 provider 取对应 skill 层，校验 7 个 `knowledge-base-*` skill 全部存在（bootstrap / base-info / api / pages / overview / context / update）；验证：单元测试覆盖「缺 knowledge-base-overview ⇒ 失败」「缺任一即失败」
- [x] 2.2 实现六领域输入检查：读 `base-info.md` 唯一入口，按各领域「状态」条件判定 scope 文件必需性（不适用 ⇒ 不要求），`database-ddl.sql` / `product.md` 视为可选；验证：单元测试覆盖「状态=不适用且文件缺失 ⇒ 通过」「状态=全量且文件缺失 ⇒ 失败并列出缺失文件」
- [x] 2.3 实现 Manifest 解析与不变量校验（`schema_version` = 4.0、`coverage.initialization` 八项、`update.last_change_package` / `processed_packages`、六领域 scope 存在、complete + global_validation passed）；验证：单元测试覆盖「in_progress ⇒ 失败」「schema 非 4.0 ⇒ 失败」「complete ⇒ 通过」
- [x] 2.4 实现变更包门禁 P1–P9：路径形态、五份齐全、字段非空（枚举占位判未填）、「不适用」须带原因、七行领域矩阵、敏感信息保守扫描（不回显命中值）、起止提交本地可解析、配置分支与 Manifest 一致；验证：单元测试逐条覆盖九个门禁的通过与失败样例
- [x] 2.5 实现「聚合初始化已完成」前置检查（根规则与已发布政策存在）；验证：单元测试覆盖「根规则缺失 ⇒ 以需先完成聚合初始化失败」

## 3. 模板骨架与文件读写面

- [x] 3.1 实现模板骨架创建：从插件源根复制知识库输入模板 9 份与变更包模板 5 份 + attachments 说明，只补缺失文件不覆盖；变更包标识由用户给定并做 `CHANGE-` 前缀归一与安全字符校验；验证：单元测试覆盖「已存在文件不被覆盖」「重复创建同标识返回已存在且不覆盖」「非法标识被拒」
- [x] 3.2 实现知识库文件树与读取：根 = canonical 聚合根 + `cadence/knowledge-base`，相对路径解析后必须仍在根内（禁 `..`、绝对路径、符号链接逃逸），单文件上限 1 MiB，返回内容与摘要；验证：单元测试覆盖「穿越路径被拒」「符号链接指向根外被拒」
- [x] 3.3 实现文件保存：写入白名单 = 全域 `*.md` + `user-input/**` 下 `*.sql`，原子写（临时文件 + rename），乐观并发（客户端摘要不符 ⇒ 拒绝覆盖），知识库 operation 运行期间拒写该目录；验证：单元测试覆盖「清单文件不可写」「摘要不符 ⇒ 冲突拒绝」「运行期 ⇒ 拒绝」「并发保存不产生半写文件」

## 4. provider turn 执行与写入面审计

- [x] 4.1 实现知识库 provider turn：以 Normal 相位 + `CodingTargetWrite`（target = canonical 聚合根）、cwd = 聚合根、可写根 = 聚合根，经 gateway 启动并记入审计；取消 / 超时 / 输出摘要语义与聚合初始化一致；验证：单元测试断言 gateway 启动审计计数增加且 envelope 形状符合预期
- [x] 4.2 实现知识库 provider 选择：复用 `select_aggregate_provider` 的同源判定，但按 `CodingTargetWrite` 分格判 L2，创建 operation 前确认该分格 `launch` 与 `write_boundary` 均 Confirmed，不满足按能力拒绝失败面返回且不产生 operation；验证：单元测试覆盖「选择通过分格与起跑分格一致」「写分格未确认 ⇒ 创建前失败」
- [x] 4.3 实现写入面审计：命令前后对聚合根做快照差分，允许面 = 知识库目录 + 项目规则目录 + 聚合根代理规则入口文件；越界 ⇒ operation 失败并记稳定码；验证：单元测试覆盖「越界写 ⇒ 失败」「允许面内写 ⇒ 通过」
- [x] 4.4 实现 trust 前置门与取消：创建期复用 `ProviderTrustPrecondition` 的 Recipe 场景判定，等待面沿用聚合初始化口径；取消后停止后续执行；验证：单元测试覆盖「trust 等待 ⇒ 失败且不启动 provider」「取消 ⇒ 终态为取消且不再推进」

## 5. HTTP 面、DTO 与稳定码登记

- [x] 5.1 新增知识库 HTTP 路由与 handler（initializations create/active/get/cancel、status、files、file 读与写、change-packages 列表/lint/创建、bootstrap-inputs 创建）；验证：`openspec` 无关的路由级集成测试覆盖创建 → 轮询 → 取消与 status 投影
- [x] 5.2 新增 `InitializationOperationKind::KnowledgeBase` 与 `InitializationRunKey::knowledge_base`，并在 durable 层实现「同 LC 已有知识库 operation 在跑 ⇒ 拒绝新启」（bootstrap 与 update 互斥）；验证：单元测试覆盖「重复点击不启动第二次」「bootstrap 与 update 互斥」
- [x] 5.3 在 `failure_copy_table.inc.rs` 登记全部知识库新稳定码的中文成品文案（含缺失文件与违规项明细），并在 `src/web/error.rs` 登记 4xx/422 状态映射；验证：单元测试断言新码既有中文文案又有非 500 的 HTTP 状态
- [x] 5.4 补齐 API 层类型与客户端（`web/src/api/knowledge-base.ts`、`web/src/api/types/knowledge-base.ts`），含 status / files / file / change-packages / lint / 模板创建 / operation 四端点；验证：前端 API 客户端单测覆盖各端点的请求路径与请求体

## 6. 前端：LC 管理抽屉内的知识库卡片

- [x] 6.1 新增 `KnowledgeBaseCard.tsx` 并挂到 `LogicalCodebaseManagementPanel.tsx`（与聚合初始化卡同层），含三个按钮与步骤列表、失败文案渲染；验证：组件测试断言三个按钮存在、无成员仓时卡片不渲染
- [x] 6.2 把 provider 摘要组件从 `AggregateInitializationCard.tsx` 提升为公共组件（输入统一 `AggregateProviderSelectionDto`，两个来源装配同一 DTO），既有调用点不破；验证：既有聚合初始化卡测试全绿 + 新组件测试覆盖两个来源
- [x] 6.3 实现「知识库更新」弹窗：列出变更包及门禁状态、显示逐条问题、新建变更包（填标识 → 创建模板骨架 → 跳详情页）、仅门禁通过时「执行更新」可点；验证：组件测试覆盖「N 处问题 ⇒ 执行禁用」「门禁通过 ⇒ 可执行」
- [x] 6.4 实现「知识库初始化」的前置失败本地态与「创建输入模板」补救入口；验证：组件测试覆盖「缺输入 ⇒ 出现创建模板入口且不启动 operation」
- [x] 6.5 执行类按钮的二次确认（沿用 `window.confirm` 惯例，文案写明真实调用 provider、消耗额度、写入聚合根）；验证：组件测试断言确认前不发起创建请求

## 7. 前端：知识库详情页与可写编辑器

- [x] 7.1 新增可写 `MonacoEditor`（复用 `monaco-local-setup.ts` 与 `useSystemMonacoTheme`），保持 `MonacoViewer` 只读语义不变；验证：组件测试断言可写变体触发 onChange 且只读变体不触发
- [x] 7.2 把 `MarkdownContent` 提取到 `components/shared/`，原处保留 re-export 保住既有 import 路径；验证：既有聊天渲染测试全绿 + 新公共组件测试覆盖渲染
- [x] 7.3 新增路由 `/workbench/projects/$projectId/logical-codebases/$lcId/knowledge-base` 与页面 `KnowledgeBasePage.tsx`（左文件树、右「编辑/预览」页签、保存按钮、未保存提示用 `hooks/useUnloadGuard.ts`）；验证：路由测试断言路径可达且文件树渲染知识库文档
- [x] 7.4 变更包 5 份文档的逐字段「未填/已填」标记（数据来自后端 lint 接口）；验证：组件测试断言未填字段被标出
- [x] 7.5 保存失败面：内容冲突（409）与知识库操作运行中（409）给出明确提示，不静默失败；验证：组件测试覆盖两种 409 的提示文案

## 8. 附带修正：provider 顺序提示内边距

- [x] 8.1 `ProviderManagementPanel.tsx` 的 provider 顺序提示补内边距并与 `AggregateInitializationCard.tsx` 的同名提示样式统一（文案与 testid 不变）；验证：既有 `ProviderManagementPanel.test.tsx` 与 `AggregateInitializationCard.test.tsx` 断言文案不变且全绿，另加样式类断言

## 9. 集成验收

- [x] 9.1 按 `specs/logical-codebase-knowledge-base/spec.md` 的场景逐条走查 UI 验收矩阵（含：未聚合初始化、缺输入、缺 skill、provider 未核验、写分格未确认、进行中重复点击、KB 未 complete、变更包各类问题、敏感内容、提交不可解析、门禁通过执行、运行期禁写、并发冲突、穿越路径、机器管理文件只读），逐条留下可复核证据；验证：验收记录逐条对应 spec 场景
- [x] 9.2 回归：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`pnpm -C web test`、`pnpm -C web build` 全绿，并记录真实输出；验证：命令退出码为 0 的新鲜输出

## 完成情况（2026-11-11）

全部工作包已完成并通过独立审查（每 Task 一轮 k3 审查 + 范围化复审）。验收证据见
`cadence/reports/2026-11-09_知识库模块验收报告.md` 与 `cadence/reports/evidence/2026-11-09-knowledge-base/`。
门禁：`cargo fmt --check` 0 差异、`cargo clippy --all-targets --all-features --locked -- -D warnings` 0 警告、
`cargo test --locked` 全绿、`pnpm -C web test` 279 文件 / 2841 用例全绿、`pnpm -C web build` 成功。

## Workflow follow-up

- 实施与验证完成后，按仓库流程执行 OpenSpec sync/archive（`openspec archive`），并把 `cadence/designs/2026-11-09_技术方案_LC知识库模块_v1.2.md` 与本 change 的验收证据登记到 `cadence/reports/`。
- 真机验证 Codex 的 `$cadence-init:knowledge-base-bootstrap` 可调用性；不成立时只改 `skill_command_prefix` 一处映射并重跑相关用例。
