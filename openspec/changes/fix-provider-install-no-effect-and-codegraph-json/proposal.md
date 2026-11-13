# 提案：修复「页面点安装没有效果」与「聚合索引 codegraph 截断报错」

> 状态：草案 —— **本 change 只立项契约，不实施**；`tasks.md` 是待办清单，不是已完成记录。
> 依据：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（提交 `c2929b02` / `d06c1d16` / `8b9012c9`）。
> Owner 决策（2026-11-13，见 `.openspec.yaml` 的 `x-owner-decisions`）：D1/D2 装完判成功 + 平台命令可见 PATH overlay；D3 64 MiB 专用上限 + `--no-metadata`；D4 前端 `path_hint` 原样渲染。

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

## 非目标

| # | 非目标 |
|---|---|
| 1 | **不改主规格**：`openspec/specs/**` 本轮不动，归档时由 delta 自动同步 |
| 2 | **不触碰四家 provider 目录**：`src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/**` 零改动（其中无任何安装逻辑） |
| 3 | **不改版本门**：codegraph 支持区间维持实现现状（`>=1.6.0, <2.0.0`）；主规格 REQ-IND-01 里「exact version 钉定 v1.5.0」与实现的既有漂移**本轮不修**，归档同步时由双审对齐 |
| 4 | **不改用户 shell profile**：平台不自动把 CLI 暴露到用户终端 PATH（「装好后点一下做 symlink 暴露」是待裁决议题，方案 §十 未决点 10） |
| 5 | **不改冷启动致命/降级分界**：工具缺失（`codegraph_missing` / `codegraph_version_mismatch`）的致命与降级语义不动；截断码**不进入**致命匹配集 |
| 6 | **不做 agent 专属环境变量注入**（`PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR` 等）：本轮只做 PATH 合并，变量注入另立议题 |
| 7 | **不处理逐文件软链农场当 npm prefix ⇒ npm `EEXIST`**（诊断布局 B2）：前缀候选收紧后概率进一步下降，但本轮显式排除 |
| 8 | **不处理服务重启后 stale `running` 安装作业无对账**：既有边缘项，另开 issue |
| 9 | **不改 npm 镜像源策略**：默认阿里镜像 + `NPM_REGISTRY` 覆盖保持不变 |
| 10 | **不采纳 magpie 的「不代跑安装」做法**：owner 2026-11-13 明确 —— magpie 这一点不作为参照，目标不变，产品内一键安装必须真的装上并能用 |

## 受影响 capability

| capability | 变更 |
|---|---|
| `provider-cli-installation` | MODIFIED REQ-PCI-02（探针口径含平台登记目录）、MODIFIED REQ-PCI-03（可写 = 真实写入探测）、MODIFIED REQ-PCI-05（装完判定口径 / 提示原样呈现 / Kimi 目录可识别）、ADDED REQ-PCI-08（「不在终端 PATH 上」显式状态） |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-01（专用抓取配额 + 专用截断码）、MODIFIED REQ-IND-04（截断属构建失败路径 ⇒ 保留 last-known-good 并标 degraded）、MODIFIED REQ-IND-05（截断码不是工具缺失码、不进入致命匹配集；正文语义不改，只补钉死边界的场景） |

## 证据出处

| 证据 | 出处 |
|---|---|
| 方案文档（唯一权威来源） | `cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（`c2929b02` / `d06c1d16` / `8b9012c9`） |
| 容器基线复现 | podman `localhost/diag-installfix:latest`（五类布局：npm 前缀不在 PATH、Kimi 官方脚本、fnm 软链农场、逐文件软链农场、root 拥有的 `/usr/local`） |
| 聚合索引截断复现 | 真实 codegraph 1.6.2 的 `files --json`（2.36 MB）+ 仓库同版本 serde_json 解析 ⇒ 逐字复现 `EOF while parsing a string at line 32565 column 48` |
| `--no-metadata` 载荷不变 | 容器实测 1.6.0 / 1.6.2：`files --json` 与 `files --json --no-metadata` 输出逐字节相同（每项仍含 `path/language/nodeCount/size`） |
| 前端二次包裹 | `web/src/components/lifecycle/ProviderInstallActionButton.tsx`（修复前 `:290-292`；修复已提交 `6cd5075e`） |
| 致命匹配集 | `src/product/logical_codebase/bootstrap_projector.inc.rs:104`（只含 `codegraph_missing` / `codegraph_version_mismatch`） |
