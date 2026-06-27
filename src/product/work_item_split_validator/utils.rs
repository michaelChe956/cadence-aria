use super::*;

pub(crate) fn error(
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
    }
}

pub(crate) fn warning(
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Warning,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
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
