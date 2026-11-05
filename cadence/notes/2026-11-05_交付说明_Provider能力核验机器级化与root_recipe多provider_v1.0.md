# 交付说明：provider 能力核验机器级化 + root recipe 对 provider 无关 + 删除 Project 二次确认

日期：2026-11-05
工作树：`.worktrees/bugfix-b-11-02`（分支 `bugfix-b-11-02`）
契约：`openspec/changes/globalize-provider-capability-verification/`（REQ-PCR-01/03/06/07/08/09 + REQ-BOOT-03/07/14）
　　　`openspec/changes/root-recipe-provider-agnostic/`（REQ-BOOT-03/07/15/16/17）
实施 Plan：`cadence/plans/2026-11-05_计划文档_实施_Provider能力核验机器级化_v1.0.md`（8 Task）
　　　　　`cadence/plans/2026-11-05_计划文档_实施_root-recipe对provider无关_v1.0.md`（4 Task）

---

## 1. 这一批交付了什么

| # | 交付 | 关键 commit |
|---|---|---|
1 | **能力核验机器级化**：capability 记录与探针证据从「按项目/逻辑代码库区分」改成「**整机一份**」；四条路由改机器级、旧四条**整条删除**（无别名/shim/双写）；批量作业去项目归属；前端入口**不再以选中项目为前置条件**；文案改机器级口径 | `8eda119` `9b23607` `667053d3` `2ae4b53d` `58b2e88f` `ec03be11` |
2 | **root recipe 对 provider 无关**（owner 选方案 B）：删除 `require_root_recipe_supported` 的 Claude-only 硬拒（**凭据三道检查一条不减**）；四条 recipe 命令按 provider 的 skill 调用语法生成（Claude `/` · Pi·Kimi `/skill:` · Codex `$cadence-init:`）；**root recipe turn 零变更 ⇒ 该 step 判失败**（不再静默假成功） | `1cb9253` `827bb0d` `77640c99` |
3 | **删除 Project 补二次确认**：后端未带 `?confirmed=true` 一律 **409 `deletion_confirmation_required`**（复用既有码、零新增码、门禁先于存在性判定 ⇒ 拒绝路径零副作用）；前端先弹强警告确认（取消零出站） | `b99c4a3` |
4 | 配套：夹具按真实四命令落盘面生产同构化 + A05 负例；1366 行测试文件按仓库惯例拆片段；过期契约表述更正；rustfmt 重排 | `6c9b8d62` `3404a803` `3a17563e` `39163bb0` `0505b276` |

## 2. 验证证据（全实测）

```
cargo test --locked --all-targets
  lib 6001/0 · it_core 198/0 · it_web 602/0 · web_logical_codebase_entrypoints 49/0
  it_product 211/0 · it_provider 55/0 · it_task_run 31/0 · it_interactive 43/0
  aggregate_initialization_zero_git_side_effects 1/0
cargo clippy --locked --all-targets -- -D warnings ⇒ exit 0
cargo fmt --check ⇒ 干净
openspec validate --specs --strict ⇒ 78 passed / 0 failed
openspec validate globalize-provider-capability-verification --strict ⇒ valid
openspec validate root-recipe-provider-agnostic --strict ⇒ valid
```

**部署（4317 共享实例）**：8 项自检全过 —— 端口在听 · `/` 与 `/api/health` 200 · `governance_complete=true` · origin 白名单非空 · **运行 exe md5 == 新件 md5** · 指纹差异（`root_recipe_turn_zero_change` 新=1/旧=0）· 页面实际加载 `assets/index-Bk5msPU7.js`（== 重建后的 dist）· `GET /api/providers/capabilities` 直接可用。

**零项目验证（owner 核心诉求，controller 实测）**：一次性空工作区实例（`.aria` 下**无 projects 目录**）⇒ 侧栏「还没有 Project」时「Provider 管理」`disabled=false / cursor=pointer`；点开面板：能力卡 1 张、四家全「已确认」、「未核验」0、「未探测」0；顶部文案「…属**机器级**能力，与项目、逻辑代码库无关。」、按钮区「**机器级核验：核验只核验一次，机器一份事实**；会真实跑一次探测，可能几分钟，消耗 provider 额度。」

**独立审查**（k3-reviewer，通道 `my-anthropic/glm-5.3`）：5 个产品 commit 审完，**无 P0–P2 缺陷**；3 条 P3 文档漂移已清。

## 3. 尚未完成（全部是 owner 真机验收）

```
globalize 8.10② 在另一个项目/逻辑代码库打开面板 ⇒ 仍是同一份机器级事实
globalize 8.10③ 机器级核验一次后冷启动自动开始
globalize 7.10  跨 LC 共享（同上，语义已被 8.10② 覆盖）
globalize 7.11  冷启动不放假定文件也能自动开始（依赖 root-recipe 修复，已落地，待验）
globalize 7.12  放了两个判定文件 ⇒ 仍走零 provider turn 路径
root-recipe 4.7 重跑聚合初始化 ⇒ Pi 收到 /skill:pre-check 且真产生变更（或明确失败）
```
⇒ **两个 change 都还不能归档**（真机验收未勾）。

## 4. 两次数据丢失事故（如实记录）

| 时间 | 现象 | 定性 |
|---|---|---|
2026-11-05 11:16:34 | 侧栏垃圾桶一击直接硬删整个项目工作区（项目 + 全部 LC + 成员登记 + capability 记录），**无任何确认** | **owner 自述是自己点的**；当时的实现**确实没有二次确认** ⇒ 本批 `b99c4a3` 已补后端 409 强制 + 前端确认框 |
2026-11-05 15:35 | 同一形态再次发生 | owner 自述**也是自己删的**；当时部署的新后端已强制 `?confirmed=true` ⇒ **删除必带确认意图**。owner 反馈「没有弹窗」这一现象**仍未解释**（可能浏览器缓存着旧 bundle、或原生确认框闪得太快）⇒ **待复验** |

**机器级化的即时收益**：能力记录已移出 `projects/` ⇒ **项目被删不再影响能力状态**（两次事故后四家「已确认」都完好）。

## 5. 已知遗留（不阻塞）

- `lcg_t06_resume_probe_denies_when_cli_breaks_id_binding` **偶发**红：`Text file busy`（测试写完 fake CLI 脚本立刻执行，负载高时写句柄未关）⇒ 复跑 3 次全过；**与本批改动无关**，属该测试自身的老问题，另行立项。
- 3 处「看似跑根配方、实际不走零变更判定」的夹具（`web_lc_operations_api.rs:100` NoopProvider、`provider_gateway_envelope.rs:558`、`web_lc_gateway_multi_provider/harness.rs:4399` live matrix）⇒ **不构成隐患**，登记备查。
- 探针工件 `probe_artifact_ref` 指向机器级证据目录；升级前遗留的项目级证据目录成为孤儿（owner 裁定「没有旧的，不用管」）。

## 6. 边界声明

- **不并回 main**（owner 2026-11-05 裁定：继续在 `bugfix-b-11-02` 上推进）。
- `mcp_configuration` 步无子进程超时兜底 ⇒ owner 裁定**暂不做**。
- 共享实例删除源追查 ⇒ **关闭**（两次均已确认是 owner 自己操作）。
