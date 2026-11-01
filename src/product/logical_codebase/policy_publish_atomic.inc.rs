// ---------------------------------------------------------------------------
// R1:根政策发布原子化与 revision 递增。
//
// 顺序固定:正文(新 revision 路径，对旧 state 不可见)→ current artifact
// (提交点)→ 不可变回执。任何一步失败都按逆序补偿——删除本次新建的正文/
// 回执、把 artifact 恢复为提交前值——保证 store 读到的 {artifact, 回执,
// 正文} 始终是同一版本，不会出现「文件新、记录旧」或反之。
// ---------------------------------------------------------------------------

/// 一次发布的目标决策:artifact 与 base 引用在写任何字节之前确定。
struct PolicyPublicationDecision {
    artifact: AggregatePolicyArtifact,
    base: AggregatePolicyPublicationReference,
}

fn policy_reference_of(artifact: &AggregatePolicyArtifact) -> AggregatePolicyPublicationReference {
    AggregatePolicyPublicationReference {
        policy_id: artifact.policy_id.clone(),
        revision: artifact.revision,
        digest: artifact.digest.clone(),
    }
}

fn path_present(path: &Path) -> Result<bool, ProductStoreError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "stat {}: {error}",
            path.display()
        ))),
    }
}

/// 既有正文文件必须与本候选逐字节一致:no-clobber 语义下旧 revision 的
/// 正文永不改写,不同字节一律冲突。
fn verify_published_body(
    locator: &Path,
    artifact: &AggregatePolicyArtifact,
) -> Result<(), ProductStoreError> {
    let meta = std::fs::symlink_metadata(locator).map_err(|error| {
        ProductStoreError::Io(format!("stat {}: {error}", locator.display()))
    })?;
    if meta.is_symlink() || !meta.is_file() {
        return Err(invalid_publication(format!(
            "policy locator {} must be a regular file",
            locator.display()
        )));
    }
    let published = std::fs::read(locator).map_err(|error| {
        ProductStoreError::Io(format!("read {}: {error}", locator.display()))
    })?;
    if published != artifact.policy_text.as_bytes() || sha256_hex(&published) != artifact.digest {
        return Err(ProductStoreError::Conflict {
            kind: "aggregate_policy_publication",
            id: locator.display().to_string(),
        });
    }
    Ok(())
}

/// 既有回执必须是同一次发布(逐字节相同),否则冲突不覆盖。
fn verify_receipt_bytes(path: &Path, expected: &[u8]) -> Result<(), ProductStoreError> {
    let existing = std::fs::read(path).map_err(|error| {
        ProductStoreError::Io(format!("read {}: {error}", path.display()))
    })?;
    if existing == expected {
        return Ok(());
    }
    Err(ProductStoreError::Conflict {
        kind: "aggregate_policy_publication",
        id: path.display().to_string(),
    })
}

/// 与 current artifact 比对决定发布目标(R1.1/R1.3):
/// - 正文逐字节未变且既有正文文件一致 → 复用当前 revision(幂等，不新增);
/// - 否则写 `current.revision + 1` 的新 revision 路径(旧 revision 永不改写)。
fn decide_publication(
    project_id: &str,
    logical_codebase_id: &str,
    canonical_root: &Path,
    current: Option<&AggregatePolicyArtifact>,
    policy_text: String,
    created_at: String,
) -> Result<PolicyPublicationDecision, ProductStoreError> {
    let digest = sha256_hex(policy_text.as_bytes());
    let Some(existing) = current else {
        return Ok(PolicyPublicationDecision {
            base: AggregatePolicyPublicationReference {
                policy_id: String::new(),
                revision: 0,
                digest: String::new(),
            },
            artifact: AggregatePolicyArtifact {
                policy_id: format!("policy/{project_id}/{logical_codebase_id}/1"),
                project_id: project_id.to_string(),
                logical_codebase_id: logical_codebase_id.to_string(),
                revision: 1,
                digest,
                policy_text,
                created_at,
            },
        });
    };
    // 正文一致还不够:既有 revision 的正文文件也必须就是这份字节，否则
    // (历史半成品/人工篡改)复用会掩盖不一致——此时走新 revision 修复。
    let locator_matches = std::fs::read(canonical_root.join(&existing.policy_id))
        .ok()
        .is_some_and(|bytes| bytes == existing.policy_text.as_bytes());
    if existing.policy_text == policy_text && locator_matches {
        return Ok(PolicyPublicationDecision {
            artifact: existing.clone(),
            base: policy_reference_of(existing),
        });
    }
    let revision = existing.revision.checked_add(1).ok_or_else(|| {
        invalid_publication(format!("policy revision overflow from {}", existing.revision))
    })?;
    Ok(PolicyPublicationDecision {
        base: policy_reference_of(existing),
        artifact: AggregatePolicyArtifact {
            policy_id: format!("policy/{project_id}/{logical_codebase_id}/{revision}"),
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            revision,
            digest,
            policy_text,
            created_at,
        },
    })
}

