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
        let vite = [
            "vite.config.ts",
            "vite.config.js",
            "vite.config.mts",
            "vite.config.mjs",
            "vite.config.cjs",
            "vite.config.cts",
        ]
        .iter()
        .any(|name| root.join(name).is_file());
        let maven = root.join("pom.xml").is_file();
        let gradle = [
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ]
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
            if maven {
                tech_stack.push("maven".to_string());
            }
            if gradle {
                tech_stack.push("gradle".to_string());
            }
            let repo_type = match (vite, maven || gradle) {
                (true, true) => RepositoryType::Mixed,
                (true, false) => RepositoryType::Frontend,
                (false, true) => RepositoryType::Backend,
                (false, false) => RepositoryType::Library,
            };
            return Ok(DetectedRepositoryProfile {
                repo_type,
                tech_stack,
                initialization_commands: Vec::new(),
            });
        }

        if maven || gradle {
            let mut tech_stack = Vec::new();
            if maven {
                tech_stack.push("maven".to_string());
            }
            if gradle {
                tech_stack.push("gradle".to_string());
            }
            return Ok(DetectedRepositoryProfile {
                repo_type: RepositoryType::Backend,
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

