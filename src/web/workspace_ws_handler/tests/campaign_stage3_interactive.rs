//! 阶段 3 Task 8.2 —— interactive 对话门 campaign 用例（8.2/8.2a/8.2b/8.2c）。
//!
//! 全部用例走真实 ws inbound 分发（`handle_workspace_inbound_message`）与真实
//! durable store（`.aria` 落盘回读），fake revision provider 通过测试构造器注入
//! `ProviderRegistry`（脚本化两个完整 SC markdown 候选 / validation reject /
//! transport 死亡与恢复 / 挂起单飞），不读取任何 campaign 环境变量；生产路径
//! 零感知。每步审计记录
//! `{scenario_id, command_id, turn_id, attempt_no, budget_before, budget_after,
//! provider_start_keys, event_prefix_digest, artifact_ref, observed_status}`，
//! 候选只保存 digest/ref。

use super::*;

// 大文件守卫(>1200 行)按 include! 拆分:harness=fixture/脚本化 provider/
// 每步审计 helper;cases=8.2/8.2a/8.2b/8.2c 六个用例。
include!("campaign_stage3_interactive/harness.rs");
include!("campaign_stage3_interactive/cases.rs");
