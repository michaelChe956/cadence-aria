use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};

pub const PRODUCT_DATA_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDataSchema {
    pub product_data_schema_version: u32,
}

pub fn ensure_product_data_schema(
    paths: &ProductAppPaths,
) -> Result<ProductDataSchema, ProductStoreError> {
    let schema_path = paths.product_data_schema_path();
    if schema_path.is_file() {
        let schema: ProductDataSchema = read_json(&schema_path)?;
        if schema.product_data_schema_version == PRODUCT_DATA_SCHEMA_VERSION {
            return Ok(schema);
        }
        return Err(ProductStoreError::Io(format!(
            "product_data_schema_unsupported: expected v{PRODUCT_DATA_SCHEMA_VERSION}, found v{}",
            schema.product_data_schema_version
        )));
    }

    let legacy_business_data_exists = [paths.projects_root(), paths.state_root()]
        .into_iter()
        .filter(|path| path.exists())
        .map(std::fs::read_dir)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ProductStoreError::Io(error.to_string()))?
        .into_iter()
        .any(|mut entries| entries.next().is_some());

    if legacy_business_data_exists {
        return Err(ProductStoreError::Io(
            "product_data_schema_unsupported: legacy business data exists".to_string(),
        ));
    }

    let schema = ProductDataSchema {
        product_data_schema_version: PRODUCT_DATA_SCHEMA_VERSION,
    };
    write_json(&schema_path, &schema)?;
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::{ProductDataSchema, ensure_product_data_schema};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::{read_json, write_json};

    #[test]
    fn product_data_schema_creates_v2_for_empty_root() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));

        let schema = ensure_product_data_schema(&paths).unwrap();

        assert_eq!(schema.product_data_schema_version, 2);
        assert_eq!(
            read_json::<ProductDataSchema>(&paths.product_data_schema_path()).unwrap(),
            schema
        );
    }

    #[test]
    fn product_data_schema_accepts_existing_v2() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let expected = ProductDataSchema {
            product_data_schema_version: 2,
        };
        write_json(&paths.product_data_schema_path(), &expected).unwrap();

        let schema = ensure_product_data_schema(&paths).unwrap();

        assert_eq!(schema, expected);
    }

    #[test]
    fn product_data_schema_rejects_legacy_business_data() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        std::fs::create_dir_all(paths.projects_root()).unwrap();
        write_json(
            &paths.projects_root().join("legacy.json"),
            &serde_json::json!({"legacy": true}),
        )
        .unwrap();

        let error = ensure_product_data_schema(&paths).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("product_data_schema_unsupported")
        );
    }

    #[test]
    fn product_data_schema_rejects_missing_schema_when_coding_data_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let attempt_root = paths
            .issue_root("project_0001", "issue_0001")
            .join("coding-attempts");
        std::fs::create_dir_all(&attempt_root).unwrap();
        write_json(
            &attempt_root.join("coding_attempt_0001.json"),
            &serde_json::json!({"id": "coding_attempt_0001"}),
        )
        .unwrap();

        let error = ensure_product_data_schema(&paths).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("product_data_schema_unsupported")
        );
    }
}
