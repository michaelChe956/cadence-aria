## Context

当前工作台（`web/src/app-shell.tsx` 挂载于 `/workbench`）没有任何版本公告或 changelog 机制。版本号仅存在于 `Cargo.toml`（当前 0.0.5），历史 tag `v0.0.1`~`v0.0.5`。前端已有多个 Dialog 组件（`SettingsDialog`、`CreateLifecycleIssueDialog` 等）可作视觉参考。项目无用户账号体系，工作台为本地工具。

需求来源：参考 GitHub Releases 的精炼要点列表形式，发版时手动维护中文说明，进入工作台时按版本去重弹窗。该方案来自已确认的 brainstorming（数据来源=手写本地文件；去重=localStorage 存版本号；触发点=AppShell 挂载 + 版本号编译期注入）。

## Goals / Non-Goals

**Goals:**

- 纯前端实现，零后端改动、零运行时网络依赖。
- 版本号与 changelog 数据同源、编译期嵌入。
- 去重逻辑可独立测试，与 UI 组件解耦。

**Non-Goals:**

- 不接 GitHub Releases API，不做后端接口、不做用户级已读同步、不做"查看历史更新"入口。
- 不回头补 `0.0.4` 及更早版本内容。
- 不自动从 commit 生成 changelog。
- 不做 `Cargo.toml` → changelog 的版本号自动同步脚本（首版手动）。

## Decisions

### 决策 1：数据源为手写本地 ts 数据，编译期嵌入

changelog 以 ts 数据结构集中在 `web/src/whats-new/changelog.ts`：导出 `CURRENT_VERSION` 字符串、`ChangelogEntry` 接口、`CHANGELOG` 数组（每项含 version/date/title/highlights）。

- 备选 A：单独 `.md` 文件 + 运行期解析——多一层解析、类型不安全，弃。
- 备选 B：后端 API——当前无账号体系、多一次请求，过度设计，弃。
- 备选 C：运行时拉 GitHub API——受速率限制、需联网、release 正文难保证中文，弃。
- 选定方案：要点列表（非完整 markdown 正文），更克制、适合弹窗、渲染简单、类型安全；纯数据便于将来替换数据源而不动组件。

### 决策 2：去重用 localStorage 存已读版本号

key 为 `aria-whats-new-seen`，值为版本号字符串。进入工作台时读取并与 `CURRENT_VERSION` 比较；关闭弹窗时写入当前版本号。

- 备选：后端按用户/机器记录——无账号体系、实现成本高，弃。
- 弃备选权衡：换浏览器/清缓存会再弹一次，属可接受。

### 决策 3：触发点在 AppShell 挂载，版本号编译期注入

在 `web/src/app-shell.tsx` 的 `useEffect` 挂载时检查。版本号通过 import `CURRENT_VERSION`（ts 模块）直接带入，构建期确定。

- 备选：后端 API 返版本号——多一次请求且单一来源收益在本场景不显著，弃。
- 备选：在 router/main 更外层弹——与"打开工作台时弹"的原始需求不符，弃。

### 决策 4：去重逻辑抽成独立模块

将"读/写已读版本 + 判定是否应弹"抽成独立 hook 或纯函数模块（`web/src/whats-new/useWhatsNew.ts`），使 `AppShell` 只负责渲染，去重逻辑可单独单测。

## Risks / Trade-offs

- [换浏览器/清缓存会重复弹窗] → 可接受；属于本地工具的合理行为。
- [发版时漏更新 changelog 数据] → 通过发版流程文档与 tasks 提醒；后续可加脚本校验 `CURRENT_VERSION` 与 `Cargo.toml` 一致。
- [localStorage 不可用] → 按 spec 静默降级，try/catch 包裹读写，不阻断工作台。
- [要点列表需人工保证中文与准确性] → 由发版者负责，与 GitHub Releases 手写正文同样的责任边界。

## Migration Plan

- 纯新增功能，无数据迁移、无兼容性破坏。
- 首次上线时，所有现有用户的本地无 `aria-whats-new-seen` 记录，会在下次进入工作台时看到首个有记录版本（如 0.0.5 或上线版本）的弹窗一次，符合预期。
- 回滚：移除新增文件与 `app-shell.tsx` 的改动即可，无副作用。
