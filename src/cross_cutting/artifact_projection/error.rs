use crate::protocol::artifacts::ProjectionKind;
use crate::protocol::document_ops::HeadingPath;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionCompileError {
    MissingRequiredSection {
        heading_path: HeadingPath,
    },
    InvalidIdFormat {
        id: String,
        expected_pattern: String,
    },
    DuplicateId {
        id: String,
        section: String,
    },
    ReferenceUnknown {
        ref_id: String,
        context: String,
    },
    PriorityInvalid {
        value: String,
    },
    ExecutionModeInvalid {
        value: String,
    },
    DependencyEndpointMissing {
        from: String,
        to: String,
    },
    EmptyPayload {
        projection_kind: ProjectionKind,
    },
}

impl fmt::Display for ProjectionCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectionCompileError::MissingRequiredSection { heading_path } => {
                write!(
                    formatter,
                    "missing required projection section {}",
                    heading_path
                        .0
                        .last()
                        .map(String::as_str)
                        .unwrap_or("<unknown>")
                )
            }
            ProjectionCompileError::InvalidIdFormat {
                id,
                expected_pattern,
            } => write!(
                formatter,
                "invalid projection id {id}, expected {expected_pattern}"
            ),
            ProjectionCompileError::DuplicateId { id, section } => {
                write!(formatter, "duplicate projection id {id} in {section}")
            }
            ProjectionCompileError::ReferenceUnknown { ref_id, context } => {
                write!(
                    formatter,
                    "unknown projection reference {ref_id} in {context}"
                )
            }
            ProjectionCompileError::PriorityInvalid { value } => {
                write!(formatter, "invalid requirement priority {value}")
            }
            ProjectionCompileError::ExecutionModeInvalid { value } => {
                write!(formatter, "invalid execution mode {value}")
            }
            ProjectionCompileError::DependencyEndpointMissing { from, to } => {
                write!(formatter, "dependency endpoint missing: {from} -> {to}")
            }
            ProjectionCompileError::EmptyPayload { projection_kind } => {
                write!(formatter, "empty projection payload {projection_kind:?}")
            }
        }
    }
}

impl std::error::Error for ProjectionCompileError {}
