use std::path::PathBuf;

// C-4 Task 1 spike：`codegraph query --json` 输出 schema 的解析冒烟测试。
//
// 实测过程（真实 CLI，v1.5.0）：
//   1. 构造 2 成员仓临时聚合根：api/src/lib.ts（定义 format_compact_duration、
//      apiPing）与 web/src/app.ts（import 并调用 format_compact_duration）。
//   2. `codegraph init .` + `codegraph sync .` 建索引。
//   3. `codegraph query "format_compact_duration" --json` 输出固化为 fixture。
//
// 实测结论（fixture 固化的事实）：
//   - 顶层是数组，每项为 `{ "node": {...}, "score": <f64> }`。
//   - node.filePath 是聚合根相对路径（定义点为 "api/src/lib.ts"）。
//   - node.startLine 为数值，node.signature 为非空字符串。
//   - 跨成员命中：web/src/app.ts 的 import 引用节点也被 query 命中
//     （kind=import，filePath="web/src/app.ts"），而非「仅定义点命中」。
// 以上字段即 T2 EvidenceHit { file_path, start_line, symbol } 的映射来源。
#[test]
fn query_hit_fixture_exposes_documented_schema() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/logical_codebase/evidence_index/tests/fixtures/query_hit.json");
    let raw = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read fixture {fixture_path:?}: {error}"));
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("fixture must be valid JSON");

    let hits = value.as_array().expect("top level must be an array");
    assert!(!hits.is_empty(), "query must return at least one hit");

    // 定义点命中：filePath 为聚合根相对路径，startLine 为数值，signature 非空。
    let definition = hits
        .iter()
        .find(|hit| hit["node"]["kind"] == "function")
        .unwrap_or_else(|| panic!("fixture must contain a function node"));
    assert_eq!(definition["node"]["name"], "format_compact_duration");
    assert_eq!(definition["node"]["filePath"], "api/src/lib.ts");
    assert!(definition["node"]["startLine"].is_u64());
    let signature = definition["node"]["signature"]
        .as_str()
        .expect("node.signature must be a string");
    assert!(!signature.is_empty(), "node.signature must be non-empty");
    assert!(definition["score"].is_f64(), "score must be a number");

    // 跨成员命中确认：web/src/app.ts 的 import 引用也被 query 命中。
    let cross_member = hits
        .iter()
        .find(|hit| hit["node"]["filePath"] == "web/src/app.ts")
        .unwrap_or_else(|| panic!("fixture must contain the cross-member web/src/app.ts hit"));
    assert_eq!(cross_member["node"]["kind"], "import");
    assert_eq!(cross_member["node"]["startLine"], 1);
    assert!(
        cross_member["node"]["signature"]
            .as_str()
            .is_some_and(|signature| signature.contains("format_compact_duration")),
        "cross-member import signature must reference format_compact_duration"
    );
}
