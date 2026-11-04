# 失败簇收敛：failure_copy（FixCodeTableGap，2026-11-03 v1.0）

数据源：Main 统一 lib 全量日志 `/tmp/aria-full-lib-test.log`（本簇原定窄跑在
排队中被叫停取消，未产生额外编译）。

## 簇内测试结果（grep 'failure_copy' + create_product_issue）

| 测试 | 结果 |
| --- | --- |
| `failure_copy::tests::issue_attribution_failure_hits_the_table`（本次新增） | ok |
| `web::handlers::product_resources::create_product_issue_tests::create_product_issue_missing_lc_yields_failure_copy_registered_stable_code`（本次新增） | ok |
| `web::handlers::product_resources::create_product_issue_tests::create_product_issue_other_rejections_keep_generic_behavior`（本次新增） | ok |
| `failure_copy::tests::` 其余 12 项（selection/seven_layout/unknown/…） | ok |
| `failure_copy::tests::every_table_entry_has_three_chinese_segments` | **FAILED（既有红，已修，见下）** |

## failures 原文（Main 要求）

```
thread 'product::logical_codebase::failure_copy::tests::every_table_entry_has_three_chinese_segments' panicked at src/product/logical_codebase/failure_copy.rs:433:13:
aggregate_rule_and_mcp_config_failed 的标题里出现了机器码：provider 的第二步（rule_and_mcp_config）没有完成
```

## 处置

- **归因**：非本次改动引入。`failure_copy_table.inc.rs` / `failure_copy.rs`
  均为今日块2 未提交（untracked）产出；两个 provider turn 步骤条目的
  title_zh 内嵌了 ≥12 位小写/下划线机器码，违反模块契约「title_zh 不把
  机器码当标题」（巡检在第一处 panic，`aggregate_openspec_and_examples_failed`
  同病未及报出）。
- **修复**（`failure_copy_table.inc.rs`，仅两行 title_zh；码字/machine_line/
  reason 均未动）：
  - `aggregate_rule_and_mcp_config_failed` →「provider 的第二步（规则与 MCP 配置）没有完成」
  - `aggregate_openspec_and_examples_failed` →「provider 的第三步（openspec 与示例文件）没有完成」
- **自证**：脚本复扫全表 87 条 → 标题机器码违例 0、码字无重复；本修复未
  自行排跑（按 Main 指令），留待统一 `cargo test --all-targets` 验证。

## 本簇任务状态（已勾/剩余）

- [x] 缺口①：`issue_not_bound_to_logical_codebase` 码表补登记（TABLE 末尾
  新节；三段齐全、`reason_zh` 非 Option、triage=Environment；未造新码、
  未在 handler 另写中文）+ 钉死测试（`issue_attribution_failure_hits_the_table` ok）
- [x] 缺口②：创建入口缺 LC 提交 → 稳定码 400（Result 提取器拦截
  `JsonDataError` 且 body 含 `missing field \`logical_codebase_id\``；
  其余反序列化拒绝原样透传；字段类型未动）+ 两个 HTTP 行为测试 ok
- [x] 簇内既有红（两处标题机器码）修复，待统一跑复核
- [ ] 剩余：无（REQ-BOOT-08 范围表述与码表实际覆盖面不一致属契约层记录，
  见 `2026-11-03_码表补登记与422修复_v1.0.md`，须另开 change，本簇不处理）

## 行数门禁

`failure_copy_table.inc.rs` 754 / `failure_copy.rs` 739 /
`product_resources.rs` 639 —— 均 < 1200 硬线，均不在 GRANDFATHERED 登记表。
