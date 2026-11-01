#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JavaScript {
    #[default]
    Enabled,
    Disabled,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}
/// Settings fixed at view construction. Loading a document never changes them.
#[derive(Clone, Debug)]
pub struct WebViewSettings {
    pub javascript: JavaScript,
    /// Auto follows WPE's process-wide preference. Independent Light/Dark is
    /// unsupported by this WPE API and rejected before native allocation.
    pub theme: Theme,
    pub background: [f64; 4],
    pub max_document_bytes: usize,
    pub max_resource_bytes: usize,
    pub max_total_resource_bytes: usize,
}
impl Default for WebViewSettings {
    fn default() -> Self {
        Self {
            javascript: JavaScript::Enabled,
            theme: Theme::Auto,
            background: [1., 1., 1., 1.],
            max_document_bytes: 4 * 1024 * 1024,
            max_resource_bytes: 16 * 1024 * 1024,
            max_total_resource_bytes: 32 * 1024 * 1024,
        }
    }
}

impl WebViewSettings {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.theme != Theme::Auto {
            return Err(crate::Error::Unsupported(
                "independent per-view color-scheme preference on WPE",
            ));
        }
        if self
            .background
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(crate::Error::InvalidInput(
                "background channels must be finite values from 0 to 1".into(),
            ));
        }
        if self.max_document_bytes > 8 * 1024 * 1024
            || self.max_resource_bytes > 32 * 1024 * 1024
            || self.max_total_resource_bytes > 64 * 1024 * 1024
        {
            return Err(crate::Error::InvalidInput("configured byte limits exceed native renderer hard caps (8MiB document, 32MiB resource, 64MiB aggregate)".into()));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_fail_before_native_construction() {
        let mut s = WebViewSettings::default();
        assert!(s.validate().is_ok());
        s.max_document_bytes = 8 * 1024 * 1024 + 1;
        assert!(s.validate().is_err());
        s = WebViewSettings::default();
        s.background[0] = f64::NAN;
        assert!(s.validate().is_err());
    }
}
