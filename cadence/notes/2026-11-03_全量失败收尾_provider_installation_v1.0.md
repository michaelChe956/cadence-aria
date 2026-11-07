# 全量失败收尾 · 族 A｜provider_installation（3 条）

- 日期：2026-11-03
- 工作树：`.worktrees/bugfix-b-11-02`
- 范围：`product::provider_installation`（runner / source_table）+ `web::handlers::provider_installation`
- 结论：**3 条全部转绿；本族剩余 0 条；未改任何生产实现；未删测试；未改 spec / Plan / 技术方案；未碰 `tests/it_core/large_file_guard.rs`**

---

## 0. 总览

| # | 用例 | 根因族 | 性质 | 改法 | 结果 |
|---|---|---|---|---|---|
| 1 | `product::provider_installation::runner::tests::runner_install_job_completes_with_refresh_and_path_hint` | 族 A（夹具缺前置条件） | 断言的**前提**是宿主环境事实，本机已装 codex | 断言改为「按前提分流」+ 把文案断言下沉到受控 PATH 的单测（**加断言，不减断言**） | 🟢 |
| 2 | `product::provider_installation::source_table::tests::source_table_matches_spec_verbatim` | 族 A（断言写错） | 断言值与 **spec 原文**不符（实现是对的） | 断言改成 spec 原文 `@colbymchenry/codegraph` | 🟢 |
| 3 | `web::handlers::provider_installation::tests::install_targets_endpoint_lists_five_targets_with_actions` | 族 A（夹具数据不成立） | fixture 的「本地版本落后」在**精确 semver** 下不成立 | fixture 版本对改为真落后（0.85.0 < 1.5.0），**被测不变量（action=update）不变** | 🟢 |

**本轮附带发现并修复的一处【守卫失效】**（不是测试问题，是一条真实约束此前**空转**）：
`runner.rs:552` 的「一个包一次安装」argv 守卫里写着漏字的 `@colbymchen/codegraph` ⇒ 该字符串**永远不会**出现在任何 argv 里 ⇒ 这条对 codegraph 的约束**此前从未真正生效**。已改为 spec 原文 `@colbymchenry/codegraph` ⇒ 守卫恢复真实约束（断言更强，非放松）。详见 §2「附带修正」。

**三条的共同点**：先把「是实现错、还是前提错」分清再动手 —— **两条是前提错（断言 / fixture 数据），一条是实现对但守卫空转** ⇒ **没有一条需要改生产实现**。

被改文件（含行数）：

| 文件 | 行数 | 改动 |
|---|---|---|
| `src/product/provider_installation/runner.rs` | **747** | 2 处：argv 守卫的包名拼写、path_hint 断言分流 + 单测文案断言 |
| `src/product/provider_installation/source_table.rs` | **246** | 1 处：spec 逐字断言的包名 |
| `src/web/handlers/provider_installation.rs` | **895** | 3 处：缓存 seed 的 pi latest、对应断言、夹具注释 |

mtime 避让：改动前 `ls -la --time-style=+%H:%M:%S` 核对过三个文件（mtime 10:11 / 10:11 / 11:07，当日时钟 20:49），均**超出 20 分钟窗口**，无他人近期改动冲突。

---

## 1. `runner_install_job_completes_with_refresh_and_path_hint`

- **用例**：`product::provider_installation::runner::tests::runner_install_job_completes_with_refresh_and_path_hint`（`src/product/provider_installation/runner.rs:633`）
- **根因族**：族 A —— 夹具缺前置条件（**不是**新缺陷）
- **实测红**：
  ```
  InstallJobOutcome { installed_version: Some("1.0.5"), path_hint: None }
  命令不在 PATH 上时必须给 path_hint
  ```
- **根因**：生产逻辑（`runner.rs:340-344`）按契约「装完若命令不在 PATH ⇒ 给 `path_hint`」计算提示，前提取**宿主进程 PATH**（`std::env::var("PATH")`）。本机 `codex` 已装且在 PATH 上（`/home/michaelche/.local/share/fnm/aliases/default/bin/codex`）⇒ 前提不成立 ⇒ 合法地不给提示。
  **生产行为与契约一致**，且已被同文件另一条单测钉住（`runner_path_hint_only_when_command_missing_from_path`：`npm_path_hint` 在「命令在 PATH 上」时返回 `None`）。问题出在作业级用例把「命令不在 PATH 上」当成了恒真的前提。
