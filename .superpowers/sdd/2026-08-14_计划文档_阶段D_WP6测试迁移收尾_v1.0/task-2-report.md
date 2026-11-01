# Task 2 报告：50 成员预算合成 case

## 范围与 fixture

- 新增 `tests/it_web/evidence_query/part_04.rs`，并由 `tests/it_web/evidence_query.rs` include。
- `FiftyMemberFixture` 通过 `LogicalCodebaseStore` 直接落盘 50 条 manifest/member/checkout 记录，通过 `AggregateIndexStore` 直接落盘 active aggregate-index record。
- 每个成员仅创建 aggregate-root 下的普通目录与一个 synthetic source 文件；不初始化 50 个真实 Git 仓库、不调用真实 Provider。
- 跨仓 HTTP evidence 场景复用现有真实认证 fixture 的两个 Git 成员（api/web），再以 store + 普通目录追加 48 个合成成员，更新 manifest/index/attempt membership revision 为 2。这样既覆盖 50 成员 ACL 目录扫描，又保留真实 CodeGraph 查询命中路径。

## 断言要点

1. **成员清单/等价查询**：通过真实 Web `POST /api/projects/{project}/issues/{issue}/story-specs:generate` 的 planning context 消费路径查询 authoritative member listing；断言 50 个 `logical_repository_id`、50 个 alias 全量出现且分别唯一，manifest/member/checkout 数量均为 50。
2. **跨仓 evidence**：真实 HTTP `POST /api/evidence-query`，复用真实 token 与真实 handler；断言 `web/src/app.ts` 命中，目标 api 目录、非成员 other 目录及 48 个 synthetic member 目录均不泄露；耗时上限 `<5s`，注释说明该上限是防回归宽松上限而非 benchmark，避免 CI 抖动。
3. **规划预算**：仓库已有 `PlanningContextResolver` / `InventoryInjection` 截断能力，因此使用 50 条 store fixture、长 profile metadata、`PlanningContextResolver::build` 实际路径断言 `truncated=true`、保留目标成员、`omitted_member_count=49`、省略 49 个成员且输出不超过 hard budget。

## 红绿证据

- 早期 targeted run 曾因初版测试 fixture profile 太短而使预算边界未触发（测试失败：`50-member inventory must report truncation`）；随后将测试数据改为真实 resolver 路径并注入长 profile，确认边界断言有效。
- 修正后的 targeted：`cargo test --locked --test it_web fifty_member --no-fail-fast` → 3 passed。
- 全量 it_web：`cargo test --locked --test it_web` → 379 passed / 0 failed / 12 ignored（当前仓库基线测试数为 376，新增 3）。

## 变更文件

- `tests/it_web/evidence_query.rs`：include 新增 `part_04.rs`。
- `tests/it_web/evidence_query/part_04.rs`：50 成员合成 fixture、成员清单/HTTP evidence/预算边界三项测试。
- 本报告：任务报告产物。

## 收尾验证

- `cargo fmt`、`cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo check --locked`：通过。
- `git diff --check`：通过。
- 未使用 `-j 1`；无真实 Provider 调用。

## 自审与残余风险

- 测试仅新增 it_web 场景，未修改生产代码，范围未扩大。
- 成员清单没有独立 public logical-member route；因此断言通过真实 Story planning HTTP 入口的 authoritative inventory 消费路径，报告中明确标注为等价查询路径。
- evidence fixture 中追加成员为普通目录，不是 48 个真实 Git checkout；这是契约要求的合成数据方式。现有 api/web 两个真实成员保证 CodeGraph HTTP 查询仍是真实路径。
- 5 秒耗时断言覆盖了 HTTP handler + 成员 ACL 枚举 + CodeGraph 查询，但不是严格性能基准。
