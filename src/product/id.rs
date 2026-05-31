use sha2::{Digest, Sha256};

pub fn repo_hash_for_path(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    hex::encode(digest)[..12].to_string()
}

pub fn next_sequential_id(prefix: &str, existing_len: usize) -> String {
    format!("{prefix}_{:04}", existing_len + 1)
}