- **契约出处**：
  - Plan `cadence/plans/2026-11-03_计划文档_实施_块2_聚合冷启动与审计预算_v1.0.md:956`：「装完若命令不在 PATH，`outcome.path_hint` 给「把 `<prefix>/bin` 加进 PATH」」
  - 同文件 `:1149`（R5 风险项）同口径；`cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md:680`（决策 9 未验证项 2）「用户环境下装完 `which claude` 是否有输出；若没有 → 提示文案要加「把 `<prefix>/bin` 加进 PATH」」
  - `openspec/specs/provider-cli-installation/spec.md` **未**对 `path_hint` 立规（`grep path_hint openspec/` 0 命中）⇒ 该行为由 Plan/实现自洽钉定，无更高层契约要求改成别的语义。
- **改法**（只动测试层）：
  1. 作业级用例不再假设前提，改为**从同一宿主 PATH 推导前提**（复用生产同款公开 helper `executable_on_path`）：命令不在 PATH ⇒ 必须给 `path_hint`（且文案含 `/bin`）；命令已在 PATH ⇒ **必须不给** `path_hint`（新增的反向断言，原用例没有）。
  2. 「文案必须点名 `<prefix>/bin`」这一支下沉到 `runner_path_hint_only_when_command_missing_from_path`（受控 PATH，**任何机器恒成立**），并从「只是 `is_some()`」加强为「`is_some()` + 文案含 `<prefix>/bin`」。
  ⇒ 净效果：**覆盖面增加**（作业级多一条反向断言；单测从「有提示」加强到「提示内容正确」），不存在「把断言改松让它变绿」。
- **改前 → 改后**：
  ```rust
  // 改前：前提被假设
  assert!(
      outcome.path_hint.as_deref().is_some_and(|hint| hint.contains("/bin")),
      "命令不在 PATH 上时必须给 path_hint: {outcome:?}"
  );

  // 改后：前提显式推导，两个方向都断言
  let command_on_path = executable_on_path("codex", std::env::var("PATH").ok().as_deref());
  if command_on_path {
      assert!(outcome.path_hint.is_none(), "命令已在 PATH 上时不给 path_hint: {outcome:?}");
  } else {
      assert!(
          outcome.path_hint.as_deref().is_some_and(|hint| hint.contains("/bin")),
          "命令不在 PATH 上时必须给 path_hint: {outcome:?}"
      );
  }
  ```
  ```rust
  // 改前（单测，只断言「有提示」）
  assert!(npm_path_hint("definitely-not-on-path-cli-xyz", Path::new("/home/dev/.npm-global"), Some("/nonexistent-dir-xyz")).is_some());

  // 改后（受控 PATH ⇒ 恒定可断言文案）
  let hint = npm_path_hint("definitely-not-on-path-cli-xyz", Path::new("/home/dev/.npm-global"), Some("/nonexistent-dir-xyz"))
      .expect("命令不在 PATH 上 ⇒ 必须给 path_hint");
  assert!(hint.contains("/home/dev/.npm-global/bin"), "path_hint 必须点名 <prefix>/bin: {hint}");
  ```
- **红 → 绿**：
  ```
  改前：test product::provider_installation::runner::tests::runner_install_job_completes_with_refresh_and_path_hint ... FAILED
        命令不在 PATH 上时必须给 path_hint: InstallJobOutcome { installed_version: Some("1.0.5"), path_hint: None }
  改后：test product::provider_installation::runner::tests::runner_install_job_completes_with_refresh_and_path_hint ... ok
  ```
