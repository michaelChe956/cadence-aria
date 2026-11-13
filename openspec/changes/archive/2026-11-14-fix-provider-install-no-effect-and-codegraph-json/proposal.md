# 提案：修复「页面点安装没有效果」与「聚合索引 codegraph 截断报错」

> 状态：契约已获批并**已实施**（2026-11-13）；`tasks.md` 记录工作包与其实施状态（`[x]` 已落地 / `[ ]` 待办），未闭环项为 **OpenSpec 归档同步**（主规格漂移对齐 + CHANGELOG 留痕）与**容器 E2E 复验**；终审遗留项（第 13 包）**已由本 change 的修复波落地**，逐条附实现证据（提交 / 红测试）。
> 依据：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（提交 `c2929b02` / `d06c1d16` / `8b9012c9`）。
> Owner 决策（2026-11-13，见 `.openspec.yaml` 的 `x-owner-decisions`）：D1/D2 装完判成功 + 平台命令可见 PATH overlay；D3 64 MiB 专用上限（额外旗标经 owner 裁决去掉：实测零收益 + 未来上游移除该旗标的风险）；D4 前端 `path_hint` 原样渲染；另增「更新语义」（owner 明确范围含更新）；另增「聚合索引成员覆盖验收改走 codegraph 库 API」（owner 2026-11-13 裁决）、「暴露到终端 PATH」显式动作（owner 2026-11-13 裁决）与「agent 专属环境变量补进子进程」（owner 2026-11-13 裁决「本轮做」，方案 §五 2.12）。

## Why（为什么现在做）

三个用户现场缺陷互相独立，但都落在同一条链路上：**平台自己的命令可见性与平台的判断口径不一致**，用户点一次按钮看不到任何变化。

| # | 症状 | 现场证据 |
|---|---|---|
| 1 | 本地没装四家 provider（Claude Code / Codex / Pi / Kimi Code）中的任意一个或多个，点【安装】**没有效果，从未装成功** | 用户报告 |
| 2 | 本地没装 codegraph，点【安装】**没有效果** | 用户报告 |
| 3 | 聚合索引 degraded，错误 `aggregate_index_failed:codegraph_files_invalid_json: parse JSON from codegraph files: EOF while parsing a string at line 32565 column 48` | 用户截图 |

### 容器实测复现结论（不是推理）

复现环境：podman `localhost/diag-installfix:latest`（真实 codegraph 1.6.2 / node:24-bookworm），服务 PATH 刻意不含 npm 全局 bin（`PATH=/shim:/usr/bin:/bin`，`npm prefix -g=/opt/node`）。

| 布局 | 复现结果 |
|---|---|
| npm 类（codegraph / claude-code / codex） | `npm install -g --prefix /opt/node …` **exit 0**，二进制落到 `/opt/node/bin/<cmd>`；但装后探测 `env: 'codegraph': No such file or directory`（127）⇒ 作业 completed、`installed_version` 为空、`GET /api/providers/install-targets` 的 `cli_available` 仍为 false ⇒ 界面仍显示「未安装」⇒ **用户点一次没效果、再点一次再没效果** |
| Kimi 官方脚本 | 脚本输出 `Installed to /root/.kimi-code/bin/kimi` + `Added /root/.kimi-code/bin to PATH in /root/.bashrc` ⇒ **只改 shell rc**，运行中的服务进程 PATH 不变 ⇒ 探测 127 ⇒ 必然「没有效果」，且**没有任何提示**（`path_hint` 缺失） |
| 聚合索引 | `codegraph files --json` 真实输出 **2.36 MB**；被 1 MiB 抓取上限截断后 ⇒ `codegraph_files_invalid_json`（实测逐字复现用户截图里的 `EOF while parsing a string at line 32565 column 48`） |

### 根因（代码事实）

**根因 A —— 聚合索引：输出被 1 MiB 上限截断，且截断事实被丢弃、伪装成协议异常**

