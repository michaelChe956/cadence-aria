#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGitSnapshot {
    status_porcelain: Vec<u8>,
    head: Vec<u8>,
    refs: Vec<u8>,
    worktree_list: Vec<u8>,
    config: Vec<u8>,
    hooks: Vec<(PathBuf, Vec<u8>)>,
    index: Option<Vec<u8>>,
}

impl RepositoryGitSnapshot {
    pub fn capture(root: &Path) -> Result<Self, ProductStoreError> {
        Ok(Self {
            status_porcelain: git_stdout(root, &["status", "--porcelain"])?,
            head: git_stdout(root, &["rev-parse", "HEAD"])?,
            refs: git_stdout(root, &["for-each-ref", "--format=%(refname) %(objectname)"])?,
            worktree_list: git_stdout(root, &["worktree", "list", "--porcelain"])?,
            config: fs::read(root.join(".git/config")).unwrap_or_default(),
            hooks: read_tree_bytes(&root.join(".git/hooks"))?,
            index: fs::read(root.join(".git/index")).ok(),
        })
    }

    pub fn assert_unchanged(&self, after: &Self) -> Result<(), ProductStoreError> {
        if self == after {
            Ok(())
        } else {
            Err(ProductStoreError::IdentityMismatch {
                kind: "registration_git_side_effect",
                id: "git_snapshot_changed".into(),
            })
        }
    }
}

#[derive(Debug, Clone)]
struct GitCandidateEvidence {
    git_root: PathBuf,
    canonical_git_dir: PathBuf,
    source_key_digest: String,
}

fn discover_git_directories(root: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let mut directories = Vec::new();
    discover_git_directories_recursive(root, root, &mut directories)?;
    directories.sort();
    Ok(directories)
}

fn discover_git_directories_recursive(
    root: &Path,
    directory: &Path,
    directories: &mut Vec<PathBuf>,
) -> Result<(), ProductStoreError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        ProductStoreError::Io(format!(
            "read aggregate directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!(
                "read aggregate directory entry {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "inspect aggregate entry {}: {error}",
                path.display()
            ))
        })?;
        if !file_type.is_dir() || path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path == root {
            continue;
        }
        if path.join(".git").exists() {
            directories.push(path.clone());
        }
        discover_git_directories_recursive(root, &path, directories)?;
    }
    Ok(())
}

fn git_probe(
    repository_path: &Path,
    arguments: &[&str],
) -> Result<Option<String>, ProductStoreError> {
    let allowed = [
        ["status", "--porcelain"].as_slice(),
        ["rev-parse", "HEAD"].as_slice(),
        ["for-each-ref", "--format=%(refname) %(objectname)"].as_slice(),
        ["worktree", "list", "--porcelain"].as_slice(),
        ["rev-parse", "--show-toplevel"].as_slice(),
        ["rev-parse", "--git-dir"].as_slice(),
        ["config", "--get", "remote.origin.url"].as_slice(),
    ];
    if !allowed.contains(&arguments) {
        return Err(ProductStoreError::InvalidRecord {
            kind: "registration_git_command",
            reason: format!("Git command is not allowed: {arguments:?}"),
        });
    }
    let output = Command::new("git")
        .current_dir(repository_path)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", repository_path.display()))
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|error| ProductStoreError::Io(format!("Git output was not UTF-8: {error}")))
}

fn git_stdout(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, ProductStoreError> {
    let allowed = [
        ["status", "--porcelain"].as_slice(),
        ["rev-parse", "HEAD"].as_slice(),
        ["for-each-ref", "--format=%(refname) %(objectname)"].as_slice(),
        ["worktree", "list", "--porcelain"].as_slice(),
        ["rev-parse", "--show-toplevel"].as_slice(),
        ["rev-parse", "--git-dir"].as_slice(),
        ["config", "--get", "remote.origin.url"].as_slice(),
    ];
    if !allowed.contains(&arguments) {
        return Err(ProductStoreError::InvalidRecord {
            kind: "registration_git_command",
            reason: format!("Git command is not allowed: {arguments:?}"),
        });
    }
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", root.display()))
        })?;
    if !output.status.success() {
        return Err(ProductStoreError::Io(format!(
            "git exited {:?} in {}: {}",
            output.status.code(),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn read_tree_bytes(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, ProductStoreError> {
    fn visit(
        root: &Path,
        path: &Path,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), ProductStoreError> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read Git hooks {}: {error}",
                    path.display()
                )));
            }
        };
        let mut paths = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ProductStoreError::Io(format!("read Git hooks entry: {error}")))?;
        paths.sort();
        for child in paths {
            let metadata = fs::symlink_metadata(&child).map_err(|error| {
                ProductStoreError::Io(format!("inspect Git hooks {}: {error}", child.display()))
            })?;
            if metadata.is_dir() {
                visit(root, &child, files)?;
            } else if metadata.is_file() {
                let relative = child.strip_prefix(root).map_err(|error| {
                    ProductStoreError::Io(format!(
                        "relativize Git hooks {}: {error}",
                        child.display()
                    ))
                })?;
                files.push((
                    relative.to_path_buf(),
                    fs::read(&child).map_err(|error| {
                        ProductStoreError::Io(format!("read Git hook {}: {error}", child.display()))
                    })?,
                ));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    if root.exists() {
        visit(root, root, &mut files)?;
    }
    Ok(files)
}

fn git_probe_inconsistent(repository_path: &Path, observation: &str) -> ProductStoreError {
    ProductStoreError::Io(format!(
        "Git probe for {} succeeded without {observation} in {}",
        repository_path.display(),
        repository_path.display()
    ))
}

fn preflight_revision(
    canonical_path: Option<&Path>,
    git_root: Option<&Path>,
    source_identity: Option<&RepositorySourceIdentity>,
    head: Option<&str>,
    status: Option<&str>,
) -> String {
    let status_digest = format!(
        "sha256:{:x}",
        Sha256::digest(status.unwrap_or_default().as_bytes())
    );
    let payload = format!(
        "{}\0{}\0{}\0{}\0{}",
        canonical_path
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        git_root
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        source_identity
            .map(|identity| identity.key_digest.as_str())
            .unwrap_or_default(),
        head.unwrap_or_default(),
        status_digest,
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}