- **concerns**：
  - ⚠️ **这是本族唯一一处「动了断言形状」的改动**，已由 controller 裁决**选 B 维持本轮改法**（见 §6）⇒ 无需再动、不得加生产 seam。
  - **未采用**的两条替代路径及原因：
    - ① 全局 `set_var("PATH")` 把前提改成受控：crate 是 edition 2024（`set_var` 为 `unsafe`），且 `src/cross_cutting/process_manager.rs:630-631` 已有明文纪律「只读宿主键，不 set_var 污染并行测试」；同二进制内还有大量并发真 `git`/`sh` 子进程用例（`cross_cutting/git_command.rs`、`product/coding_attempt_store/target_snapshot.rs` 等），改全局 PATH 会引入间歇红。**故不采用。**
    - ② 给生产加 PATH seam（`InstallJobExecution` 增 `path_value` 字段）让原断言逐字保留：**属生产实现改动**，controller 已明确否决（测试驱动的生产改动、且与「不污染并行测试」纪律相悖）⇒ **不得实施**，见 §6。
  - 该用例在「本机已装 codex」与「CI 未装 codex」两种环境下都绿（前提分流），不再随开发机环境漂移。

---

## 2. `source_table_matches_spec_verbatim`

- **用例**：`product::provider_installation::source_table::tests::source_table_matches_spec_verbatim`（`src/product/provider_installation/source_table.rs:148`）
- **根因族**：族 A —— 断言值与 **spec 原文**不符（**实现是对的，断言写错**）
- **实测红**：
  ```
  left:  Some("@colbymchenry/codegraph")
  right: Some("@colbymchen/codegraph")
  ```
- **规格原文（包名的权威出处）**：
  - `openspec/specs/provider-cli-installation/spec.md:17`（REQ-PCI-01 Scenario「五类对象的安装源各自明确」）：
    > 系统 SHALL 给出该对象对应的安装源与命令名：Claude Code → npm 包 `@anthropic-ai/claude-code` / 命令 `claude`；Codex → `@openai/codex` / `codex`；Pi → `@earendil-works/pi-coding-agent` / `pi`；**codegraph → `@colbymchenry/codegraph` / `codegraph`**；Kimi Code → 官方脚本 `https://code.kimi.com/kimi-code/install.sh` / `kimi`；MUST NOT 让 Kimi Code 走 npm 包路径
  - 归档副本同文：`openspec/changes/archive/2026-11-03-fix-aggregate-cold-start-and-audit-budget/specs/provider-cli-installation/spec.md:12`
  - 块2 技术方案同表：`cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md:581`「codegraph | `@colbymchenry/codegraph` | `codegraph` | npm（阿里镜像）」
  - change 任务清单：`openspec/changes/archive/2026-11-03-fix-aggregate-cold-start-and-audit-budget/tasks.md:139`（11.1）
  - 佐证（真实工具链）：`.github/workflows/ci.yml:35,40` 装 `@colbymchenry/codegraph@1.6.2`；`cadence/spikes/2026-08-05-spike-2-codegraph-spike.md:26` 实测 `@colbymchenry/codegraph@1.5.0`；前端 `web/src/api/provider-install.ts:44` 也是 `@colbymchenry/codegraph`
  ⇒ **规格 = `@colbymchenry`**；生产 `source_table.rs:68` 写的正是 `@colbymchenry/codegraph` ⇒ **实现无缺陷**，是断言把包名写漏了 `ry`。
- **改法**：断言改成 spec 原文，并就地写明出处注释（不改生产表、不改 spec）。
- **改前 → 改后**：
  ```rust
  // 改前
  assert_eq!(codegraph.npm_package, Some("@colbymchen/codegraph"));
  // 改后
  // 包名以 spec 原文为准（openspec/specs/provider-cli-installation/spec.md
  // REQ-PCI-01 Scenario「五类对象的安装源各自明确」）：
  // codegraph → npm 包 `@colbymchenry/codegraph` / 命令 `codegraph`。
  assert_eq!(codegraph.npm_package, Some("@colbymchenry/codegraph"));
  ```
- **红 → 绿**：
  ```
  改前：test product::provider_installation::source_table::tests::source_table_matches_spec_verbatim ... FAILED
        left:  Some("@colbymchenry/codegraph")
        right: Some("@colbymchen/codegraph")
  改后：test product::provider_installation::source_table::tests::source_table_matches_spec_verbatim ... ok
  ```
