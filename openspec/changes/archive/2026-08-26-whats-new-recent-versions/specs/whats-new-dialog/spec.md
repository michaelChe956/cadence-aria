## MODIFIED Requirements

### Requirement: 进入工作台展示当前版本更新说明

系统 SHALL 在用户进入工作台（`/workbench`）时，若当前版本（`CURRENT_VERSION`）的更新说明未被标记为已读，则展示版本更新弹窗。弹窗正文 SHALL 展示"版本不高于当前版本"的最新至多 4 个版本条目，按新→旧排列（当前版本区块在最上）；版本高于当前版本的预备条目 SHALL NOT 展示。

#### Scenario: 首次进入当前版本

- **WHEN** 用户进入工作台，且浏览器本地未记录当前版本为已读
- **THEN** 系统展示版本更新弹窗，内容包含至多 4 个版本的更新区块，当前版本区块位于最上

#### Scenario: 已读当前版本不再弹出

- **WHEN** 用户进入工作台，且浏览器本地已记录当前版本为已读
- **THEN** 系统不展示版本更新弹窗

#### Scenario: 未发布的预备条目不展示

- **WHEN** `CHANGELOG` 中存在版本高于 `CURRENT_VERSION` 的预备条目
- **THEN** 弹窗不展示该条目，展示窗口从 `CURRENT_VERSION` 起向下取至多 4 条

## ADDED Requirements

### Requirement: 更新动态滚动窗口维护

`CHANGELOG` 数组 SHALL 按版本新→旧排列；展示窗口 SHALL 取"不高于当前版本"的前 4 条。发布升版本时维护者 SHALL 人工裁剪数组长度至 4 条（删除最旧条目）。

#### Scenario: 窗口不超过 4 条

- **WHEN** 弹窗渲染展示窗口
- **THEN** 展示的版本区块数不超过 4，且全部不高于当前版本

#### Scenario: 数组有序性可被测试锁定

- **WHEN** 运行 changelog 相关测试
- **THEN** 断言数组条目按版本新→旧排列，且展示窗口选择逻辑输出至多 4 条
