# 任务清单：Kimi Code 也支持「更新」

> 每个工作包独立可验收；实现走 TDD（先红后绿）。契约见 `specs/provider-cli-installation/spec.md`
> （MODIFIED REQ-PCI-01 / REQ-PCI-05；REMOVED REQ-PCI-04 + ADDED REQ-PCI-07），设计取舍见 `design.md`。

## 1. 契约与计划

- [x] 1.1 评审并确认 delta（REQ-PCI-01 / 04 / 05）与 design 的取舍
- [x] 1.2 `openspec validate kimi-provider-update-support --strict` 通过
- [x] 1.3 Plan 写入 `cadence/plans/2026-11-09_实施计划_kimi支持更新.md`（文件 / 步骤 / 命令 / 测试）

## 2. 后端：版本来源与安装来源分离

- [x] 2.1 失败测试：`source_table` 断言五类对象的**版本来源包**（npm 四类 = 各自包；
      Kimi = `@moonshot-ai/kimi-code`）且 **Kimi 的安装源仍是官方脚本**（`npm_package` 仍为 None）
- [x] 2.2 实现：`InstallSourceRow` 新增 `version_package` 字段并填表；
      `npm_package` 语义收敛为「安装源」，两字段各自单测
- [x] 2.3 失败测试：handler `install-targets` 对 Kimi 返回 `latest_version` 且
      本地落后时 `action = "update"`（用既有 durable 版本缓存打桩，不打真网络）
- [x] 2.4 实现：handler 的 latest 查询改用**版本来源**；`resolve_install_action` 保持通用
- [x] 2.5 回归：`cargo test --locked --lib provider_installation` 全绿
      （含既有「Kimi 不做版本判定」相关断言的同步更新）

## 3. 前端：Kimi 的【更新】按钮

- [x] 3.1 失败测试：`ProviderInstallActionButton` 在 Kimi 版本落后时渲染【更新】
      并展示当前/最新版本；未确认远程脚本来源时不发起 POST
- [x] 3.2 实现：去掉 Kimi 的更新排除；来源告知文案覆盖「安装/更新」
- [x] 3.3 回归：`pnpm -C web test`（全量）绿；`pnpm -C web build` 通过

## 4. 端到端验收（真服务 + 内置浏览器）

- [x] 4.1 发布态构建：`pnpm -C web build` → `CARGO_TARGET_DIR=/tmp/deploy-target cargo build --release --locked`
- [x] 4.2 经 `bash /tmp/aria-deploy/start.sh` 重启（源 = `scripts/aria-web-start.sh`）+ `--check` 6 项自检
- [x] 4.3 内置浏览器：Provider 管理面板里 Kimi Code 行出现【更新】+ 当前/最新版本；
      点击 → 来源告知 → 确认 → 作业 `completed` → `kimi --version` 变为 2.5.2 →
      「版本落后」标签与【更新】按钮消失
- [x] 4.4 回归：其余四类的按钮规则未受影响（claude/codex/pi/codegraph 逐项核对）

## 5. 收尾

- [x] 5.1 `cadence/notes/` 记录根因、实现与验收证据
- [x] 5.2 k3 审查（实现 + delta）并处置发现（结论：无「必须先修」；F1-F4 四条 P3 文档一致性已随本 change 处置）
- [x] 5.3 归档：sync 主规格 → archive change；提交并推送功能分支