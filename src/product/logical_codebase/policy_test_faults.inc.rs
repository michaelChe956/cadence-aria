#[cfg(test)]
mod publish_faults {
    use crate::product::json_store::ProductStoreError;
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

    static ARMED: Mutex<Option<(PathBuf, FaultPhase)>> = Mutex::new(None);
    /// R3 源改写 seam:(绝对路径, 替换字节)。
    pub(super) static SOURCE_MUTATIONS: Mutex<Option<(PathBuf, Vec<u8>)>> = Mutex::new(None);

    /// 注入以测试专属 tempdir 为界:并行测试互不污染。
    pub(super) fn arm(root: &Path, phase: FaultPhase) {
        *ARMED.lock().expect("publish fault mutex") = Some((root.to_path_buf(), phase));
    }

    /// 命中(路径在注入 root 下且阶段相同)即消费(单发);未命中照常执行。
    pub(super) fn trip(path: &Path, phase: FaultPhase) -> Option<ProductStoreError> {
        let mut armed = ARMED.lock().expect("publish fault mutex");
        match armed.as_ref() {
            Some((root, armed_phase)) if *armed_phase == phase && path.starts_with(root) => {
                let error = ProductStoreError::Io(format!("injected publish fault at {phase:?}"));
                *armed = None;
                Some(error)
            }
            _ => None,
        }
    }

    /// R3 测试:读后立刻改写命中路径,模拟外部同步替换/重写在合成窗口内发生。

    /// 与 `trip` 不同，本 seam 持久武装(每次读都会改写)，使重试同样观察到变化。

    pub(super) fn arm_source_mutator(root: &Path, relative_path: &str, replacement: &[u8]) {

        *SOURCE_MUTATIONS

            .lock()

            .expect("publish source mutation mutex") = Some((root.join(relative_path), replacement.to_vec()));

    }



    /// 命中即改写目标文件(保持武装，不清除)。

    pub(super) fn mutate_source_after_read(path: &Path) {

        let armed = SOURCE_MUTATIONS

            .lock()

            .expect("publish source mutation mutex");

        if let Some((target, replacement)) = armed.as_ref()

            && target == path

        {

            let _ = std::fs::write(target, replacement);

        }

    }
}
