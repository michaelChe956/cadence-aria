fn mappings_by_physical_id(
    journal: &IdentityMigrationJournal,
) -> Result<BTreeMap<String, RepositoryIdentityMapping>, ProductStoreError> {
    let mut mappings = BTreeMap::new();
    for mapping in &journal.mappings {
        validate_relative_id(&mapping.physical_repository_id)?;
        if mappings
            .insert(mapping.physical_repository_id.clone(), mapping.clone())
            .is_some()
        {
            return Err(ProductStoreError::Ambiguous {
                kind: "identity_migration_mapping",
                id: mapping.physical_repository_id.clone(),
            });
        }
    }
    Ok(mappings)
}

fn mapping_for_physical<'a>(
    mappings: &'a BTreeMap<String, RepositoryIdentityMapping>,
    physical_repository_id: &str,
) -> Result<&'a RepositoryIdentityMapping, ProductStoreError> {
    validate_relative_id(physical_repository_id)?;
    mappings
        .get(physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "identity_migration_mapping",
            id: physical_repository_id.to_string(),
        })
}

fn assign_optional_identity<T: Copy + Eq>(
    slot: &mut Option<T>,
    expected: T,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    match slot {
        Some(actual) if *actual != expected => Err(ProductStoreError::IdentityMismatch {
            kind,
            id: id.to_string(),
        }),
        Some(_) => Ok(()),
        None => {
            *slot = Some(expected);
            Ok(())
        }
    }
}

fn assign_vec_identity<T: Eq>(
    slot: &mut Vec<T>,
    expected: Vec<T>,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    if slot.is_empty() {
        *slot = expected;
        return Ok(());
    }
    if *slot == expected {
        return Ok(());
    }
    Err(ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    })
}

