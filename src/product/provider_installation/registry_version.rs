//! registry `dist-tags.latest` 查询 + 24h durable 版本缓存
//!（REQ-PCI-07 / tasks 11.3、11.4）。
//!
//! 缓存条目记录包名、最新版本与查询时间戳；durable（磁盘 JSON，跨进程
//! 重启可复用）。本模块按**版本来源包名**工作（`InstallSourceRow::version_package`），
//! 与安装来源相互独立 —— Kimi Code 的安装源是官方脚本，但版本来源是 npm 包
//! `@moonshot-ai/kimi-code`，同样经本模块判定「有新版」（REQ-PCI-07）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// 缓存有效期：24 小时（REQ-PCI-07）。
pub const VERSION_CACHE_TTL: Duration = Duration::hours(24);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCacheEntry {
    pub package: String,
    pub latest_version: String,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct VersionCacheFile {
    entries: BTreeMap<String, VersionCacheEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryVersionError {
    #[error("registry query failed: {0}")]
    Query(String),
    #[error("version cache I/O failed: {0}")]
    Cache(String),
}

/// registry 查询 seam（测试注入计数/失败 fake）。
#[async_trait::async_trait]
pub trait RegistryLatestVersionSource: Send + Sync {
    async fn dist_tags_latest(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<String, RegistryVersionError>;
}

#[derive(Debug, Deserialize)]
struct RegistryPackument {
    #[serde(rename = "dist-tags")]
    dist_tags: BTreeMap<String, String>,
}

/// 生产实现：`reqwest` GET `{registry}/{package}`，读 `dist-tags.latest`。
pub struct ReqwestRegistrySource {
    client: reqwest::Client,
}

impl Default for ReqwestRegistrySource {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestRegistrySource {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("build reqwest client"),
        }
    }
}

#[async_trait::async_trait]
impl RegistryLatestVersionSource for ReqwestRegistrySource {
    async fn dist_tags_latest(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<String, RegistryVersionError> {
        let url = format!("{}/{}", registry.trim_end_matches('/'), package);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| RegistryVersionError::Query(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RegistryVersionError::Query(format!(
                "registry responded HTTP {}",
                response.status()
            )));
        }
        let packument: RegistryPackument = response
            .json()
            .await
            .map_err(|error| RegistryVersionError::Query(error.to_string()))?;
        packument.dist_tags.get("latest").cloned().ok_or_else(|| {
            RegistryVersionError::Query("dist-tags.latest missing from packument".to_string())
        })
    }
}

/// 24h durable 缓存优先的 latest 版本解析器。
pub struct RegistryVersionService {
    cache_path: PathBuf,
    source: Arc<dyn RegistryLatestVersionSource>,
}

impl RegistryVersionService {
    pub fn new(
        cache_path: impl Into<PathBuf>,
        source: Arc<dyn RegistryLatestVersionSource>,
    ) -> Self {
        Self {
            cache_path: cache_path.into(),
            source,
        }
    }

    /// 24h 内命中缓存则**不**发 HTTP；未命中/过期则查询并落 durable 缓存。
    pub async fn latest_version(
        &self,
        registry: &str,
        package: &str,
        now: DateTime<Utc>,
    ) -> Result<VersionCacheEntry, RegistryVersionError> {
        if let Some(entry) = self.read_cache().await?.entries.get(package)
            && now.signed_duration_since(entry.checked_at) < VERSION_CACHE_TTL
        {
            return Ok(entry.clone());
        }
        let latest_version = self.source.dist_tags_latest(registry, package).await?;
        let entry = VersionCacheEntry {
            package: package.to_string(),
            latest_version,
            checked_at: now,
        };
        let mut cache = self.read_cache().await?;
        cache.entries.insert(package.to_string(), entry.clone());
        self.write_cache(&cache).await?;
        Ok(entry)
    }

    async fn read_cache(&self) -> Result<VersionCacheFile, RegistryVersionError> {
        match tokio::fs::read(&self.cache_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| RegistryVersionError::Cache(error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(VersionCacheFile::default())
            }
            Err(error) => Err(RegistryVersionError::Cache(error.to_string())),
        }
    }

    async fn write_cache(&self, cache: &VersionCacheFile) -> Result<(), RegistryVersionError> {
        if let Some(parent) = self.cache_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| RegistryVersionError::Cache(error.to_string()))?;
        }
        let bytes = serde_json::to_vec(cache)
            .map_err(|error| RegistryVersionError::Cache(error.to_string()))?;
        // 原子写（temp + rename），沿用 provider_health 的 persist 范式。
        let temp_path = self
            .cache_path
            .with_extension(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
        tokio::fs::write(&temp_path, &bytes)
            .await
            .map_err(|error| RegistryVersionError::Cache(error.to_string()))?;
        tokio::fs::rename(&temp_path, &self.cache_path)
            .await
            .map_err(|error| RegistryVersionError::Cache(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;
    use crate::cross_cutting::semver::compare_semver;

    const REGISTRY: &str = "https://registry.npmmirror.com";
    const PACKAGE: &str = "@openai/codex";

    fn fixed_time(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 12, 0, 0)
            .single()
            .expect("fixed timestamp")
    }

    struct CountingSource {
        calls: AtomicUsize,
        latest: &'static str,
    }

    impl CountingSource {
        fn new(latest: &'static str) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                latest,
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(AtomicOrdering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl RegistryLatestVersionSource for CountingSource {
        async fn dist_tags_latest(
            &self,
            _registry: &str,
            _package: &str,
        ) -> Result<String, RegistryVersionError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(self.latest.to_string())
        }
    }

    struct FailingSource;

    #[async_trait::async_trait]
    impl RegistryLatestVersionSource for FailingSource {
        async fn dist_tags_latest(
            &self,
            _registry: &str,
            _package: &str,
        ) -> Result<String, RegistryVersionError> {
            Err(RegistryVersionError::Query("offline".to_string()))
        }
    }

    /// 测试点 #2：精确 semver 比较（`1.5.0 > 0.85.0`；
    /// `1.5.0-rc.1 < 1.5.0`；`1.5.0 == 1.5.0`）。
    #[test]
    fn registry_version_semver_compare_is_exact() {
        assert_eq!(compare_semver("1.5.0", "0.85.0"), Ordering::Greater);
        assert_eq!(compare_semver("1.5.0-rc.1", "1.5.0"), Ordering::Less);
        assert_eq!(compare_semver("1.5.0", "1.5.0"), Ordering::Equal);
    }

    /// 测试点 #3a：缓存命中（<24h）不发 HTTP。
    #[tokio::test]
    async fn registry_version_cache_hit_within_24h_skips_http() {
        let root = tempdir().expect("root");
        let source = CountingSource::new("2.0.0");
        let service =
            RegistryVersionService::new(root.path().join("version-cache.json"), source.clone());
        let first = service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1))
            .await
            .expect("first query");
        assert_eq!(first.latest_version, "2.0.0");
        assert_eq!(source.calls(), 1);

        let second = service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1) + Duration::hours(23))
            .await
            .expect("cache hit");
        assert_eq!(second.latest_version, "2.0.0");
        assert_eq!(second.checked_at, first.checked_at);
        assert_eq!(source.calls(), 1, "24h 内必须复用缓存，不发 HTTP");
    }

    /// 测试点 #3b：过期（≥24h）重查。
    #[tokio::test]
    async fn registry_version_cache_expires_after_24h_and_requeries() {
        let root = tempdir().expect("root");
        let source = CountingSource::new("2.0.0");
        let service =
            RegistryVersionService::new(root.path().join("version-cache.json"), source.clone());
        service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1))
            .await
            .expect("first query");
        assert_eq!(source.calls(), 1);

        let refreshed = service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1) + Duration::hours(24))
            .await
            .expect("requery after expiry");
        assert_eq!(source.calls(), 2, "满 24h 必须重新查询 registry");
        assert_eq!(
            refreshed.checked_at,
            fixed_time(1) + Duration::hours(24),
            "重查后以新时间戳落缓存"
        );
    }

    /// 测试点 #3c：缓存跨进程可读（重建 service 后仍命中）。
    #[tokio::test]
    async fn registry_version_cache_survives_service_restart() {
        let root = tempdir().expect("root");
        let cache_path = root.path().join("version-cache.json");
        let first_service =
            RegistryVersionService::new(cache_path.clone(), CountingSource::new("1.9.9"));
        first_service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1))
            .await
            .expect("first process query");

        // 模拟进程重启：新的 service 实例 + 新的查询源。
        let restarted_source = CountingSource::new("9.9.9");
        let restarted = RegistryVersionService::new(cache_path, restarted_source.clone());
        let entry = restarted
            .latest_version(REGISTRY, PACKAGE, fixed_time(1) + Duration::hours(1))
            .await
            .expect("cache hit after restart");
        assert_eq!(
            entry.latest_version, "1.9.9",
            "重启后必须命中 durable 缓存而不是重新查询"
        );
        assert_eq!(restarted_source.calls(), 0);
    }

    #[tokio::test]
    async fn registry_version_query_failure_propagates_without_cache() {
        let root = tempdir().expect("root");
        let service = RegistryVersionService::new(
            root.path().join("version-cache.json"),
            Arc::new(FailingSource),
        );
        let result = service
            .latest_version(REGISTRY, PACKAGE, fixed_time(1))
            .await;
        assert!(
            result.is_err(),
            "registry 不可达时错误必须上抛（调用方降级为 latest=None）"
        );
    }
}
