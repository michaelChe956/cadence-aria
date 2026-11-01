impl RepositoryStore {
    pub fn find_by_path(
        &self,
        project_id: &str,
        path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        let canonical_path = canonicalize_repo_path(path)?;
        let canonical_text = canonical_path.to_string_lossy();
        let target_hash = repo_hash_for_path(canonical_text.as_ref());

        Ok(self.list(project_id)?.into_iter().find(|record| {
            if record.repo_hash == target_hash {
                return true;
            }

            fs::canonicalize(&record.path)
                .map(|record_path| record_path == canonical_path)
                .unwrap_or_else(|_| record.path.to_string_lossy() == canonical_text)
        }))
    }


}