fn child_json_paths(
    root: &Path,
    exact_file_name: Option<&str>,
) -> Result<Vec<PathBuf>, ProductStoreError> {
    fn collect(
        root: &Path,
        exact_file_name: Option<&str>,
        paths: &mut Vec<PathBuf>,
    ) -> Result<(), ProductStoreError> {
        if !root.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if let Some(file_name) = exact_file_name {
                if path.is_dir() {
                    collect(&path, Some(file_name), paths)?;
                } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
                    paths.push(path);
                }
            } else if path.is_dir() {
                collect(&path, None, paths)?;
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    collect(root, exact_file_name, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn read_json_records<T: serde::de::DeserializeOwned>(
    root: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    child_json_paths(root, None)?
        .into_iter()
        .map(|path| read_json(&path))
        .collect()
}

fn read_json_records_shallow<T: serde::de::DeserializeOwned>(
    root: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
    {
        let path = entry
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?
            .path();
        if path.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.into_iter().map(|path| read_json(&path)).collect()
}

fn rewrite_json_records<T, F>(root: &Path, mut mutate: F) -> Result<(), ProductStoreError>
where
    T: serde::de::DeserializeOwned + Serialize,
    F: FnMut(&mut T) -> Result<(), ProductStoreError>,
{
    for path in child_json_paths(root, None)? {
        let mut record: T = read_json(&path)?;
        mutate(&mut record)?;
        write_json(&path, &record)?;
    }
    Ok(())
}

fn source_repositories_digest(
    repositories: &[RepositoryRecord],
) -> Result<String, ProductStoreError> {
    let canonical_json = serde_json::to_vec(repositories).map_err(|error| {
        ProductStoreError::Json(format!("serialize legacy repositories: {error}"))
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical_json)))
}

fn duplicate_repository_id(repositories: &[RepositoryRecord]) -> Option<String> {
    repositories
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .map(|pair| pair[0].id.clone())
}

fn duplicate_mapping_legacy_id(mappings: &[RepositoryIdentityMapping]) -> Option<String> {
    let mut ids = HashSet::new();
    mappings.iter().find_map(|mapping| {
        (!ids.insert(&mapping.legacy_repository_id)).then(|| mapping.legacy_repository_id.clone())
    })
}

fn duplicate_logical_repository_id(ids: &[LogicalRepositoryId]) -> Option<LogicalRepositoryId> {
    let mut seen = HashSet::new();
    ids.iter().find_map(|id| (!seen.insert(*id)).then_some(*id))
}

fn mapping_idempotency_key(
    project_id: &str,
    legacy_repository_id: &str,
    source_digest: &str,
) -> String {
    format!("map:{project_id}:{legacy_repository_id}:{source_digest}")
}

fn repository_source_identity(
    repository: &RepositoryRecord,
) -> Result<RepositorySourceIdentity, ProductStoreError> {
    let canonical_path = canonicalize_repository_path(&repository.path)?;
    let git_dir_output = run_git(&canonical_path, &["rev-parse", "--git-dir"])?;
    let git_dir = PathBuf::from(git_dir_output.trim());
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        canonical_path.join(git_dir)
    };
    let canonical_git_dir = canonicalize_repository_path(&git_dir)?;
    let canonical_origin = match run_git(&canonical_path, &["remote", "get-url", "origin"]) {
        Ok(value) => {
            let origin = value.trim();
            (!origin.is_empty()).then(|| origin.to_string())
        }
        Err(ProductStoreError::Io(message)) if message.contains("git exited") => None,
        Err(error) => return Err(error),
    };
    Ok(RepositorySourceIdentity::from_git_parts(
        &canonical_path,
        canonical_git_dir,
        canonical_origin,
    ))
}

fn canonicalize_repository_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    std::fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}

fn run_git(repository_path: &Path, arguments: &[&str]) -> Result<String, ProductStoreError> {
    let output = Command::new("git")
        .current_dir(repository_path)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", repository_path.display()))
        })?;
    if !output.status.success() {
        return Err(ProductStoreError::Io(format!(
            "git exited {:?} in {}: {}",
            output.status.code(),
            repository_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| ProductStoreError::Io(format!("git output was not UTF-8: {error}")))
}

fn common_non_git_parent(inputs: &[AuthorityInput]) -> Option<PathBuf> {
    let first = inputs.first()?.canonical_path.parent()?.to_path_buf();
    let mut common = first;
    for input in &inputs[1..] {
        let parent = input.canonical_path.parent()?;
        while !parent.starts_with(&common) {
            common = common.parent()?.to_path_buf();
        }
    }
    Some(common)
}

fn expected_member(input: &AuthorityInput) -> CodebaseMemberRecord {
    CodebaseMemberRecord {
        logical_repository_id: input.mapping.logical_repository_id,
        physical_repository_id: input.mapping.physical_repository_id.clone(),
        alias: input.repository.name.clone(),
        role: "repository".to_string(),
        ordinal: input.ordinal,
        source_identity: input.source_identity.clone(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids: vec![input.mapping.primary_checkout_id],
        status: MemberStatus::Active,
        created_at: input.repository.created_at.clone(),
        updated_at: input.repository.updated_at.clone(),
    }
}

fn expected_checkout(input: &AuthorityInput) -> RepositoryCheckoutRecord {
    RepositoryCheckoutRecord {
        checkout_id: input.mapping.primary_checkout_id,
        logical_repository_id: input.mapping.logical_repository_id,
        physical_repository_id: input.mapping.physical_repository_id.clone(),
        kind: CheckoutKind::Main,
        canonical_path: input.canonical_path.clone(),
        checkout_path_hash: repo_hash_for_path(input.canonical_path.to_string_lossy().as_ref()),
        git_dir_identity: input.source_identity.git_dir_identity(),
        revision: None,
        availability: CheckoutAvailability::Available,
        observed_at: input.repository.updated_at.clone(),
        created_at: input.repository.created_at.clone(),
        updated_at: input.repository.updated_at.clone(),
    }
}

fn touch(journal: &mut IdentityMigrationJournal) {
    journal.updated_at = Utc::now().to_rfc3339();
}

