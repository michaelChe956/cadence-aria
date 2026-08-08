## Why

每次发布新版本后，用户无法直观获知本次更新了哪些内容。需要在用户打开工作台（`/workbench`）时，以中文要点列表的形式弹窗告知本版本更新信息，参考 GitHub Releases 的精炼形式，让用户及时了解版本变化。

## What Changes

- 新增前端"版本更新弹窗"（What's New Dialog）：进入工作台时展示当前版本的中文要点列表。
- 新增本地维护的中文 changelog 数据源（`web/src/whats-new/`），记录每个版本的标题、日期和要点。
- 弹窗按版本号在浏览器 localStorage 去重：用户看过当前版本后，同一版本不再重复弹出；版本升级后再次弹出。
- 发版流程新增一步：手动同步更新 `Cargo.toml` 版本号与 changelog 数据中的当前版本。

## Capabilities

### New Capabilities

- `whats-new-dialog`: 进入工作台时按版本去重展示版本更新说明的弹窗能力。

### Modified Capabilities

无。

## Impact

- 前端：新增 `web/src/whats-new/`（changelog 数据 + 去重 hook）、新增 `WhatsNewDialog` 组件及测试；修改 `web/src/app-shell.tsx`（挂载时检查并渲染弹窗）。
- 后端：无改动。
- 数据：新增 localStorage key `aria-whats-new-seen`。
- 发版流程：开发者每次发版需手动维护 changelog 数据。
