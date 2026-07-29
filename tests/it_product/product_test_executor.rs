use cadence_aria::product::test_executor::planned_test_commands_from_markdown;

#[test]
fn extracts_planned_verification_commands_from_work_item_markdown() {
    let markdown = r#"
# 爬楼梯问题 Work Item

## 任务拆分

验证命令：
- `uv run python -m unittest -v tests.test_climbing_stairs`

预期结果：
- 测试通过

## 验证命令

主验证命令：
- `uv run python -m unittest -v tests.test_climbing_stairs`

辅助检查命令：
- `git diff -- climbing_stairs.py tests/test_climbing_stairs.py`

验收条件：
- `climb_stairs(1)` 返回 `1`。
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].id, "planned_001");
    assert_eq!(
        specs[0].command,
        vec![
            "uv",
            "run",
            "python",
            "-m",
            "unittest",
            "-v",
            "tests.test_climbing_stairs"
        ]
    );
    assert_eq!(specs[1].id, "planned_002");
    assert_eq!(
        specs[1].command,
        vec![
            "git",
            "diff",
            "--",
            "climbing_stairs.py",
            "tests/test_climbing_stairs.py"
        ]
    );
}

#[test]
fn extracts_planned_verification_commands_from_fenced_code_blocks() {
    let markdown = r#"
# 爬楼梯问题 Work Item

## 验证命令

首选无第三方测试依赖命令：

```bash
uv run python -m unittest discover -s tests -v
```

范围检查命令：

```bash
git diff -- climbing_stairs.py tests/test_climbing_stairs.py
```
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 2);
    assert_eq!(
        specs[0].command,
        vec![
            "uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"
        ]
    );
    assert_eq!(
        specs[1].command,
        vec![
            "git",
            "diff",
            "--",
            "climbing_stairs.py",
            "tests/test_climbing_stairs.py"
        ]
    );
}

#[test]
fn normalizes_planned_pnpm_commands_from_cd_web_form() {
    let markdown = r#"
# Provider 依赖 Work Item

## 验证命令

- `cargo test --locked --lib provider_dependencies`
- `cd web && pnpm test`
- `cd web && pnpm build`
- `cd web && pnpm test:e2e`
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 4);
    assert_eq!(
        specs[0].command,
        vec![
            "cargo",
            "test",
            "--locked",
            "--lib",
            "provider_dependencies"
        ]
    );
    assert_eq!(specs[1].command, vec!["pnpm", "-C", "web", "test"]);
    assert_eq!(specs[2].command, vec!["pnpm", "-C", "web", "build"]);
    assert_eq!(specs[3].command, vec!["pnpm", "-C", "web", "test:e2e"]);
}
