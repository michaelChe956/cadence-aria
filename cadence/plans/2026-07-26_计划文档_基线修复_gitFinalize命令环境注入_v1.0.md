# git_finalize 命令环境注入 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `git_finalize` 的 git 子进程携带 `LC_ALL=C` + 验证过的 `HOME` +（存在时的）`SSH_AUTH_SOCK`，修复因 `env_clear` 隔离导致 `git commit` 身份解析失败（exit 128）与 `git push` 潜在 ssh 失败。

**Architecture:** `RepositoryRegistrationCoordinator` 新增 `git_environment` 字段（构造时从进程环境默认组装，`with_git_environment` 注入点覆盖），`run_git` 用该字段替代内联 `{LC_ALL: C}`；Web 层 builder 用 `validate_user_home` 验证过的 home 显式注入。bounded runner 的 `env_clear` 隔离语义不变。

**Tech Stack:** Rust / cargo（宿主机工具链，不使用 Docker）。

**关联契约：** OpenSpec change `openspec/changes/fix-git-finalize-command-environment/`（proposal / design / specs / tasks 已获批）。

## Global Constraints

- 🔴 禁止给任何 cargo 命令携带 `-j`；定向单测必须带 `--lib`（如 `cargo test --locked --lib repository_store`）。
- 不修改 `src/cross_cutting/`（bounded runner / process_manager 的隔离语义不动）。
- `git_environment` 只允许三个键：`LC_ALL`、`HOME`、`SSH_AUTH_SOCK`；其余变量保持隔离。
- 不硬编码任何主机绝对路径；注入 HOME 必须来自 `validate_user_home` 或测试注入。
- 测试组织：`src/product/repository_store/registration/tests/cases.rs` 用 `include!` 引入 `cases/*.rs`，新测试追加到 `cases/git_finalize.rs` 末尾，所需 helper（`RecordingProjectLookup`、`RecordingRepositoryPersistence`、`RecordingCadence`、`RecordingInitializer`、`AvailableHealth`、`registry()`）来自父模块 `tests/mod.rs`，已 `use super::*` 可见。
- `git_finalize` 与 `run_git` 对测试可见（`pub(super)`/`pub(crate)` 链路已成立），新增 `with_git_environment` 用 `pub(crate)`。

---

### Task 1: 失败测试（TDD RED）

**Files:**
- Test: `src/product/repository_store/registration/tests/cases/git_finalize.rs`（文件末尾追加）

**Interfaces:**
- Consumes: 父模块 helper（见 Global Constraints）；`crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner`（unit struct，已实现 `BoundedCommandRunner`）。
- Produces: 两个测试函数，Task 2 使其转绿：
  - `git_finalize_commits_with_injected_home_identity`（真实 git 行为回归）
  - `default_git_environment_includes_allowed_keys_only_when_present`（环境组装单测）

- [ ] **Step 1: 在 `cases/git_finalize.rs` 末尾追加两个测试**