/// 冻结候选必须相对 current 可提交:相同 → 复用;严格后继 → 提交;落后或
/// 已被更晚发布推进(current 不再是候选的合法前驱) → 在任何写盘前停等，
/// 绝不先把正文写回再让 artifact 保存失败。
fn ensure_candidate_committable(
    project_id: &str,
    current: Option<&AggregatePolicyArtifact>,
    candidate: &AggregatePolicyArtifact,
) -> Result<(), ProductStoreError> {
    match current {
        Some(existing) if existing == candidate => Ok(()),
        Some(existing) => existing.validate_successor(candidate).map_err(|_| {
            invalid_publication(format!(
                "policy_publication_superseded: candidate revision {} can no longer advance the \
                 current revision {} of project {project_id}",
                candidate.revision, existing.revision
            ))
        }),
        None if candidate.revision == 1 => Ok(()),
        None => Err(invalid_publication(format!(
            "policy_publication_superseded: candidate revision {} has no current artifact to \
             advance in project {project_id}",
            candidate.revision
        ))),
    }
}

/// 回滚 artifact:恢复提交前值;提交前无 artifact 时删除本次写入的文件。
fn restore_artifact(
    path: &Path,
    previous: Option<&AggregatePolicyArtifact>,
) -> Result<(), ProductStoreError> {
    match previous {
        Some(artifact) => write_artifact_durable(path, artifact),
        None => match std::fs::remove_file(path) {
            Ok(()) => fsync_parent(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductStoreError::Io(format!(
                "remove {}: {error}",
                path.display()
            ))),
        },
    }
}

impl AggregatePolicyArtifactStore {
    /// 提交一次已决策的发布(调用方持 scope 锁):正文 → current artifact
    /// (提交点)→ 不可变回执;任一步失败按逆序补偿。
    fn commit_publication(
        &self,
        project_id: &str,
        canonical_root: &Path,
        artifact: &AggregatePolicyArtifact,
        output: &AggregatePolicyPublicationOutput,
        output_path: &Path,
    ) -> Result<(), ProductStoreError> {
        let previous = self.get(project_id)?;
        let locator = canonical_root.join(&artifact.policy_id);
        let mut created: Vec<PathBuf> = Vec::new();
        let mut artifact_written = false;

        let committed = (|| -> Result<(), ProductStoreError> {
            // 1) 正文:新 revision 路径本身对旧 state 不可见。
            if path_present(&locator)? {
                verify_published_body(&locator, artifact)?;
            } else {
                #[cfg(test)]
                if let Some(error) = publish_faults::trip(&locator, FaultPhase::LocatorPublish) {
                    return Err(error);
                }
                publish_bytes_no_clobber(
                    &locator,
                    artifact.policy_text.as_bytes(),
                    Some(canonical_root),
                )?;
                created.push(locator.clone());
            }
            // 2) current artifact:提交点。与 current 相同则幂等跳过。
            if !matches!(&previous, Some(existing) if existing == artifact) {
                #[cfg(test)]
                if let Some(error) =
                    publish_faults::trip(&self.artifact_path(project_id)?, FaultPhase::ArtifactSave)
                {
                    return Err(error);
                }
                self.save_unlocked(project_id, artifact)?;
                artifact_written = true;
            }
            // 3) 不可变回执。
            let output_bytes = serde_json::to_vec_pretty(output)
                .map_err(|error| ProductStoreError::Json(error.to_string()))?;
            #[cfg(test)]
            if let Some(error) = publish_faults::trip(output_path, FaultPhase::OutputWrite) {
                return Err(error);
            }
            if path_present(output_path)? {
                verify_receipt_bytes(output_path, &output_bytes)?;
            } else {
                publish_bytes_no_clobber(output_path, &output_bytes, None)?;
                created.push(output_path.to_path_buf());
            }
            Ok(())
        })();

        if let Err(error) = committed {
            for path in created.iter().rev() {
                if let Err(remove_error) = std::fs::remove_file(path) {
                    tracing::warn!(
                        path = %path.display(),
                        %remove_error,
                        "policy publication rollback could not remove a file"
                    );
                }
            }
            if artifact_written
                && let Err(restore_error) =
                    restore_artifact(&self.artifact_path(project_id)?, previous.as_ref())
            {
                tracing::warn!(
                    %restore_error,
                    "policy publication rollback could not restore the previous artifact"
                );
            }
            return Err(error);
        }
        Ok(())
    }
}
