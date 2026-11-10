# Tasks

## 1. 全域前置门（聚合初始化未完成 ⇒ 知识库一律不可用）

- [ ] 1.1 把「聚合初始化已完成」判定统一到冷启动投影的「规则/政策」步（`LogicalCodebaseBootstrapProjector::project`），替换 `preflight.rs` 里「根规则 + 政策 artifact」的两文件判定，投影读不出按未完成处理；验证：单元测试覆盖「规则/政策 completed ⇒ 通过」「未完成 ⇒ 拒」「投影读不出 ⇒ 拒」
- [ ] 1.2 在知识库 HTTP 层做**一处**统一前置门并覆盖全部既有端点（status/files/file 读写/bootstrap-inputs/change-packages×3/initializations×4），失败统一返回 `knowledge_base_requires_aggregate_initialization` + 中文可执行文案；验证：集成测试逐端点断言门未满足时 4xx 且无写副作用
- [ ] 1.3 前端在卡片与详情页呈现同一前置提示，且详情页在门未满足时禁用保存/确认等写入口；验证：组件测试断言提示文案与禁用态

## 2. 两段式输入草稿（provider 调 /knowledge-base-user-input）

- [ ] 2.1 新增 `KnowledgeBaseOperationKind::UserInput` 与 `KnowledgeBaseStepKind::PrepareUserInput`，命令文本用既有 `skill_command_prefix` 生成 `knowledge-base-user-input`，旧 operation 记录反序列化仍成功；验证：单元测试覆盖三 provider 的命令文本与旧记录反序列化
- [ ] 2.2 起草 operation 的 provider turn：cwd = canonical 聚合根、写入面审计允许面 = `cadence/knowledge-base/user-input/**`、越界即失败、单飞与取消沿用既有机制；验证：单元测试覆盖允许面内通过与越界失败
- [ ] 2.3 实现「输入未准备」判定（9 份缺失或全部与插件模板逐字相同 ⇒ 未准备；任一不同 ⇒ 已准备），插件模板源路径可注入以便测试；验证：单元测试覆盖缺失/全占位/已填写三种形态
- [ ] 2.4 创建初始化 operation 时按状态分叉：未准备 ⇒ 起草；已准备未确认 ⇒ `knowledge_base_user_input_not_confirmed` 且不产生 operation；已确认 ⇒ 跑 bootstrap；验证：集成测试逐态断言

## 3. 用户确认状态与失效语义

- [ ] 3.1 新增 per-LC 确认记录存储（平台侧 .aria，不写 `user-input/`），记录 `path → { digest, confirmed_at }`；验证：单元测试覆盖写入/读取/按 LC 隔离
- [ ] 3.2 新增输入清单与确认端点：GET 列出 9 份文件及「未填/待确认/已确认」状态；POST 确认（单份/批量）时服务端重算摘要比对，不一致以 `knowledge_base_user_input_confirmation_stale` 拒绝；验证：集成测试覆盖单份确认、批量确认、过期摘要被拒、编辑后确认失效
- [ ] 3.3 详情页新增输入确认区：逐份状态徽标、逐份「确认」与「全部确认」、已确认文件被编辑后提示「需重新确认」；验证：组件测试覆盖确认动作与失效提示

## 4. 详情页改版（全屏工作区）

- [ ] 4.1 CockpitShell 增加路由级沉浸模式（知识库路由白名单命中时只渲染 children，不渲染全局浮动件）；验证：既有外壳测试全绿 + 新增「知识库路由下无设置触发器」断言
- [ ] 4.2 详情页重做：顶栏（知识库标题 + 所属项目/逻辑代码库关联信息 + 状态徽标 + 返回工作台按钮）、左侧文件树（可折叠、固定宽）+ 右侧编辑/预览两栏自适应、编辑区占满可用高度、下半屏无空白；验证：组件测试断言顶栏信息、返回入口与布局结构
- [ ] 4.3 保持机器管理文件只读呈现与未保存守卫不变；验证：既有详情页测试全绿

## 5. 验收与归档

- [ ] 5.1 按 spec 的验收矩阵逐条真机走查（前置门四态、两段式三态、确认与失效、详情页形态），留截图证据到 `cadence/reports/evidence/`；验证：逐条对应 spec scenario
- [ ] 5.2 全量门禁：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`pnpm -C web test`、`pnpm -C web build`、`pnpm -C web lint`；验证：全部退出码 0
- [ ] 5.3 勾选工作包并执行 OpenSpec sync/archive；验证：`openspec validate <change> --strict` 通过且主规格已同步新 requirement
