use std::path::PathBuf;

use tower_http::services::{ServeDir, ServeFile};

pub fn static_dist_service() -> ServeDir<ServeFile> {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist");
    let index = dist.join("index.html");
    ServeDir::new(dist).fallback(ServeFile::new(index))
}
