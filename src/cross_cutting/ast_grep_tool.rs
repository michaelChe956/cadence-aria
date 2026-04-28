use crate::cross_cutting::document_ops::DocumentOpError;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstGrepCapability {
    pub command: String,
    pub version: String,
}

pub fn probe_ast_grep() -> Result<AstGrepCapability, DocumentOpError> {
    let output = Command::new("ast-grep").arg("--version").output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(_) => return Err(DocumentOpError::MissingOptionalTool("ast-grep".to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DocumentOpError::MissingOptionalTool("ast-grep".to_string()));
        }
        Err(error) => return Err(DocumentOpError::IoError(error.to_string())),
    };

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(AstGrepCapability {
        command: "ast-grep".to_string(),
        version,
    })
}
