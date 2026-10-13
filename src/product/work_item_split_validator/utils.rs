use super::*;

pub(crate) fn error(
    kind: PlanningFailureKind,
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    // F2b（REQ-SG-01/02/02A）：显式 typed 工厂——kind 由调用 producer 的
    // 具体失败分支给出（canonical 传递 / 供需三元 ID / security operational /
    // 缺事实折叠 Unknown），不从 code/message 反推；原 stable code 保持。
    // 旧三参签名与 kind=None 兼容层已删除，无 shim（plan 接地 P2-1）。
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
        kind: Some(kind),
    }
}

pub(crate) fn warning(
    kind: PlanningFailureKind,
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    // F2b：同 error——warning 也携带显式 kind；advisory 语义不变。
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Warning,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
        kind: Some(kind),
    }
}

pub(crate) fn compute_reachability(
    edges: &HashSet<(String, String)>,
) -> HashMap<String, HashSet<String>> {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, to) in edges {
        adjacency.entry(from.clone()).or_default().push(to.clone());
    }

    let nodes: HashSet<String> = edges
        .iter()
        .flat_map(|(from, to)| [from.clone(), to.clone()])
        .collect();

    let mut reachability: HashMap<String, HashSet<String>> = HashMap::new();
    for node in &nodes {
        let mut reachable = HashSet::new();
        let mut stack: Vec<String> = adjacency.get(node).cloned().unwrap_or_default();
        while let Some(current) = stack.pop() {
            if reachable.insert(current.clone())
                && let Some(neighbors) = adjacency.get(&current)
            {
                for neighbor in neighbors {
                    stack.push(neighbor.clone());
                }
            }
        }
        reachability.insert(node.clone(), reachable);
    }
    reachability
}

pub(crate) fn is_cwd_inside_repository(cwd: &str) -> bool {
    if cwd.is_empty() {
        return true;
    }
    if cwd.starts_with('/') {
        return false;
    }
    cwd.split('/').all(|part| part != "..")
}

pub(crate) fn is_command_unsafe(command: &str) -> bool {
    let normalized = command.to_ascii_lowercase();
    let dangerous_substrings = [
        "rm -rf /",
        "rm -rf /*",
        "git reset --hard",
        "git clean -fdx",
        "> /",
        ">> /",
        "> ../",
        ">> ../",
        "| sh",
        "| bash",
        "mkfs",
        "dd if=",
    ];
    for pattern in dangerous_substrings {
        if normalized.contains(pattern) {
            return true;
        }
    }
    false
}