| 环节 | 位置 | 事实 |
|---|---|---|
| 上限 | `src/product/logical_codebase/aggregate_index/codegraph_cli.rs:39` | `const OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;`，套给**所有** codegraph 调用 |
| 截断 | `src/cross_cutting/bounded_command_runner.rs:191`（`read_limited`） | 恰好保留 `limit` 字节、丢弃其余并置 `stdout_truncated`（达到上限后继续读取并丢弃 ⇒ 子进程不会因管道阻塞而挂死 ⇒ 现象是「exit 0 但 JSON 解析失败」） |
| 丢标志 | `codegraph_cli.rs` 的 `fn command_result` | 只取 `stdout/stderr/duration_ms`，`stdout_truncated` 被丢掉 |
| 误报 | `codegraph_cli.rs` 的 `files()` 解析分支 | 截断文本喂 `serde_json::from_str` ⇒ `codegraph_files_invalid_json`（伪装成「上游协议不符」，把排障方向带偏） |

**根因 A2 —— 聚合索引：验收方式与仓库规模不匹配（索引是好的，平台读不出来）**

| 环节 | 位置 | 事实 |
|---|---|---|
| 验收方式 | 聚合索引成员覆盖验收 | 用 `codegraph files --json` 拉**整个仓库的文件清单**再全量读进内存判成员覆盖 |
| 载荷 | 实测 | 每文件约 110~200 B；`codegraph.db` : JSON ≈ **13 : 1** |
| 规模 | 用户现场 | **3.1 GB** 索引 ⇒ 清单约 **230 MB** |
| 结论 | 抓取上限（1 MiB / 64 MiB） | **无论如何都装不下** ⇒ 成员覆盖验收**必失败**（现场报 `codegraph_files_invalid_json`，把「读不出来」伪装成「上游协议不符」） |

**根因 B —— 安装链：装到的目录不在服务进程 PATH 上 ⇒ 装完探测仍失败 ⇒ 界面仍显示「未安装」**

| 环节 | 位置 | 事实 |
|---|---|---|
| 前缀回落 | `src/product/provider_installation/runner.rs:397`（`resolve_effective_prefix`） | CLI 缺失时回落到 `npm prefix -g`；**不检查**该前缀的 `bin` 是否在 PATH 上 |
| 前缀探测 | `src/product/provider_installation/effective_prefix.rs:38`（`visible_cli_prefix`） | 只认「PATH 上已有该命令」的前缀（CLI 缺失时此路不通） |
| 装后探测 | `runner.rs:202`（`HealthRefreshProber::refresh_after_install`） | codegraph 分支跑 `codegraph --version`（按 PATH 查找）；四家 provider 走 `provider_health.refresh()`（同样只按 PATH 找） |
| 提示缺失 | `runner.rs:245`（`npm_path_hint`） | 只对 `row.npm_package.is_some()` 产出 ⇒ **Kimi 完全没有提示** |

**根因 C —— 安装链次因：可写性判定只看权限位 ⇒ 选到根本写不进去的系统前缀**

`effective_prefix.rs:61`（`is_writable_dir`）→ `:74`（`dir_is_writable`）只看 `mode & 0o222`。root 拥有的 `/usr/local`（0755）⇒ 对任意用户都判「可写」⇒ npm 报 `EACCES … mkdir '/usr/local/lib/node_modules/@colbymchenry'` ⇒ `$HOME/.npm-global` 兜底永远走不到。容器实测同点 `os.access(W_OK) → False`。

**根因 D —— 放大器：前端把后端整句提示再包一层**

后端 `path_hint` 本身已是完整句子（「已安装到 /opt/node；若 `codegraph` 不在 PATH 上，请把 /opt/node/bin 加入 PATH 后重试」），前端又拼成「…；若命令不在 PATH，可把 <整句> 加入 PATH」⇒ 用户看到破碎双句，更像「没效果」。

