/// 聚合索引激活落点的 manifest 指针回写（store 侧通道）。
///
/// 独立 include：`store.rs` 已贴近 1200 行门禁（`it_core::large_file_guard`），
/// 沿用 `store_purge.inc.rs` 的 impl 续篇先例；文本包含进 `store.rs`，
/// 可访问其私有路径与校验辅助。
impl LogicalCodebaseStore {
    /// 回写 manifest 的 `active_aggregate_index_id`：该世代聚合索引
    /// `replace_active` 成为 active 的那一刻由
    /// `AggregateIndexOperation::apply_index` 调用（生产唯一写方）。此前该
    /// 指针生产上无人写入——重建成功后 evidence 链首查（`load_or_pin_index`）
    /// 仍恒 `evidence_index_unavailable`（503），而 pin 文件要靠一次成功首查
    /// 才生成（鸡生蛋）。
    ///
    /// 与成员注册共用 `.manifest-registration.lock` 串行化 manifest
    /// 读-改-写（与 attach 的 membership_revision 推进互不丢更新）；
    /// `member_ids`/`membership_revision` 原样保留（同修订回写，
    /// `save_manifest` 校验放行）；指针已指向该世代时幂等直返，不空刷
    /// `updated_at`。巡检口径：dangling-reference patrol 按本字段引用
    /// `AggregateIndex` 记录判存活——回写指向的 id 即刚落盘的 active 世代。
    pub fn set_active_aggregate_index(
        &self,
        project_id: &str,
        aggregate_index_id: &str,
    ) -> Result<LogicalCodebaseManifest, ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock = self
            .scope_root(project_id)?
            .join(".manifest-registration.lock");
        with_exact_exclusive_lock(&lock, || {
            let mut manifest =
                self.load_manifest(project_id)?
                    .ok_or_else(|| ProductStoreError::NotFound {
                        kind: "logical_codebase_manifest",
                        id: project_id.to_string(),
                    })?;
            if manifest.active_aggregate_index_id.as_deref() == Some(aggregate_index_id) {
                return Ok(manifest);
            }
            manifest.active_aggregate_index_id = Some(aggregate_index_id.to_string());
            manifest.updated_at = Utc::now().to_rfc3339();
            self.save_manifest(project_id, &manifest)?;
            Ok(manifest)
        })
    }
}