```rust
#[tokio::test]
async fn git_finalize_commits_with_injected_home_identity() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = Aria Finalize\n\temail = aria-finalize@example.com\n",
    )
    .unwrap();
    let git_root = temp.path().join("repository");
    std::fs::create_dir_all(&git_root).unwrap();
    for argv in [vec!["init", "-b", "main"], vec!["config", "commit.gpgsign", "false"]] {
        let status = std::process::Command::new("git")
            .args(&argv)
            .current_dir(&git_root)
            .status()
            .unwrap();
        assert!(status.success(), "git {argv:?} failed");
    }
    std::fs::write(git_root.join("AGENTS.md"), "# agents\n").unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
        Arc::new(RecordingProjectLookup {
            calls: calls.clone(),
        }),
        Arc::new(RecordingRepositoryPersistence {
            calls: calls.clone(),
            created: AtomicUsize::new(0),
        }),
        RepositoryInitializationOperationStore::new(ProductAppPaths::new(
            temp.path().join(".aria"),
        )),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        registry(),
        Arc::new(RecordingCadence {
            calls: calls.clone(),
            source_root: temp.path().join("cadence-source"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner),
        Arc::new(|| "2026-07-25T00:00:00Z".to_string()),
        Arc::new(RecordingInitializer {
            calls: calls.clone(),
        }),
        Duration::from_secs(30),
        Duration::from_secs(30),
    )
    .with_git_environment(std::collections::BTreeMap::from([
        ("LC_ALL".to_string(), "C".to_string()),
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
    ]));

    let outcome = coordinator
        .git_finalize(&git_root, CancellationToken::new())
        .await
        .expect("git_finalize with injected HOME must succeed");
    assert!(
        outcome.is_some(),
        "no remote configured, push must be skipped with a note"
    );

    let log = std::process::Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>"])
        .current_dir(&git_root)
        .output()
        .unwrap();
    assert!(log.status.success());
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "Aria Finalize <aria-finalize@example.com>"
    );
}

#[test]
fn default_git_environment_includes_allowed_keys_only_when_present() {
    let environment = super::super::default_git_environment(|key| match key {
        "HOME" => Some(std::ffi::OsString::from("/home/tester")),
        "SSH_AUTH_SOCK" => None,
        _ => None,
    });
    assert_eq!(environment.get("LC_ALL").map(String::as_str), Some("C"));
    assert_eq!(
        environment.get("HOME").map(String::as_str),
        Some("/home/tester")
    );
    assert!(!environment.contains_key("SSH_AUTH_SOCK"));
    assert_eq!(environment.len(), 2);

    let empty = super::super::default_git_environment(|key| {
        (key == "HOME").then(|| std::ffi::OsString::from(""))
    });
    assert_eq!(empty.len(), 1, "empty HOME must be skipped");
    assert_eq!(empty.get("LC_ALL").map(String::as_str), Some("C"));
}
```

- [ ] **Step 2: 运行确认 RED**

Run: `cargo test --locked --lib git_finalize`
Expected: FAIL——`with_git_environment` 与 `default_git_environment` 不存在（编译错误即 RED）；这同时证明行为测试此前无法通过（真实 git 下 commit 必然 128）。

- [ ] **Step 3: 提交**

```bash
git add src/product/repository_store/registration/tests/cases/git_finalize.rs
git commit -m "test: require injected home identity for git_finalize commit"
```

---

### Task 2: coordinator 环境字段与 run_git 改造（TDD GREEN）

**Files:**
- Modify: `src/product/repository_store/registration.rs`（struct 字段 :251-264、`new_with_operations` :308-332、`run_git` :764-783，新增 `default_git_environment` 与 `with_git_environment`）

**Interfaces:**
- Consumes: Task 1 测试。
- Produces:
  - `pub(crate) fn RepositoryRegistrationCoordinator::with_git_environment(self, BTreeMap<String, String>) -> Self`
  - `fn default_git_environment(read: impl Fn(&str) -> Option<std::ffi::OsString>) -> BTreeMap<String, String>`（`registration` 模块私有，测试经 `super::super::` 访问）
  - Task 3 使用 `git_environment()` 访问器：`#[cfg(test)] pub(crate) fn git_environment(&self) -> &BTreeMap<String, String>`

- [ ] **Step 1: struct 新增字段（registration.rs :251-264 区域）**

在 `initialization_timeout: Duration,` 之后追加：

```rust
    git_environment: BTreeMap<String, String>,
```

确认文件顶部已 `use std::collections::BTreeMap;`（`run_git` 已在用，存在）。

- [ ] **Step 2: 新增组装函数与注入点（`impl RepositoryRegistrationCoordinator` 内）**

```rust
    pub(crate) fn with_git_environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.git_environment = environment;
        self
    }

    #[cfg(test)]
    pub(crate) fn git_environment(&self) -> &BTreeMap<String, String> {
        &self.git_environment
    }
```

在 `impl` 块外（模块级、`run_git` 附近）新增：

