#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourcePolicy {
    /// Normal web-engine networking. No workload profile is selected.
    #[default]
    Unrestricted,
    /// Only explicitly supplied document resources and data images are available.
    /// Requires JavaScript::Disabled; unsupported settings fail at construction.
    ProvidedOnly,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationRequest {
    pub url: String,
    pub document_id: Option<crate::DocumentId>,
    pub user_gesture: Option<bool>,
    pub new_window: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationDecision {
    Allow,
    Cancel,
    Delegate,
}
