use std::path::PathBuf;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::image_create::ImageCreateError;

pub fn media_type_extension(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpeg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

pub fn valid_image_id(image_id: &str) -> bool {
    uuid::Uuid::parse_str(image_id).is_ok()
}

pub async fn write_image_atomic(
    paths: &AriaStatePaths,
    image_id: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<PathBuf, ImageCreateError> {
    let destination = paths
        .image_create_image_file(image_id, media_type)
        .ok_or_else(|| {
            ImageCreateError::Store(format!(
                "invalid image id or media type: {image_id}/{media_type}"
            ))
        })?;
    let parent = destination.parent().expect("image file has parent");
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        ImageCreateError::Store(format!("create {}: {error}", parent.display()))
    })?;
    let temp_path = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    use tokio::io::AsyncWriteExt;
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .map_err(|error| {
                ImageCreateError::Store(format!("create {}: {error}", temp_path.display()))
            })?;
        file.write_all(bytes).await.map_err(|error| {
            ImageCreateError::Store(format!("write {}: {error}", temp_path.display()))
        })?;
        file.sync_all().await.map_err(|error| {
            ImageCreateError::Store(format!("sync {}: {error}", temp_path.display()))
        })?;
        drop(file);
        tokio::fs::rename(&temp_path, &destination)
            .await
            .map_err(|error| {
                ImageCreateError::Store(format!("rename {}: {error}", destination.display()))
            })
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    result.map(|_| destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;

    fn service(root: &std::path::Path) -> AriaStatePaths {
        AriaStatePaths::from_workspace_root(root)
    }

    #[tokio::test]
    async fn write_image_atomic_persists_bytes_and_rejects_bad_input() {
        let root = tempfile::tempdir().expect("root");
        let paths = service(root.path());
        let id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";

        let written = write_image_atomic(&paths, id, "image/png", b"png-bytes")
            .await
            .expect("write ok");
        assert_eq!(
            written,
            paths.image_create_images_dir().join(format!("{id}.png"))
        );
        assert_eq!(
            tokio::fs::read(&written).await.expect("read back"),
            b"png-bytes"
        );
        let left: Vec<String> = std::fs::read_dir(paths.image_create_images_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.is_empty());

        assert!(
            write_image_atomic(&paths, "../escape", "image/png", b"x")
                .await
                .is_err()
        );
        assert!(
            write_image_atomic(&paths, id, "image/gif", b"x")
                .await
                .is_err()
        );
    }
}