**已排除的机制（有执行证据）**：npm 子进程缺 HOME / 环境不继承（`src/cross_cutting/process_manager.rs` 的 `spawn_with_environment` 已无 `env_clear`，子进程继承宿主环境，`request.environment` 只是覆盖层）；按钮不渲染 / 请求丢失 / 作业卡死（路由与前端调用链完整、作业落盘后才 spawn）。

## 范围

| # | 范围项 |
|---|---|
| 1 | **聚合索引抓取配额专用化**：`codegraph files --json` 走有文档上限的专用配额（64 MiB），stdout 被截断时以专用机器码 `codegraph_files_output_truncated` 报告，不再伪装成 `codegraph_files_invalid_json` |
| 2 | **平台命令可见 PATH**：装完 CLI 所在的 bin 目录纳入「平台自身命令可见 PATH」（共享 PATH overlay，不改进程真实 env），装后探测立即可见 |
| 3 | **装后判定口径**：装完 CLI 不在宿主 PATH 上、但在平台登记目录内 ⇒ 视为「已安装、探针通过」；MUST NOT 在装完后仍显示「未安装」 |
| 4 | **前缀候选与可写性**：「可写前缀」的「可写」按真实写入探测判定（mode 预筛 + 实际创建/删除临时文件）；系统级不可写前缀 MUST NOT 被选中；平台 MUST NOT 提权 |
| 5 | **安装提示与状态**：Kimi 官方脚本的安装目录 SHALL 可被平台识别并用于装后探测与提示；`path_hint` 原样呈现（前端 MUST NOT 二次包裹）；新增「装了但不在终端 PATH 上」的显式状态 |
| 6 | **装后验证**：版本优先读 npm 包 `package.json`；除 `--version` 外 MUST 跑一个真实命令证明可执行；失败区分「没装上 / 装上但缺平台包 / 装上但不在 PATH」三种语义 |
| 7 | **npm 安装参数**：`--allow-scripts=<pkg>`；装完探测失败时以 `--include=optional` 重试一次 |
| 8 | **更新语义**（owner 2026-11-13 明确范围含更新）：更新 SHALL 装到**被更新对象当前所在的前缀**（已安装但不在 PATH 上时从已安装包真实路径反推 npm 前缀，MUST NOT 改装到别的前缀）；更新后版本未变 SHALL 明确报告「命令仍是旧版」，MUST NOT 静默成功；**更新前后的版本取数 SHALL 同源同口径**（口径不一致 ⇒ 「版本未变」恒不成立 ⇒ 该判定在生产上永不触发）；Kimi 更新沿用官方脚本原地替换并经装后验证 |
| 9 | **聚合索引成员覆盖验收改走库 API**（owner 2026-11-13 裁决，已实施）：成员覆盖判定经 codegraph **库 API**（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）逐成员目录级查询，载荷是成员级布尔事实（几百字节），MUST NOT 以全量文件清单为常规验收路径；目录边界 MUST 是目录语义；库 API 不可用（未安装 / 版本不兼容）时 MAY 回退既有 CLI 清单路径并留可审计痕迹，但 MUST NOT 把回退当常规路径；索引健康 SHALL 只告警不阻断激活 |
| 10 | **「暴露到终端 PATH」显式动作**（owner 2026-11-13 裁决，已实施）：用户**显式触发**时，只在「已存在 + 真实可写 + 确实在用户终端 PATH 上」的目录里创建**单个**软链；MUST NOT 修改用户的 shell 配置文件；失败 MUST 如实报告（含各候选目录的不合格原因） |
| 11 | **agent 专属环境变量补进子进程**（owner 2026-11-13 裁决「本轮做」，方案 §五 2.12；已实施）：启动期探测**交互式登录 shell**（2s 硬上限、不阻塞启动）取得 `PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR`，只在「调用方覆盖层与平台进程 env 都没有」时补进平台发起的子进程 env；探测失败/超时 SHALL 不阻塞任何流程 |

