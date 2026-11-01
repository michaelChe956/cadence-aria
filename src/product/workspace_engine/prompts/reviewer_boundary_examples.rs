pub(crate) fn design_reviewer_boundary_examples() -> &'static str {
    DESIGN_REVIEWER_BOUNDARY_EXAMPLES
}

const DESIGN_REVIEWER_BOUNDARY_EXAMPLES: &str = r#"
[design_reviewer_boundary_examples]
以下是 Design 边界的对照判例，只用于校准 severity 口径；不得照抄其中的文字或 ID 到真实输出。真实 finding 的 evidence 必须逐字引自当前产物。

判例 1（抽象验收追踪，合规）—— 产物摘录：
| ## 设计决策
| - [DEC-001] 删除采用软删除并保留审计字段。
| ## 追踪关系
| - source ids: Story Spec story_spec_example, Issue issue_example。
| - [DEC-001] -> [REQ-003] / [AC-003]（验收口径：删除后查询不再返回该记录）
正确判定：把 [DEC-*] 关联到 [REQ-*]/[AC-*] 且不描述如何测试，是 Design 必须承担的抽象追踪，最高只能是 suggestion；findings 为空的 pass 同样正确。错误判定：对此产生 must_fix 或 blocking。

判例 2（可执行测试内容，越界）—— 产物摘录：
| ## 公共组件
| - [CMP-002] RetryPolicy：统一重试与退避；[API-002] 定义调用方重试接口，并负责为自身编写单元测试与集成测试。
| ## 风险
| - 回归验证计划：第一步在 tests/idempotency.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked idempotency，第三步补充 3 条超时场景用例。
正确判定：出现具体测试文件或模块、测试框架或夹具、可运行命令、分步测试场景，或把测试与验证职责指派给组件或文件，必须 must_fix。错误判定：把上述可执行测试内容降级为 suggestion 或 pass。

判例 3（风险章节合法提及验证归属，不得误伤）—— 产物摘录：
| ## 风险
| - [DEC-001] 幂等键冲突概率未知；缓解：由下游 Work Item 阶段安排验证，Design 阶段不定义测试方案。
正确判定：只出现「测试」「验证」词面，而没有具体文件、框架、命令、分步场景或职责指派时，不构成 must_fix；应为 pass 且 findings 为空。错误判定：以「风险未验证」产生 must_fix。

判定顺序：先判断是否命中可执行信号（具体测试文件或模块、测试框架或夹具、可运行命令、分步测试场景、把测试或验证职责指派给组件或文件）；命中才可 must_fix，未命中最高 suggestion。
"#;
