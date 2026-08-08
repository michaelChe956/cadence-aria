/// The repository profile observed from deterministic, read-only filesystem signals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRepositoryProfile {
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
    pub initialization_commands: Vec<String>,
}

/// Detects member technology without invoking package managers, build tools, or
/// repository initialization commands.
pub struct RepositoryProfileDetector;

impl RepositoryProfileDetector {
    pub fn detect(root: &Path) -> Result<DetectedRepositoryProfile, ProductStoreError> {
        let package_json = root.join("package.json").is_file();
        let pnpm =
            root.join("pnpm-lock.yaml").is_file() || root.join("pnpm-workspace.yaml").is_file();
        let vite = ["vite.config.ts", "vite.config.js", "vite.config.mts"]
            .iter()
            .any(|name| root.join(name).is_file());

        if package_json {
            let mut tech_stack = vec!["package.json".to_string()];
            if pnpm {
                tech_stack.push("pnpm".to_string());
            }
            if vite {
                tech_stack.push("vite".to_string());
            }
            let repo_type = if vite {
                RepositoryType::Frontend
            } else {
                RepositoryType::Library
            };
            return Ok(DetectedRepositoryProfile {
                repo_type,
                tech_stack,
                initialization_commands: Vec::new(),
            });
        }

        Ok(DetectedRepositoryProfile {
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            initialization_commands: Vec::new(),
        })
    }
}

