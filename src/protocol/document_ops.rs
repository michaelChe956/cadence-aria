use serde::{Deserialize, Serialize};

pub type Sha256Hex = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DocumentModel {
    pub source_path: String,
    pub sha256: Sha256Hex,
    pub sections: Vec<DocumentSection>,
    #[serde(skip, default)]
    pub source_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DocumentSection {
    pub heading_path: Vec<String>,
    pub level: u8,
    pub start_line: u32,
    pub end_line: u32,
    pub blocks: Vec<DocumentBlock>,
    pub raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "block_kind", content = "content", rename_all = "snake_case")]
pub enum DocumentBlock {
    Paragraph(String),
    BulletList(Vec<String>),
    OrderedList(Vec<String>),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    CodeBlock {
        lang: Option<String>,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HeadingPath(pub Vec<String>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DocumentPatch {
    pub heading_path: HeadingPath,
    pub replacement_blocks: Vec<DocumentBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DocumentPatchResult {
    pub changed: bool,
    pub old_sha256: Sha256Hex,
    pub new_sha256: Sha256Hex,
    pub updated_heading_path: HeadingPath,
    pub warnings: Vec<String>,
}
