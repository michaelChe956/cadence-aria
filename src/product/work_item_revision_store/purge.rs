use std::path::Path;

use crate::product::json_store::ProductStoreError;

use super::WorkItemRevisionStore;

impl WorkItemRevisionStore {
    /// 删除一个 plan 的全部 revision 产物与 publication 记录。
    ///
    /// - `work-item-revisions/<plan_id>/`（`plan_root`，含 lineage / plan-revisions /
    ///   draft-revisions / verification / publications 等全部子目录）
    /// - `work-item-revision-publications/<plan_id>/`（initial / amendment 发布日志）
    ///
    /// NotFound 视为成功：清理路径不应要求被清理对象预先存在。
    pub fn purge_plan_revisions(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        let plan_root = self.plan_root(project_id, issue_id, plan_id);
        remove_dir_all_if_exists(&plan_root)?;

        let publications = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-item-revision-publications")
            .join(plan_id);
        remove_dir_all_if_exists(&publications)?;

        Ok(())
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}