```rust
fn default_git_environment(
    read: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([("LC_ALL".to_string(), "C".to_string())]);
    for key in ["HOME", "SSH_AUTH_SOCK"] {
        if let Some(value) = read(key).filter(|value| !value.is_empty()) {
            environment.insert(key.to_string(), value.to_string_lossy().into_owned());
        }
    }
    environment
}
```

- [ ] **Step 3: `new_with_operations` 初始化字段（:308-332 区域）**

`Self { ... }` 字面量中 `initialization_timeout,` 之后追加：

```rust
            git_environment: default_git_environment(|key| std::env::var_os(key)),
```

- [ ] **Step 4: `run_git` 改用字段（:777 区域）**

oldText（逐字）：

```rust
                environment: BTreeMap::from([("LC_ALL".to_string(), "C".to_string())]),
```

newText（逐字）：

```rust
                environment: self.git_environment.clone(),
```

- [ ] **Step 5: 运行确认 GREEN**

Run: `cargo test --locked --lib repository_store`
Expected: PASS（含 Task 1 两个新测试与全部既有用例）

- [ ] **Step 6: 提交**

```bash
git add src/product/repository_store/registration.rs
git commit -m "fix: inject HOME and SSH_AUTH_SOCK into git_finalize command environment"
```

---

### Task 3: Web 层显式注入验证过的 HOME

**Files:**
- Modify: `src/web/handlers/repository_registration.rs`（`build()` :195-241 区域；文件已有 `BTreeMap` 可用性需确认，若无则 `use std::collections::BTreeMap;`）
- Test: 同文件 `#[cfg(test)]` 测试模块（:498 附近已有 `resolve_user_home` 测试）

**Interfaces:**
- Consumes: Task 2 的 `with_git_environment` 与 `git_environment()` 访问器。
- Produces: builder `build()` 产出的 coordinator 携带 `{LC_ALL, HOME=<验证过的 home>, SSH_AUTH_SOCK?}`。

- [ ] **Step 1: 修改 `build()`**

oldText（逐字，:205-211 区域）：

```rust
        let cadence_skills = self.cadence_skills.unwrap_or_else(|| {
            Arc::new(CadenceSkillsManager::with_dependencies(
                home,
                self.runner.clone(),
                self.command_environment,
            ))
        });
```

newText（逐字）：

```rust
        let cadence_skills = self.cadence_skills.unwrap_or_else(|| {
            Arc::new(CadenceSkillsManager::with_dependencies(
                home.clone(),
                self.runner.clone(),
                self.command_environment,
            ))
        });
        let git_environment = git_finalize_environment(&home);
```

oldText（逐字，:225-240 区域）：

```rust
        Ok(RepositoryRegistrationDependencies {
            coordinator: Arc::new(RepositoryRegistrationCoordinator::new_with_operations(
```

newText（逐字）：

```rust
        let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
```

oldText（逐字）：

```rust
                self.git_command_timeout,
                self.initialization_timeout,
            )),
        })
    }
}
```

newText（逐字）：

```rust
                self.git_command_timeout,
                self.initialization_timeout,
            )
            .with_git_environment(git_environment);
        Ok(RepositoryRegistrationDependencies {
            coordinator: Arc::new(coordinator),
        })
    }
}

fn git_finalize_environment(home: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut environment = std::collections::BTreeMap::from([
        ("LC_ALL".to_string(), "C".to_string()),
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
    ]);
    if let Some(value) = std::env::var_os("SSH_AUTH_SOCK").filter(|value| !value.is_empty()) {
        environment.insert(
            "SSH_AUTH_SOCK".to_string(),
            value.to_string_lossy().into_owned(),
        );
    }
    environment
}
```

注意：`new_with_operations(...)` 原为 `Arc::new(...)` 的直接参数，改成先绑定 `let coordinator` 后，原调用末尾的 `)),` 需对应调整为 `)` 收口（上方 newText 已给出收口形态），中间参数行缩进会失配——收口后立即运行 `rustfmt src/web/handlers/repository_registration.rs` 整理格式（只格式化该文件，不用 `cargo fmt` 全量）。`build()` 内后续不再使用 `home`，fake 路径（`default_dependencies` :330-345）经同一 `build()`，自动获得相同注入，无需改动。

