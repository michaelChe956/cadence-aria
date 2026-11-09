# 提案：Kimi Code 也支持「更新」（版本来源与安装来源分离）

## Why

平台已有 provider / codegraph 的安装与更新能力，但 **Kimi Code 永远不会出现【更新】按钮**：
`source_table` 用同一个字段 `npm_package` 同时表达「版本判定源」与「安装源」，
Kimi 走官方脚本（非 npm）⇒ `npm_package = None` ⇒ registry 查不到 latest ⇒
`latest_version = None` ⇒ 版本永远判不出落后。

现实后果（本机实测 2026-11-09）：kimi 已装 **2.4.1**，官方最新 **2.5.2**，
而官方 `/latest` 端点与 npm 上的 `@moonshot-ai/kimi-code`（`dist-tags.latest`）
都给出 2.5.2 —— **信息是公开可得的，只是平台没去取**。
用户在界面上看不到差距、拿不到更新入口，只能自己去跑 `kimi update`。

同时 REQ-PCI-01 的既有约束（Kimi 走官方脚本、执行前明示来源）**必须保持**：
Kimi 的**安装源**不能改成 npm。

## What Changes

- **版本来源与安装来源分离**：安装源表新增「版本来源包」字段。
  - Claude Code / Codex / Pi / codegraph：版本来源 = 各自的 npm 包（与安装源同包）。
  - **Kimi Code：版本来源 = npm 包 `@moonshot-ai/kimi-code`**（官方以该包名发布，
    与官方 `https://code.kimi.com/kimi-code/latest` 端点同版本号）；
    **安装源仍是官方脚本**（`install.sh`），REQ-PCI-01「Kimi 不走 npm 路径」不变。
- **Kimi 参与版本判定与按钮规则**：Kimi 已安装且版本落后时，界面显示【更新】+
  当前版本 vs 最新版本；已是最新则不显示任何按钮（与其它四类同口径）。
- **Kimi 的更新动作 = 重跑官方安装脚本**：脚本自身解析最新版本并原地替换
  `~/.kimi-code/bin/kimi`（旧件自动留 `kimi.bak`）⇒ 安装与更新同一条通道、
  同一份「来自 code.kimi.com」的来源告知与用户确认。
- **不新增远程执行面**：不引入 `kimi update` 子命令作为执行路径（官方 CLI 有
  `kimi update`，但它是 CLI 自己的实现、无 dry-run、且与「先告知来源再确认」
  的口径耦合更弱）；版本判定只用**只读**的 registry 查询。

## Capabilities

### New Capabilities

- 无：本 change 不新建 capability，只修改既有 capability 的 requirement。

### Modified Capabilities

- `provider-cli-installation`：
  - MODIFIED REQ-PCI-01（Kimi 的来源告知覆盖「安装**与更新**」两种触发）。
  - REMOVED REQ-PCI-04 + ADDED REQ-PCI-07（版本来源按对象声明；Kimi 用 npm 包
    `@moonshot-ai/kimi-code` 判定「有新版」，删除「Kimi 不做版本判定」这一约束；
    其余条款语义逐条承接）。
  - MODIFIED REQ-PCI-05（Kimi 的更新复用官方脚本通道，原地升级）。

## Impact

- **契约层**：`openspec/specs/provider-cli-installation/spec.md` 的上述 delta；
  本 change 不改主规格文件（归档时合并）。
- **实现面**：
  - `src/product/provider_installation/source_table.rs`：安装源表新增「版本来源包」字段。
  - `src/web/handlers/provider_installation.rs`：`install-targets` 的 latest 查询改用版本来源。
  - `web/src/components/lifecycle/ProviderInstallActionButton.tsx`：Kimi 不再被排除在【更新】之外，
    来源告知文案覆盖「安装/更新」。
  - runner / job / 失败分类**不改**：Kimi 的执行计划本来就是官方脚本，重跑即升级。
- **数据兼容**：版本缓存键 = 版本来源包名（Kimi 新增一个键），
  旧缓存条目按包名命中，不受影响；24 小时 durable 缓存语义不变。
- **风险与边界**：Kimi 的 npm 包名与官方二进制版本号当前一致（实测同为 2.5.2）；
  若官方将来让两者分叉，版本判定会失真 —— 以 `dist-tags.latest` 为准并保留
  「装完重新探测」的既有事实刷新（REQ-PCI-05），不做额外推断。