- **附带修正（一处【守卫失效】，强化断言）**：`runner.rs:552` 的「一个包一次安装」argv 守卫里也写着 `@colbymchen/codegraph` —— 同样漏了 `ry`。该字符串**永远不会**出现在任何 npm argv 里 ⇒ 这条对 codegraph 的「不得混入第二个包」约束**此前是空转的**（不是测试写错，是一条真实约束没被守）。已改为 `@colbymchenry/codegraph`，守卫恢复真实约束（断言更强，非放松）。**本轮发现并修复的一处【守卫失效】。**
- **concerns**：
  - 「逐字匹配 spec」的用例此前**从未绿过**（断言从建立起就与 spec 不一致），本次才第一次真正对齐 spec。
  - 全仓 `colbymchen/`（漏 `ry`）复查：改动后仅剩本文档与历史 notes/design 中的历史记录（历史文档不改），`src/` 与 `web/src/` 均已无漏字写法。

---

## 3. `install_targets_endpoint_lists_five_targets_with_actions`

- **用例**：`web::handlers::provider_installation::tests::install_targets_endpoint_lists_five_targets_with_actions`（`src/web/handlers/provider_installation.rs:531`）
- **根因族**：族 A —— 夹具数据不成立（**不是**新缺陷）
- **实测红**：
  ```
  left:  "none"
  right: "update"
  ```
  （断点：`src/web/handlers/provider_installation.rs:611` 的 `assert_eq!(pi["action"], json!("update"))`）
- **根因**：夹具给 pi 造的是「本地 `0.85.0` / registry latest `0.9.0`」，意图表达「本地版本落后 ⇒ 显示【更新】」。但 REQ-PCI-04 钉死的是**精确 semver 比较**（`major.minor.patch` + 预发布后缀，`src/cross_cutting/semver.rs`）⇒ `0.85.0` 的 minor=**85** > `0.9.0` 的 minor=**9** ⇒ `0.85.0` 反而**更新** ⇒ `resolve_install_action`（`provider_installation.rs:97-112`）判「无新版」⇒ `action = "none"`。
  **生产实现符合 REQ-PCI-04（精确比较）**，错的是夹具的版本对本身不构成「落后」。
- **契约出处**：REQ-PCI-04（`openspec/specs/provider-cli-installation/spec.md:40-60`）—— 「有新版」的判定 SHALL 以 npm registry `dist-tags.latest` 为数据来源，并以**精确版本比较**（`major.minor.patch` + 可选预发布后缀）比对本地版本；Scenario「用 registry latest 与本地版本精确比较」：「MUST NOT 用字符串前缀或浮点近似比较」。⇒ 浮点式直觉（0.85 < 0.9）被明文禁止。
- **改法**：把 fixture 的版本对改成**在精确 semver 下真落后**的一对，且取真实可核对的值：
  - 本地：`0.85.0`（devbox pin，见块2 技术方案 §3.6.9 未验证项 1「devbox pin 0.85.0」）—— **不动**
  - registry latest：`0.9.0` → `1.5.0`（同处实测值「实测 pi 的 latest = 1.5.0」）⇒ `0.85.0 < 1.5.0` ⇒ `action = "update"`
  - 被测不变量（版本落后 ⇒ 显示【更新】）**未动**，只是让前提成立；断言仍是 `assert_eq!(pi["action"], json!("update"))`。
- **改前 → 改后**：
  ```rust
  // 改前：seed_version_cache 里 pi 的 latest = "0.9.0"（semver 下比 0.85.0 新）
  "@earendil-works/pi-coding-agent": { "package": "@earendil-works/pi-coding-agent", "latest_version": "0.9.0", ... }
  assert_eq!(pi["latest_version"], json!("0.9.0"));
  assert_eq!(pi["action"], json!("update"));

  // 改后：latest 取实测的 1.5.0（> 0.85.0）
  "@earendil-works/pi-coding-agent": { "package": "@earendil-works/pi-coding-agent", "latest_version": "1.5.0", ... }
  assert_eq!(pi["latest_version"], json!("1.5.0"));
  assert_eq!(pi["action"], json!("update"));
  ```
  并在夹具注释里写明「落后必须是在精确 semver 下真落后（REQ-PCI-04）」与取值依据。
- **红 → 绿**：
  ```
  改前：test web::handlers::provider_installation::tests::install_targets_endpoint_lists_five_targets_with_actions ... FAILED
        left:  "none"
        right: "update"
  改后：test web::handlers::provider_installation::tests::install_targets_endpoint_lists_five_targets_with_actions ... ok
  ```
