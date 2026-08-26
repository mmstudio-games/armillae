/// The canonical message/content position associated with a compatibility fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageContentLocation {
    pub message_index: usize,
    pub content_index: usize,
}

/// An explicit action taken while projecting a canonical request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompatibilityAction {
    /// Provider-private content was retained in canonical history but not sent to the target.
    NotForwarded,
}

/// A safe, content-free fact produced by target Provider projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityFact {
    pub location: MessageContentLocation,
    pub source_provider: String,
    pub target_provider: String,
    pub kind: String,
    pub action: CompatibilityAction,
    pub lossy: bool,
}

/// The observable result of projecting one canonical request to a target Provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionReport {
    pub target_provider: String,
    pub facts: Vec<CompatibilityFact>,
}

impl ProjectionReport {
    pub fn exact(target_provider: impl Into<String>) -> Self {
        Self {
            target_provider: target_provider.into(),
            facts: Vec::new(),
        }
    }

    pub fn is_exact(&self) -> bool {
        self.facts.is_empty()
    }
}
