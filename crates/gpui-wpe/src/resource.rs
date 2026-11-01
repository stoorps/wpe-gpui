pub use crate::engine::ResourceResponder;
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
static DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);
impl DocumentId {
    pub(crate) fn from_native(id: u64) -> Option<Self> {
        (id != 0).then_some(Self(id))
    }
    pub fn new() -> Self {
        Self(DOCUMENT_ID.fetch_add(1, Ordering::Relaxed))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Debug)]
pub struct ResourceResponse {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}
impl ResourceResponse {
    pub fn new(bytes: impl Into<Vec<u8>>, mime_type: impl Into<String>) -> Result<Self> {
        let mime_type = mime_type.into();
        if mime_type.is_empty()
            || mime_type.len() > 128
            || mime_type.bytes().any(|b| b < 32 || b == 127)
        {
            return Err(Error::InvalidInput("invalid resource MIME type".into()));
        }
        Ok(Self {
            bytes: bytes.into(),
            mime_type,
        })
    }
}
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 160
        || matches!(key, "." | "..")
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err(Error::InvalidInput(
            "resource key must be an opaque ASCII identifier".into(),
        ));
    }
    Ok(())
}
pub fn resource_uri(id: DocumentId, key: &str) -> Result<String> {
    validate_key(key)?;
    Ok(format!("gpui-resource://{}/{key}", id.get()))
}
#[derive(Clone, Debug)]
pub struct ResourceRequest {
    pub document_id: DocumentId,
    pub key: String,
}
#[derive(Clone, Debug)]
pub struct Document {
    id: DocumentId,
    html: String,
    resources: BTreeMap<String, ResourceResponse>,
}
impl Document {
    pub fn html(html: impl Into<String>) -> Self {
        Self {
            id: DocumentId::new(),
            html: html.into(),
            resources: BTreeMap::new(),
        }
    }
    pub fn id(&self) -> DocumentId {
        self.id
    }
    pub fn with_resource(
        mut self,
        key: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
        mime: &str,
    ) -> Result<Self> {
        let key = key.into();
        validate_key(&key)?;
        self.resources
            .insert(key, ResourceResponse::new(bytes, mime)?);
        Ok(self)
    }
    pub fn html_source(&self) -> &str {
        &self.html
    }
    pub fn base_url(&self) -> String {
        format!("gpui-document://{}/body", self.id.get())
    }
    pub fn resources(&self) -> &BTreeMap<String, ResourceResponse> {
        &self.resources
    }
    /// Replace the prepared body without changing its resource namespace.
    /// Call this before the identity is first loaded. An already loaded identity
    /// cannot be reused for different HTML; create a new Document instead.
    /// New load operations still get a distinct request generation internally.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = html.into();
        self
    }
}
#[cfg(test)]
mod tests {
    use super::{Document, validate_key};
    #[test]
    fn opaque_keys_and_unique_origins() {
        let a = Document::html("hi");
        let b = Document::html("hi");
        assert_ne!(a.id(), b.id());
        for key in ["../x", "..", "a/b", "%2e%2e", "a?b", ""] {
            assert!(validate_key(key).is_err());
        }
        assert!(validate_key("image-1.png").is_ok());
    }
}
