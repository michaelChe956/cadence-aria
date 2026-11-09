# 设计：Kimi Code 也支持「更新」

## 1. 事实核对（实测，2026-11-09）

| 事实 | 取证 | 对设计的含义 |
|---|---|---|
| Kimi Code 的 npm 包名是 `@moonshot-ai/kimi-code` | `registry.npmmirror.com/@moonshot-ai/kimi-code` → `dist-tags.latest = 2.5.2`；官方 registry 同值 | 版本判定可直接复用现成的 registry 查询机制 |
| 官方 `/latest` 端点与 npm latest 同版本 | `GET https://code.kimi.com/kimi-code/latest` → `2.5.2` | 两个来源一致，判定口径无歧义 |
| 官方安装脚本支持「不指定版本 = 取最新」 | `install.sh`：`KIMI_VERSION` 未设时拉 `${KIMI_DOWNLOAD_BASE}/latest` | 重跑脚本 = 原地升级 ⇒ 更新动作不需要新执行面 |
| 官方安装脚本原地替换并留备份 | `install.sh`：`cp ${KIMI_INSTALL_DIR}/bin/kimi ${KIMI_INSTALL_DIR}/bin/kimi.bak` 后 `install -m 0755` | 升级可回退；默认 `KIMI_INSTALL_DIR=$HOME/.kimi-code` 与本机一致 |
| 官方脚本自己也知道 npm 包 | 脚本在 musl/Alpine 上提示改用 `npm install -g @moonshot-ai/kimi-code` | npm 与脚本是官方并行的两条分发路径，版本号同源 |
| `kimi update`（= `kimi upgrade`）存在且在 `--help` 命令列表里 | `kimi --help` → `upgrade|update [options] Upgrade Kimi Code to the latest version.`（`-y` 跳过确认） | 可作为**手动兜底命令**写进失败提示，但**不作为平台的执行路径**（无 dry-run；与「先告知来源再确认」耦合弱） |
| 本机 kimi = 2.4.1，最新 2.5.2 | `kimi --version` / npm latest | 存在真实差距，需求成立 |

## 2. 设计取舍

### 2.1 版本来源与安装来源分离（核心改动）

现状：`InstallSourceRow { npm_package, install_script }` 一个字段同时管两件事。

| 选项 | 结论 |
|---|---|
| A. 新增 `version_package` 字段（安装源不变，版本来源单列） | ✅ 采用 |
| B. 把 Kimi 的 `npm_package` 填成 `@moenshot-ai/kimi-code` | ❌ 会让 Kimi 走 npm 安装路径，违反 REQ-PCI-01 |
| C. 新增一个「非 npm 版本源」类型（抓 `/latest` 纯文本端点） | ❌ 多一套拉取器与缓存口径，而 npm 包名已有同版本号且现成机制可用 |

采用 A：字段语义单一（`npm_package` = 安装源、`version_package` = 版本来源），
npm 类对象两者同值，Kimi 只有后者。

### 2.2 Kimi 的更新动作：重跑官方脚本，不调 `kimi update`

- 官方脚本是**已文档化**的安装路径（REQ-PCI-01 已承载其「来源告知 + 确认」口径），
  重跑即原地升级 ⇒ 不新增执行面、不新增确认面、不新增失败分类。
- `kimi update` 是 CLI 自己的实现：无 dry-run（无法先查后装）、无版本锁、
  且确认语义在我们这层之外（`-y` 直接跳过）⇒ 只作为失败提示里的手动兜底命令。
- 版本不锁死：脚本自行解析 `/latest` ⇒ 装到的就是真正的最新（与 UI 显示的
  cached latest 可能差一两个版本，方向一致且更新）。

### 2.3 安全口径不变

Kimi 的【更新】与【安装】走**同一个**前端确认框：
「将执行来自 code.kimi.com 的安装脚本（远程 shell 脚本）」，
后端 `POST /api/providers/kimi_code/install` 仍要求
`acknowledge_remote_script=true`（无确认直接 400）。

### 2.4 前端按钮规则

`ProviderInstallActionButton` 现状把 Kimi 排除在【更新】之外
（`target !== "kimi_code" && capabilityConfirmed && versionGap < 0`）。
改为只按「已核验 + 版本落后」判定（与其它四类同口径）；
Kimi 的版本比较走既有的 `compareVersions`（2.4.1 < 2.5.2）。

## 3. 契约映射（delta → 实现）

| delta 条款 | 实现落点 |
|---|---|
| REQ-PCI-01 MODIFIED（来源告知覆盖更新） | `ProviderInstallActionButton` 的脚本告知文案（安装/更新）+ 后端 `acknowledge_remote_script` 校验（既有） |
| REQ-PCI-04 REMOVED + REQ-PCI-07 ADDED（版本来源按对象声明 / Kimi 不再豁免版本判定） | `source_table.rs` 新增 `version_package`；`provider_installation.rs` 的 latest 查询改用版本来源；`resolve_install_action` 保持通用 |
| REQ-PCI-05 MODIFIED（Kimi 更新复用官方脚本） | `runner.rs` 的 `kimi_install_plan`（既有，重跑即升级）+ REQ-PCI-05 的装完重探（既有） |

> **条款形态说明**：`openspec validate --strict` 规定 MODIFIED 块必须保留主规格里
> 该 requirement 的**全部既有 scenario 名**（MODIFIED 替换整块，archive 拒绝丢场景）。
> REQ-PCI-04 的「Kimi 不做版本判定」场景与新语义**直接冲突**，无法保留 ⇒
> 采用 REMOVED REQ-PCI-04（带 Rationale 说明由 REQ-PCI-07 承接）+ ADDED REQ-PCI-07
> （逐条承接精确比较 / 预发布 / 24h durable 缓存 / 缓存条目字段）。

## 4. 验证口径

- 单元：安装源表（版本来源字段值）、handler（Kimi 落后 → `action=update`）、
  前端（Kimi 版本落后 → 渲染【更新】+ 当前/最新版本；未确认来源时不发 API）。
- 端到端：真服务 + 内置浏览器，Kimi 行显示【更新】→ 点击 → 确认来源 →
  作业 `completed` → `kimi --version` 变为 2.5.2 → 「版本落后」标签消失。

## 5. 不做（明确排除）

- 不把 Kimi 的安装源改成 npm（REQ-PCI-01 不动）。
- 不新增 `kimi update` 执行路径（只写进失败提示的手动兜底命令）。
- 不做跨 provider 的批量更新（块3 批量的既有语义不变）。
- 不改 REQ-PCI-03（npm 镜像/前缀形态）、REQ-PCI-06（失败分类与手动命令口径）。