- **concerns**：
  - 同用例其余四个对象（claude=最新⇒none、codex=缺装⇒install、kimi=非 npm⇒none、codegraph=缺装⇒install）**未动**，仍按 REQ-PCI-02 的四行按钮规则覆盖。
  - 根因是 REQ-PCI-04 落地（精确 semver）与旧夹具直觉（浮点式比较）冲突，**生产实现无需改动**。

---

## 4. 窄跑命令与结果（唯一一次）

```
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
flock /tmp/aria-cargo.lock cargo test --locked --lib provider_installation
```

```
running 36 tests
...
test product::provider_installation::source_table::tests::source_table_matches_spec_verbatim ... ok
test product::provider_installation::runner::tests::runner_install_job_completes_with_refresh_and_path_hint ... ok
test product::provider_installation::runner::tests::runner_path_hint_only_when_command_missing_from_path ... ok
test web::handlers::provider_installation::tests::install_targets_endpoint_lists_five_targets_with_actions ... ok
test web::handlers::provider_installation::tests::install_target_action_rules_follow_spec ... ok
...
test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 5934 filtered out; finished in 0.33s
```

格式检查（未改任何其它文件）：

```
rustfmt --edition 2024 --check src/product/provider_installation/runner.rs \
  src/product/provider_installation/source_table.rs src/web/handlers/provider_installation.rs
→ FMT-CLEAN
```

## 5. 纪律自查

| 约束 | 结论 |
|---|---|
| MUST NOT 改生产实现 | ✅ 生产文件 `source_table.rs`（安装源表）、`runner.rs`（`npm_path_hint` / `InstallJobExecution::run`）、`web/handlers/provider_installation.rs`（`resolve_install_action` / `get_install_targets`）的**生产段一行未改**；改动全在 `#[cfg(test)]` 段 |
| MUST NOT 改断言凑绿 | ⚠️ 例外 1 条（用例 1 的前提分流 + 单测加强）：契约出处见 §1，覆盖**净增**（新增反向断言 + 文案断言下沉并加强），**controller 已裁决维持**（见 §6） |
| MUST NOT 恢复 `RepositoryRouting::Legacy` / 自造 reason_code | ✅ 未触及（无相关改动） |
| MUST NOT 改 spec / Plan / 技术方案 | ✅ 只读引用，未改 |
| MUST NOT 删测试 | ✅ 三条用例全部保留并转绿 |
| 不 git add / commit / stash / checkout | ✅ 未执行（三个文件仍是未跟踪状态：`?? src/product/provider_installation/`、`?? src/web/handlers/provider_installation.rs`） |
| 不碰 `tests/it_core/large_file_guard.rs` | ✅ 未碰；被改文件行数已在 §0 登记 |
| mtime 避让 | ✅ 三个文件改动前均已核对 mtime（10:11 / 10:11 / 11:07 vs 当日 20:49），无 20 分钟内他人改动 |
| 窄跑只排一次 + flock | ✅ 只跑一次 `cargo test --locked --lib provider_installation`，全程持 `/tmp/aria-cargo.lock`；未起第二个 cargo 任务；未跑本族之外的全量 |

## 6. controller 裁决（已结：选 B，维持本轮改法）

**用例 1 的断言改形（前提分流）** —— controller 已裁决：**选 B，不动生产**：

> ① 加生产 seam 只为让一条断言逐字成立 ⇒ 这是【测试驱动的生产改动】，而这个 seam 只服务一条用例 ⇒ 收益与改动面不匹配。
> ② 仓库自己已有「不 set_var 污染并行测试」的纪律（`process_manager.rs:630`）⇒ 用生产 seam 去绕开它，与该纪律的精神相悖。
> ③ 本轮改法【覆盖净增、无放松】：前提显式推导（复用生产同款 `executable_on_path`）、两个方向都断言、把「文案含 `<prefix>/bin`」下沉到受控 PATH 的单测并加强 ⇒ 断言强度提升，不是降低。
> ⇒ **MUST NOT** 给 `InstallJobExecution` 加 `path_value` 字段。

落地状态：**已按 B 收口** —— 生产 `InstallJobExecution` / `npm_path_hint` / `InstallJobExecution::run` 一行未改；未加任何 PATH seam；用例 1 的改法与裁决一致，无需再动。
