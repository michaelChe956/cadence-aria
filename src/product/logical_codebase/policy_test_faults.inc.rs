#[cfg(test)]
mod publish_faults {
    use crate::product::json_store::ProductStoreError;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// 可注入故障的发布阶段。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum FaultPhase {
        SourceRead,
        OutputWrite,
        LocatorPublish,
        ArtifactSave,
        /// R4:滚动保留删除失败(发布必须仍然成功)。
        RetentionDelete,
    }

    /// 已武装的故障：**按注入 root 分槽**（root → 阶段）。
    ///
    /// 2026-11-01 修复：原实现是**进程级单槽**（`Option<(PathBuf, FaultPhase)>`）。
    /// 注释曾称「以测试专属 tempdir 为界，并行测试互不污染」，但**槽位只有一个**：
    /// 两个测试同时武装时后者覆盖前者 → 前者的故障永远不触发 →
    /// `assert!(publish(..).is_err())` 偶发失败（现场：
    /// `recipe_policy_durable_write_failure_preserves_resume_evidence`、
    /// `publish_failure_leaves_no_orphan_body_or_receipt`，全量并行下 1/3 概率挂）。
    /// 改为按 root 分槽后：并行各占各的槽，命中即消费**该 root 的那一槽**，
    /// 语义与原来一致（单发、按 root 隔离）。
    static ARMED: Mutex<BTreeMap<PathBuf, FaultPhase>> = Mutex::new(BTreeMap::new());

    /// R3 源改写 seam:(绝对路径 → 替换字节)，**按目标路径分槽**（同上原因）。
    static SOURCE_MUTATIONS: Mutex<BTreeMap<PathBuf, Vec<u8>>> =
        Mutex::new(BTreeMap::new());

    /// 注入以测试专属 tempdir 为界:并行测试互不污染（**每 root 一槽**）。
    pub(super) fn arm(root: &Path, phase: FaultPhase) {
        ARMED
            .lock()
            .expect("publish fault mutex")
            .insert(root.to_path_buf(), phase);
    }

    /// 命中(路径在注入 root 下且阶段相同)即消费(单发);未命中照常执行。
    pub(super) fn trip(path: &Path, phase: FaultPhase) -> Option<ProductStoreError> {
        let mut armed = ARMED.lock().expect("publish fault mutex");
        // 命中判定：存在某个 root 是 path 的祖先，且该 root 武装的阶段相同。
        // 命中后只消费**那一个** root 的槽（同 root 的其他阶段武装不动）。
        let hit = armed
            .iter()
            .find(|(root, armed_phase)| {
                **armed_phase == phase && path.starts_with(root.as_path())
            })
            .map(|(root, _)| root.clone());
        let root = hit?;
        let error = ProductStoreError::Io(format!("injected publish fault at {phase:?}"));
        armed.remove(&root);
        Some(error)
    }

    /// R3 测试:读后立刻改写命中路径,模拟外部同步替换/重写在合成窗口内发生。
    /// 与 `trip` 不同，本 seam 持久武装(每次读都会改写)，使重试同样观察到变化。
    /// **按目标路径分槽**（并行测试互不覆盖）。
    pub(super) fn arm_source_mutator(root: &Path, relative_path: &str, replacement: &[u8]) {
        SOURCE_MUTATIONS
            .lock()
            .expect("publish source mutation mutex")
            .insert(root.join(relative_path), replacement.to_vec());
    }

    /// 命中即改写目标文件(保持武装，不清除)。
    pub(super) fn mutate_source_after_read(path: &Path) {
        let armed = SOURCE_MUTATIONS
            .lock()
            .expect("publish source mutation mutex");

        if let Some(replacement) = armed.get(path) {
            let _ = std::fs::write(path, replacement);
        }
    }
}