- [ ] **Step 2: 追加 Web 层测试（`#[cfg(test)]` 模块内）**

```rust
    #[test]
    fn built_coordinator_carries_validated_home_in_git_environment() {
        let dependencies = RepositoryRegistrationDependencies::builder(
            ProductAppPaths::new(std::env::temp_dir().join("git-env-test")),
            "/home/tester",
            Arc::new(crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner),
            Arc::new(ProviderAvailabilityGate::new(Arc::new(
                FixedAvailableProviderHealth,
            ))),
            Arc::new(ProviderRegistry::default()),
        )
        .build()
        .expect("build");
        let environment = dependencies.coordinator.git_environment();
        assert_eq!(
            environment.get("HOME").map(String::as_str),
            Some("/home/tester")
        );
        assert_eq!(environment.get("LC_ALL").map(String::as_str), Some("C"));
        for key in environment.keys() {
            assert!(
                matches!(key.as_str(), "LC_ALL" | "HOME" | "SSH_AUTH_SOCK"),
                "unexpected key {key}"
            );
        }
    }
```

fixture 说明：`FixedAvailableProviderHealth`（本文件 :408 已实现 `ProviderHealthSource`）与 `ProviderRegistry::default()` 均为现成构造；测试模块已 `use super::*`，`ProviderAvailabilityGate`/`ProviderRegistry`/`Arc` 均在作用域内。若 `TokioBoundedCommandRunner` 路径与导出不符，以编译错误提示为准调整 import，但不得改动被测逻辑。

- [ ] **Step 3: 运行确认 GREEN**

Run: `cargo test --locked --lib repository_registration`
Expected: PASS

- [ ] **Step 4: 提交**

```bash
git add src/web/handlers/repository_registration.rs
git commit -m "fix: pass validated home to coordinator git environment at web layer"
```

---

### Task 4: 全量验证与收尾

**Files:**
- 无代码改动；勾选 `openspec/changes/fix-git-finalize-command-environment/tasks.md` 1.1–3.4。

- [ ] **Step 1: 格式化与静态检查**

Run: `cargo fmt --check`
Expected: 无输出（通过）

Run: `cargo clippy --all-targets --all-features --locked -- -D warnings`
Expected: 无 warning（通过）

- [ ] **Step 2: 定向回归**

Run: `cargo test --locked --lib repository_store`
Expected: PASS

Run: `cargo test --locked --test it_web web_repository_initialization`
Expected: PASS

- [ ] **Step 3: OpenSpec 校验**

Run: `openspec validate fix-git-finalize-command-environment --strict`
Expected: `Change 'fix-git-finalize-command-environment' is valid`

- [ ] **Step 4: 勾选 tasks.md 1.1–3.4 并提交**

```bash
git add openspec/changes/fix-git-finalize-command-environment/tasks.md
git commit -m "docs: check off fix-git-finalize-command-environment implementation tasks"
```

- [ ] **Step 5: 交付提醒（写入报告）**

向操作者汇报：服务重启后重新执行一次代码库初始化可真实验证 git_finalize；历史失败的目标仓库需手动 `git commit / git push` 补齐。

---

## Self-Review 记录

- Spec 覆盖：需求「git_finalize 命令环境」三场景 → Task 1 真实 git 测试（身份场景）+ Task 1 组装单测（隔离/条件注入场景）+ Task 3 Web 注入与键白名单断言。无缺口。
- Placeholder 扫描：代码与命令均完整；Task 3 Step 2 允许按既有测试模块实际 helper 微调 fixture，属编译适配而非占位。
- 类型/名称一致性：`with_git_environment`、`default_git_environment`、`git_environment()` 三处定义与使用逐字一致；`TokioBoundedCommandRunner` 路径与现状 `bounded_command_runner.rs:52` 一致。
