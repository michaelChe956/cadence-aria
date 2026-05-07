#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyPreset {
    ManualAll,
    ManualWrite,
    AutoReview,
    NonInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationDecision {
    PauseForConfirmation,
    RunAutomatically,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeWriteClass {
    ReadOnly,
    WritesRuntime,
    WritesWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNodeMeta {
    pub node_id: String,
    pub provider_type: String,
    pub write_class: NodeWriteClass,
}

impl ProviderNodeMeta {
    pub fn new(
        node_id: impl Into<String>,
        provider_type: impl Into<String>,
        write_class: NodeWriteClass,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            provider_type: provider_type.into(),
            write_class,
        }
    }
}

impl PolicyPreset {
    pub fn decision_for(self, node: &ProviderNodeMeta) -> ConfirmationDecision {
        match self {
            PolicyPreset::ManualAll => ConfirmationDecision::PauseForConfirmation,
            PolicyPreset::ManualWrite => match node.write_class {
                NodeWriteClass::ReadOnly => ConfirmationDecision::RunAutomatically,
                NodeWriteClass::WritesRuntime | NodeWriteClass::WritesWorkspace => {
                    ConfirmationDecision::PauseForConfirmation
                }
            },
            PolicyPreset::AutoReview => {
                if matches!(node.node_id.as_str(), "N11" | "N12" | "N16" | "N19") {
                    ConfirmationDecision::PauseForConfirmation
                } else {
                    ConfirmationDecision::RunAutomatically
                }
            }
            PolicyPreset::NonInteractive => ConfirmationDecision::RunAutomatically,
        }
    }
}

impl std::str::FromStr for PolicyPreset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual-all" => Ok(Self::ManualAll),
            "manual-write" => Ok(Self::ManualWrite),
            "auto-review" => Ok(Self::AutoReview),
            "non-interactive" => Ok(Self::NonInteractive),
            other => Err(format!("unsupported policy preset: {other}")),
        }
    }
}
