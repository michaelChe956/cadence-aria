#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalCodebaseFeature {
    enabled: bool,
}

impl LogicalCodebaseFeature {
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }

    pub const fn disabled() -> Self {
        Self { enabled: false }
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::LogicalCodebaseFeature;

    #[test]
    fn feature_flag_reports_its_configured_state() {
        assert!(LogicalCodebaseFeature::enabled().is_enabled());
        assert!(!LogicalCodebaseFeature::disabled().is_enabled());
    }
}
