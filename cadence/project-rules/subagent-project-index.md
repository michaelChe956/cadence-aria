---
description: 本项目规则索引——被派工时按活动类型定位项目专属约束
---

# 本项目规则索引

> 面向所有被派工的 subagent。通用协作纪律见 `rule://subagent-protocol`，
> 规则与工具的发现方式见 `rule://subagent-grounding`。
> 本文件只做**索引**：指向本项目专属约束的所在，正文不在此重复。

## 一、按活动定位

| 你即将做的事 | 必读 |
|---|---|
| 跑构建 / 测试 / clippy / fmt | `cadence/project-rules/build-test-commands.md` |
| 在本项目做任何 Rust 开发与本地验证 | `cadence/project-rules/README.md` §已启用项目规则 |
| 被派工执行任务（并行边界、提交、报告落盘） | `cadence/project-rules/subagent-usage.md` |
| 改 Story Spec / Design Spec / Work Item 任一产物链路 | `cadence/project-rules/workspace-artifact-bug-triage.md` |
| 改 Work Item Draft Prompt / Canonical Contract / Provider 输出约束 | `cadence/project-rules/work-item-draft-prompt-validation.md` |

## 二、最容易踩的三条硬约束

这三条违反后果明确，且不读规则一定会踩：

1. **`cargo test` 禁带 `-j 1`** —— 并行度由 `.cargo/config.toml` 的 `jobs = 8` 统一托管，命令行不写 `-j`。
2. **定向跑 `src/lib.rs` 内单元测试必须限制目标**：用 `cargo test --locked --lib <过滤名>`，禁用 `cargo test --locked <过滤名>`（后者仍会遍历全部 integration 二进制）。
3. **改前端必须重建二进制** —— 前端资源经 rust-embed 在编译期嵌入，不重建则改动不生效。

详见 `cadence/project-rules/build-test-commands.md`。

## 三、产物落盘

- Cadence 产物文档（设计、计划、需求、报告）一律放 `cadence/` 下对应子目录，命名遵循 `rule://document-storage`。
- Worker 报告落 `.superpowers/sdd/<plan>/<task>-report.md`，**只写盘不 `git add`**。
- 框架内置规则在 `.claude/rules/`（禁改）；用户自定义规则只放 `cadence/project-rules/`。

## 四、宿主机直跑，不用 Docker

本地开发、测试与 CLI 验证直接使用宿主机 Rust 环境；`rust-toolchain.toml` 是唯一工具链声明来源。工具链缺失时修宿主机环境，不要改用 Docker 绕过。
