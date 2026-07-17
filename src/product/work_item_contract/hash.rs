use sha2::{Digest, Sha256};

use crate::product::json_store::ProductStoreError;

use super::CanonicalWorkItemContract;

pub fn canonical_contract_hash(
    contract: &CanonicalWorkItemContract,
) -> Result<String, ProductStoreError> {
    let bytes =
        serde_json::to_vec(contract).map_err(|error| ProductStoreError::Json(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(hex::encode(digest))
}