## 非目标

| # | 非目标 |
|---|---|
| 1 | **不改主规格**：`openspec/specs/**` 本轮不动，归档时由 delta 自动同步 |
| 2 | **不触碰四家 provider 目录**：`src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/**` 零改动（其中无任何安装逻辑） |
| 3 | **不改版本门**：codegraph 支持区间维持实现现状（`>=1.6.0, <2.0.0`）；主规格 REQ-IND-01 里「exact version 钉定 v1.5.0」与实现的既有漂移**本轮不修**，归档同步时由双审对齐 |
| 4 | **不改用户 shell profile**：平台 MUST NOT 自动把 CLI 暴露到用户终端 PATH；用户**显式触发**的「暴露到终端 PATH」动作按 REQ-PCI-11 执行（只在终端 PATH 上的可写目录建单个软链，仍不写 `.bashrc` / `.zshrc`） |
| 5 | **不改冷启动致命/降级分界**：工具缺失（`codegraph_missing` / `codegraph_version_mismatch`）的致命与降级语义不动；截断码**不进入**致命匹配集 |
| 6 | **不处理逐文件软链农场当 npm prefix ⇒ npm `EEXIST`**（诊断布局 B2）：前缀候选收紧后概率进一步下降，但本轮显式排除 |
| 7 | **不处理服务重启后 stale `running` 安装作业无对账**：既有边缘项，另开 issue |
| 8 | **不改 npm 镜像源策略**：默认阿里镜像 + `NPM_REGISTRY` 覆盖保持不变 |
| 9 | **不采纳 magpie 的「不代跑安装」做法**：owner 2026-11-13 明确 —— magpie 这一点不作为参照，目标不变，产品内一键安装必须真的装上并能用 |
| 10 | **不把排除性查询改走库 API**：非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引的负查询**保持既有 CLI 查询机制**（载荷小，本轮不动） |
| 11 | **不因索引健康问题阻断激活**：健康闸只产告警，不改「不因健康问题阻断」的既有语义 |
| 12 | **不为前端观察器轮询节律新立 capability**：终审要求的「dirty 补跑等待期内不得被 250ms 防抖顶掉」已由 `ea1434b1` 落地（含红测试），但该能力**归属待 owner 裁决**；本 change 只如实记录已落地的行为，不为它新立 capability，也不动主规格 |

> 说明：上表原有「agent 专属环境变量注入不在本轮范围」条目**已撤销**（owner 2026-11-13 裁决「本轮做」，方案 §五 2.12 授权实现），该行为现已实施并纳入本 change 范围（范围第 11 项，契约见 REQ-PCI-12）。

## 受影响 capability

`provider-cli-installation`：

- MODIFIED REQ-PCI-02（探针口径含平台登记目录）
- MODIFIED REQ-PCI-03（可写 = 真实写入探测；安装参数形态补 `--allow-scripts=<pkg>` 与 `--include=optional` 重试）
- MODIFIED REQ-PCI-05（装完判定口径 / 提示原样呈现 / Kimi 目录可识别 / 装后三重验证与绝对路径兜底探测）
- ADDED REQ-PCI-08（「不在终端 PATH 上」显式状态）
- ADDED REQ-PCI-09（更新落点与被更新对象一致）
- ADDED REQ-PCI-10（更新后版本校验与 Kimi 更新）
- ADDED REQ-PCI-11（显式「暴露到终端 PATH」动作）
- ADDED REQ-PCI-12（登录 shell 探测到的 agent 专属环境变量补进子进程）

`logical-codebase-aggregate-index`：

- MODIFIED REQ-IND-01（专用抓取配额 + 专用截断码；**成员覆盖验收改走库 API** 的逐成员目录级查询）
- MODIFIED REQ-IND-04（截断属构建失败路径 ⇒ 保留 last-known-good 并标 degraded；回退痕迹与健康告警随记录留存）
- MODIFIED REQ-IND-05（截断码不是工具缺失码、不进入致命匹配集；正文语义不改，只补钉死边界的场景）

