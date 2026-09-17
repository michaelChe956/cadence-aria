use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

pub fn repo_hash_for_path(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    hex::encode(digest)[..12].to_string()
}

pub fn next_sequential_id(prefix: &str, existing_len: usize) -> String {
    format!("{prefix}_{:04}", existing_len + 1)
}

pub fn next_sequential_id_from_existing<'a>(
    prefix: &str,
    existing_ids: impl IntoIterator<Item = &'a str>,
) -> String {
    let prefix_with_separator = format!("{prefix}_");
    let max = existing_ids
        .into_iter()
        .filter_map(|id| {
            id.strip_suffix(".json")
                .unwrap_or(id)
                .strip_prefix(&prefix_with_separator)
                .and_then(|suffix| suffix.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0);
    next_sequential_id(prefix, max)
}

pub fn next_sequential_id_in_directory(prefix: &str, directory: &Path) -> io::Result<String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(next_sequential_id(prefix, 0));
        }
        Err(error) => return Err(error),
    };
    let prefix = format!("{prefix}_");
    let mut max = 0;
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let name = name.strip_suffix(".json").unwrap_or(name);
        if let Some(sequence) = name
            .strip_prefix(&prefix)
            .and_then(|suffix| suffix.parse::<usize>().ok())
        {
            max = max.max(sequence);
        }
    }
    Ok(next_sequential_id(prefix.trim_end_matches('_'), max))
}

#[cfg(test)]
mod tests {
    use super::next_sequential_id_in_directory;

    #[test]
    fn directory_sequence_uses_largest_matching_entry() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("issue_0001")).unwrap();
        std::fs::create_dir(tmp.path().join("issue_0003")).unwrap();
        std::fs::write(tmp.path().join("other_9999.json"), "{}").unwrap();

        assert_eq!(
            next_sequential_id_in_directory("issue", tmp.path()).unwrap(),
            "issue_0004"
        );
    }
}