## 证据出处

| 证据 | 出处 |
|---|---|
| 方案文档（唯一权威来源） | `cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（`c2929b02` / `d06c1d16` / `8b9012c9`） |
| 容器基线复现 | podman `localhost/diag-installfix:latest`（五类布局：npm 前缀不在 PATH、Kimi 官方脚本、fnm 软链农场、逐文件软链农场、root 拥有的 `/usr/local`） |
| 聚合索引截断复现 | 真实 codegraph 1.6.2 的 `files --json`（2.36 MB）+ 仓库同版本 serde_json 解析 ⇒ 逐字复现 `EOF while parsing a string at line 32565 column 48` |
| 额外旗标不改变载荷（据此去掉该旗标） | 容器实测 1.6.0 / 1.6.2：文件清单的 `--json` 调用在带与不带该额外旗标时输出逐字节相同（每项仍含 `path/language/nodeCount/size`）⇒ 零收益，且保留它只会引入「上游移除该旗标 ⇒ 文件清单全线失败」的风险 |
| 前端二次包裹 | `web/src/components/lifecycle/ProviderInstallActionButton.tsx`（修复前 `:290-292`；修复已提交 `6cd5075e`） |
| 致命匹配集 | `src/product/logical_codebase/bootstrap_projector.inc.rs:104`（只含 `codegraph_missing` / `codegraph_version_mismatch`） |
| 库 API 覆盖验收（已实施） | `src/product/logical_codebase/aggregate_index/codegraph_coverage.rs` + `codegraph_coverage_helper.mjs` + `operation.rs`（提交 `75f066e2` / `29766cb4`）：`CodeGraph.open(root)` + 逐成员 `hasFilesUnder`（目录边界精确）、自带 Node 优先（PATH node 需 ≥ 22.5）、SDK 缺失 / 不兼容 ⇒ 回退 CLI 清单路径并留审计 warning、健康闸只告警 |
| 「暴露到终端 PATH」（已实施） | `src/product/provider_installation/expose.rs` + `src/web/handlers/provider_installation.rs`（`POST /api/providers/{target}/expose`）+ `web/src/api/provider-install.ts`（提交 `1daf65b2` / `37f53aa4` / `29511974`） |
| agent 专属环境变量注入（已实施） | `src/cross_cutting/path_overlay.rs`（`SHELL_PROBE_VARS` 变量清单 `:155`、`probe_login_shell_env` 交互式 login 探测与 2s 硬上限、`shell_env_overrides` 合并优先级）+ 两个汇聚点 `src/cross_cutting/process_manager.rs`（`spawn_with_environment`）与 `src/cross_cutting/cli_adapter.rs`（`run_command_capture_with_stream`） |
| 「不在终端 PATH 上」显式状态（已实施） | `src/web/handlers/provider_installation.rs`（`off_path` DTO 字段与 `resolve_off_path`）+ `web/src/api/types/provider.ts` + `web/src/components/lifecycle/ProviderCapabilityCard.tsx`（徽标；提交 `d7d759c7` / `621d3269`） |
| 终审遗留项修复波（已实施） | `tasks.md` 第 13 包逐条括注：`0f69fe90`（真实命令参与完成判定 / 更新落点 visible 优先 / codegraph 改 `status --json` 验证 / Kimi 同版本判定）、`f9c776d8`（自带 node 三布局）、`84675998`（Node helper 只读打开索引）、`55296a8f`（overlay 合并序 Prepend→base→Append + 存储实现可注入）、`ada50bca`（off_path/expose 查找序并入服务进程 PATH + 恢复被删断言）、`ea1434b1`（观察器 dirty 补跑独立槽位）、`f165474e` / `60b2e064`（expose 入口对「已安装未核验」行可达） |
