//! Application-layer email preparation. No email policy is built into gpui-wpe.
//!
//! Callers supply decoded MIME content and an explicit Content-ID resource map.
//! This crate never parses mailbox protocols or grants WebKit network access.
//! The blocking remote image fetcher belongs on an application worker thread.
mod css;
mod markup;
mod remote;
pub use remote::{RemoteFetcher, RemoteLoadReport};

use base64::Engine as _;
use image::{ImageDecoder as _, ImageEncoder as _};
use kuchiki::traits::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use url::Url;

#[derive(Debug)]
pub enum Error {
    InvalidDocument,
    Limit(&'static str),
    InvalidImage(String),
    InvalidCid,
    ConsentMismatch,
    Cancelled,
    Network(String),
    UrlPolicy(&'static str),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDocument => f.write_str("document ID must be nonzero"),
            Self::Limit(name) => write!(f, "email resource limit exceeded: {name}"),
            Self::InvalidImage(reason) => write!(f, "unsupported or invalid image: {reason}"),
            Self::InvalidCid => f.write_str("invalid Content-ID"),
            Self::ConsentMismatch => {
                f.write_str("remote-content consent belongs to a different document")
            }
            Self::Cancelled => f.write_str("remote-content consent was revoked"),
            Self::Network(reason) => write!(f, "remote image request failed: {reason}"),
            Self::UrlPolicy(reason) => write!(f, "remote image URL denied: {reason}"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_html_bytes: usize,
    pub max_resource_bytes: usize,
    pub max_total_resource_bytes: usize,
    pub max_images: usize,
    pub max_html_nodes: usize,
    /// Maximum parsed DOM depth, including the parser's html/body elements.
    /// Checked iteratively before recursive HTML serialization.
    pub max_html_depth: usize,
    pub max_image_pixels: u64,
    pub max_decode_bytes: u64,
    /// Aggregate normalized-image footprint, accounting for at least RGBA8.
    pub max_total_decoded_image_bytes: u64,
    pub max_redirects: usize,
    pub request_timeout: Duration,
    pub dns_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_html_bytes: 4 * 1024 * 1024,
            max_resource_bytes: 8 * 1024 * 1024,
            max_total_resource_bytes: 32 * 1024 * 1024,
            max_images: 64,
            max_html_nodes: 50_000,
            max_html_depth: 128,
            max_image_pixels: 8_000_000,
            max_decode_bytes: 64 * 1024 * 1024,
            max_total_decoded_image_bytes: 128 * 1024 * 1024,
            max_redirects: 3,
            request_timeout: Duration::from_secs(15),
            dns_timeout: Duration::from_secs(3),
        }
    }
}

/// A validated image normalized to a static PNG. Animation, metadata and SVG are
/// deliberately excluded from the initial reading-pane contract.
#[derive(Clone, Debug)]
pub struct InlineResource {
    pub bytes: Vec<u8>,
    pub mime: String,
}
impl InlineResource {
    pub fn new(bytes: Vec<u8>, mime: &str, limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.max_resource_bytes {
            return Err(Error::Limit("encoded image bytes"));
        }
        let declared = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let expected = match declared.as_str() {
            "image/png" => image::ImageFormat::Png,
            "image/jpeg" => image::ImageFormat::Jpeg,
            "image/gif" => image::ImageFormat::Gif,
            "image/webp" => image::ImageFormat::WebP,
            _ => {
                return Err(Error::InvalidImage(
                    "only PNG/JPEG/GIF/WebP raster input is accepted".into(),
                ));
            }
        };
        let reader = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        if reader.format() != Some(expected) {
            return Err(Error::InvalidImage(
                "declared MIME does not match encoded image".into(),
            ));
        }
        let (width, height) = reader
            .into_dimensions()
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > limits.max_image_pixels
        {
            return Err(Error::Limit("decoded image pixels"));
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), expected);
        let mut decode_limits = image::Limits::default();
        decode_limits.max_alloc = Some(limits.max_decode_bytes);
        reader.limits(decode_limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        let orientation = decoder
            .orientation()
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        let mut image = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        image.apply_orientation(orientation);
        let (width, height) = (image.width(), image.height());
        let mut png = BoundedImageOutput {
            bytes: Vec::new(),
            limit: limits.max_resource_bytes,
        };
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(image.as_bytes(), width, height, image.color().into())
            .map_err(|e| Error::InvalidImage(e.to_string()))?;
        Ok(Self {
            bytes: png.bytes,
            mime: "image/png".into(),
        })
    }
}
fn decoded_image_bytes(resource: &InlineResource) -> Result<u64> {
    let decoder =
        image::ImageReader::with_format(Cursor::new(&resource.bytes), image::ImageFormat::Png)
            .into_decoder()
            .map_err(|error| Error::InvalidImage(error.to_string()))?;
    let (width, height) = decoder.dimensions();
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| {
            pixels.checked_mul(u64::from(decoder.color_type().bytes_per_pixel().max(4)))
        })
        .ok_or(Error::Limit("decoded resource footprint"))
}
fn resource_budget<'a>(
    resources: impl Iterator<Item = &'a InlineResource>,
    limits: &Limits,
) -> Result<(usize, u64)> {
    let mut encoded = 0usize;
    let mut decoded = 0u64;
    for resource in resources {
        encoded = encoded
            .checked_add(resource.bytes.len())
            .ok_or(Error::Limit("total image bytes"))?;
        decoded = decoded
            .checked_add(decoded_image_bytes(resource)?)
            .ok_or(Error::Limit("total decoded image bytes"))?;
        if encoded > limits.max_total_resource_bytes {
            return Err(Error::Limit("total image bytes"));
        }
        if decoded > limits.max_total_decoded_image_bytes {
            return Err(Error::Limit("total decoded image bytes"));
        }
    }
    Ok((encoded, decoded))
}
struct BoundedImageOutput {
    bytes: Vec<u8>,
    limit: usize,
}
impl std::io::Write for BoundedImageOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("normalized image limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct RemoteImage {
    pub key: String,
    pub url: Url,
}
/// Consent is intentionally scoped to exactly one prepared document. The app
/// creates this only after its user-facing consent action, never from message HTML.
#[derive(Clone, Debug)]
pub struct RemoteConsent {
    document_id: u64,
    revoked: Arc<AtomicBool>,
}
impl RemoteConsent {
    pub fn for_document(document_id: u64) -> Self {
        Self {
            document_id,
            revoked: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug)]
pub struct PreparedEmail {
    pub document_id: u64,
    pub html: String,
    pub resources: BTreeMap<String, InlineResource>,
    pub remote_images: Vec<RemoteImage>,
    /// Render with the host's own accessible text element if WebKit is unavailable
    /// or must be disabled. Do not use a vulnerable WebKit for a security fallback.
    pub fallback_text: String,
    pub fallback_html: String,
    limits: Limits,
}
impl PreparedEmail {
    /// Fetch only after an application consent action, on a worker thread. Reload
    /// the document with the returned resource map after approval so previously
    /// failed placeholders are requested again. Partial failures remain blocked.
    pub fn approve_remote_images(
        &mut self,
        consent: &RemoteConsent,
        fetcher: &RemoteFetcher,
    ) -> Result<RemoteLoadReport> {
        self.approve_remote_images_with(consent, |url, token| {
            fetcher.fetch_with_consent(url, token)
        })
    }
    /// Application-supplied fetchers support proxies/caches and deterministic
    /// testing. The app remains responsible for their URL/network policy; returned
    /// bytes are always revalidated against this document's image limits.
    pub fn approve_remote_images_with(
        &mut self,
        consent: &RemoteConsent,
        mut fetch: impl FnMut(&Url, &RemoteConsent) -> Result<InlineResource>,
    ) -> Result<RemoteLoadReport> {
        if consent.document_id != self.document_id {
            return Err(Error::ConsentMismatch);
        }
        if consent.is_revoked() {
            return Err(Error::Cancelled);
        }
        let (mut encoded_total, mut decoded_total) =
            resource_budget(self.resources.values(), &self.limits)?;
        let mut report = RemoteLoadReport::default();
        for image in &self.remote_images {
            if consent.is_revoked() {
                return Err(Error::Cancelled);
            }
            if self.resources.contains_key(&image.key) {
                continue;
            }
            let fetched = fetch(&image.url, consent);
            if consent.is_revoked() {
                return Err(Error::Cancelled);
            }
            match fetched {
                Ok(resource) => {
                    let resource =
                        InlineResource::new(resource.bytes, &resource.mime, &self.limits)?;
                    let decoded = decoded_image_bytes(&resource)?;
                    if encoded_total.saturating_add(resource.bytes.len())
                        > self.limits.max_total_resource_bytes
                        || decoded_total.saturating_add(decoded)
                            > self.limits.max_total_decoded_image_bytes
                    {
                        report.failed.push((
                            image.key.clone(),
                            "aggregate encoded/decoded resource limit".into(),
                        ));
                        continue;
                    }
                    encoded_total += resource.bytes.len();
                    decoded_total += decoded;
                    self.resources.insert(image.key.clone(), resource);
                    report.loaded += 1;
                }
                Err(error) => report.failed.push((image.key.clone(), error.to_string())),
            }
        }
        Ok(report)
    }
}

pub fn resource_uri(document_id: u64, key: &str) -> Result<String> {
    if document_id == 0 {
        return Err(Error::InvalidDocument);
    }
    if key.is_empty()
        || matches!(key, "." | "..")
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(Error::InvalidCid);
    }
    Ok(format!("gpui-resource://{document_id}/{key}"))
}
fn content_id(input: &str) -> Result<String> {
    let input = input.trim().trim_start_matches('<').trim_end_matches('>');
    let value = percent_encoding::percent_decode_str(input)
        .decode_utf8()
        .map_err(|_| Error::InvalidCid)?;
    if value.is_empty()
        || value.len() > 512
        || value.contains(['/', '\\'])
        || value.contains("..")
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error::InvalidCid);
    }
    Ok(value.into_owned())
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn bounded_text(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
pub fn plain_text_html(text: &str, max_bytes: usize) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><pre style=\"white-space:pre-wrap;overflow-wrap:anywhere\">{}</pre>",
        escape(&bounded_text(text, max_bytes))
    )
}
/// URLs are returned as navigation intentions. This never opens an application.
pub fn allowed_link(input: &str) -> Option<String> {
    if input.starts_with('#') && !input.chars().any(char::is_control) {
        return Some(input.into());
    }
    let url = Url::parse(input).ok()?;
    if !matches!(url.scheme(), "https" | "http" | "mailto")
        || !url.username().is_empty()
        || url.password().is_some()
        || input.chars().any(char::is_control)
    {
        return None;
    }
    Some(url.into())
}

fn text_from_html(cleaned: &str) -> String {
    let document = kuchiki::parse_html().one(cleaned).document_node;
    let mut text = String::new();
    for node in document.descendants() {
        match node.data() {
            kuchiki::NodeData::Text(content) => text.push_str(&content.borrow()),
            kuchiki::NodeData::Element(element)
                if matches!(
                    element.name.local.as_ref(),
                    "p" | "div" | "tr" | "br" | "li" | "h1" | "h2" | "h3" | "h4" | "blockquote"
                ) =>
            {
                text.push('\n')
            }
            _ => {}
        }
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn prepare_email(
    document_id: u64,
    html: &str,
    plain_text: &str,
    inline: &BTreeMap<String, InlineResource>,
    limits: &Limits,
) -> Result<PreparedEmail> {
    if document_id == 0 {
        return Err(Error::InvalidDocument);
    }
    if limits.max_html_depth == 0 || limits.max_html_depth > 256 {
        return Err(Error::Limit("HTML depth configuration (1..=256)"));
    }
    if html.len() > limits.max_html_bytes {
        return Err(Error::Limit("HTML bytes"));
    }
    if inline.len() > limits.max_images {
        return Err(Error::Limit("image count"));
    }
    let mut cid_map = HashMap::new();
    for (id, resource) in inline {
        if cid_map.insert(content_id(id)?, resource).is_some() {
            // Ambiguous MIME identifiers must not depend on map ordering.
            return Err(Error::InvalidCid);
        }
    }
    if html.bytes().filter(|b| *b == b'<').count() > limits.max_html_nodes {
        return Err(Error::Limit("HTML node estimate"));
    }
    markup::preflight(html)?;
    let document = kuchiki::parse_html().one(html).document_node;
    let (mut depth, mut nodes) = (0usize, 0usize);
    for edge in document.traverse() {
        match edge {
            kuchiki::iter::NodeEdge::Start(_) => {
                depth += 1;
                nodes += 1;
                if depth > limits.max_html_depth {
                    return Err(Error::Limit("HTML depth"));
                }
                if nodes > limits.max_html_nodes {
                    return Err(Error::Limit("HTML nodes"));
                }
            }
            kuchiki::iter::NodeEdge::End(_) => depth -= 1,
        }
    }
    let mut resources = BTreeMap::new();
    let mut remote_images = Vec::new();
    let mut remote_keys = HashMap::<String, String>::new();
    let mut cid_keys = HashMap::<String, String>::new();
    let mut count = 0usize;
    let mut element_ids = HashSet::new();
    // Work on parsed attributes, then pass all output through the HTML sanitizer.
    for node in document.select("*").expect("static selector") {
        let mut attrs = node.attributes.borrow_mut();
        if let Some(id) = attrs.get("id").map(str::to_owned)
            && (id.is_empty()
                || id.len() > 128
                || id.chars().any(|c| c.is_control() || c.is_whitespace())
                || !element_ids.insert(id))
        {
            attrs.remove("id");
        }
        if let Some(style) = attrs.get("style").map(str::to_owned) {
            let safe = css::sanitize_style(&style);
            if safe.is_empty() {
                attrs.remove("style");
            } else {
                attrs.insert("style", safe);
            }
        }
        if let Some(href) = attrs.get("href").map(str::to_owned) {
            if let Some(safe) = allowed_link(&href) {
                attrs.insert("href", safe);
            } else {
                attrs.remove("href");
            }
        }
        // The consumer intentionally supports no srcset, CSS resources, embedded
        // documents, media, or background-image URLs in this first release.
        attrs.remove("srcset");
        attrs.remove("background");
        attrs.remove("poster");
        for (name, max) in [
            ("width", 16_384u32),
            ("height", 16_384),
            ("cellpadding", 256),
            ("cellspacing", 256),
            ("border", 32),
            ("colspan", 128),
            ("rowspan", 128),
        ] {
            if let Some(value) = attrs.get(name) {
                let stripped = value.trim().strip_suffix('%').unwrap_or(value.trim());
                if stripped.parse::<u32>().ok().is_none_or(|v| v > max) {
                    attrs.remove(name);
                }
            }
        }
        if node.name.local.as_ref() != "img" {
            continue;
        }
        count += 1;
        if count > limits.max_images {
            return Err(Error::Limit("image count"));
        }
        let Some(src) = attrs.get("src").map(str::to_owned) else {
            continue;
        };
        let mut key = None;
        if src.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("cid:")) {
            if let Ok(id) = content_id(&src[4..])
                && let Some(resource) = cid_map.get(&id)
            {
                let name = cid_keys
                    .entry(id)
                    .or_insert_with(|| format!("inline-{}", resources.len()))
                    .clone();
                if !resources.contains_key(&name) {
                    let resource =
                        InlineResource::new(resource.bytes.clone(), &resource.mime, limits)?;
                    resources.insert(name.clone(), resource);
                }
                key = Some(name);
            }
        } else if let Some(data_uri) = src.strip_prefix("data:") {
            if let Some((header, data)) = data_uri.split_once(',') {
                let parts: Vec<_> = header.split(';').collect();
                if parts.len() == 2
                    && parts[1].eq_ignore_ascii_case("base64")
                    && data.len() <= limits.max_resource_bytes.saturating_mul(2)
                    && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
                    && let Ok(resource) = InlineResource::new(bytes, parts[0], limits)
                {
                    let name = format!("inline-{}", resources.len());
                    resources.insert(name.clone(), resource);
                    key = Some(name);
                }
            }
        } else if let Ok(url) = Url::parse(&src)
            && matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
        {
            let normalized = url.to_string();
            let name = if let Some(name) = remote_keys.get(&normalized) {
                name.clone()
            } else {
                let name = format!("remote-{}", remote_images.len());
                remote_images.push(RemoteImage {
                    key: name.clone(),
                    url,
                });
                remote_keys.insert(normalized, name.clone());
                name
            };
            key = Some(name);
        }
        attrs.remove("src");
        if let Some(key) = key {
            attrs.insert("src", resource_uri(document_id, &key)?);
        }
    }
    resource_budget(resources.values(), limits)?;
    let mut bytes = Vec::new();
    document
        .serialize(&mut bytes)
        .map_err(|e| Error::Network(e.to_string()))?;
    let rewritten = String::from_utf8(bytes).expect("HTML serializer emits UTF-8");
    let mut sanitizer = ammonia::Builder::default();
    sanitizer
        .add_tags([
            "table", "thead", "tbody", "tfoot", "tr", "td", "th", "colgroup", "col", "img",
        ])
        .add_generic_attributes(["style", "dir", "lang", "id"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "gpui-resource"]))
        .link_rel(Some("noreferrer noopener"));
    for tag in ["table", "tr", "td", "th", "col", "img"] {
        sanitizer.add_tag_attributes(
            tag,
            [
                "width",
                "height",
                "align",
                "valign",
                "bgcolor",
                "cellpadding",
                "cellspacing",
                "border",
                "colspan",
                "rowspan",
                "alt",
            ],
        );
    }
    let cleaned = sanitizer.clean(&rewritten).to_string();
    // Sanitization can discard entire foreign/active subtrees after resource
    // rewriting. Never fetch a remote image that has no surviving image element.
    let final_document = kuchiki::parse_html().one(cleaned.as_str()).document_node;
    let surviving_sources: HashSet<String> = final_document
        .select("img")
        .expect("static selector")
        .filter_map(|node| node.attributes.borrow().get("src").map(str::to_owned))
        .collect();
    resources.retain(|key, _| {
        surviving_sources.contains(&format!("gpui-resource://{document_id}/{key}"))
    });
    remote_images.retain(|image| {
        surviving_sources.contains(&format!("gpui-resource://{document_id}/{}", image.key))
    });
    let fallback = if plain_text.trim().is_empty() {
        text_from_html(&cleaned)
    } else {
        plain_text.to_owned()
    };
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src gpui-resource:; style-src 'unsafe-inline'; font-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\"><title>Email message</title>{cleaned}"
    );
    Ok(PreparedEmail {
        document_id,
        html,
        resources,
        remote_images,
        fallback_text: bounded_text(&fallback, limits.max_html_bytes),
        fallback_html: plain_text_html(&fallback, limits.max_html_bytes),
        limits: limits.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_aggregate_limits_inline_and_consented_images() {
        let limits = Limits {
            max_total_decoded_image_bytes: 16,
            ..Default::default()
        };
        let image = InlineResource::new(png(), "image/png", &limits).unwrap();
        let inline = BTreeMap::from([("a".into(), image.clone()), ("b".into(), image.clone())]);
        assert!(matches!(
            prepare_email(
                1,
                "<img src='cid:a'><img src='cid:b'>",
                "",
                &inline,
                &limits
            ),
            Err(Error::Limit("total decoded image bytes"))
        ));
        let mut email = prepare_email(
            2,
            "<img src='https://example.test/a'><img src='https://example.test/b'>",
            "",
            &BTreeMap::new(),
            &limits,
        )
        .unwrap();
        let report = email
            .approve_remote_images_with(&RemoteConsent::for_document(2), |_, _| Ok(image.clone()))
            .unwrap();
        assert_eq!(report.loaded, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(email.resources.len(), 1);
    }
    #[test]
    fn normalized_image_applies_exif_orientation_and_drops_metadata() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            1,
            image::Rgb([180, 30, 90]),
        ));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image)
            .unwrap();
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut with_exif = jpeg[..2].to_vec();
        with_exif.extend_from_slice(&[0xff, 0xe1]);
        with_exif.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        with_exif.extend_from_slice(exif);
        with_exif.extend_from_slice(&jpeg[2..]);
        let normalized = InlineResource::new(with_exif, "image/jpeg", &Limits::default()).unwrap();
        let decoded = image::load_from_memory(&normalized.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 2));
        assert!(!normalized.bytes.windows(4).any(|chunk| chunk == b"eXIf"));
    }
    fn png() -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([30, 60, 90, 255]),
        ));
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }
    fn prepare(html: &str) -> PreparedEmail {
        prepare_email(
            17,
            html,
            "fallback <text>",
            &BTreeMap::new(),
            &Limits::default(),
        )
        .unwrap()
    }
    #[test]
    fn scripts_event_handlers_and_replacement_policy_are_removed() {
        let mail = prepare(
            r#"<meta http-equiv="Content-Security-Policy" content="default-src *"><base href="https://evil.test"><script>alert(1)</script><iframe src="https://evil.test/frame"></iframe><img src="https://evil.test/image" onerror="alert(2)"><form action="https://evil.test/form"><input></form><a href="javascript:alert(3)">bad</a>"#,
        );
        assert_eq!(mail.html.matches("Content-Security-Policy").count(), 1);
        for item in [
            "<script",
            "onerror",
            "<iframe",
            "<form",
            "javascript:",
            "<base ",
        ] {
            assert!(!mail.html.contains(item), "{item}: {}", mail.html);
        }
        assert_eq!(mail.remote_images.len(), 1);
        assert!(mail.html.contains("gpui-resource://17/remote-0"));
        assert!(!mail.html.contains("https://evil.test/image"));
        assert!(mail.resources.is_empty());
    }
    #[test]
    fn tables_and_safe_inline_layout_survive() {
        let mail = prepare(
            r#"<table width="600" cellpadding="16"><tr><td style="color:#123456;max-width:600px;padding:12px;background:u\72l(https://bad.test/a)">Newsletter שלום 中文 👋</td></tr></table>"#,
        );
        assert!(mail.html.contains("<table"));
        assert!(mail.html.contains("cellpadding=\"16\""));
        assert!(mail.html.contains("max-width:600px"));
        assert!(mail.html.contains("Newsletter שלום 中文 👋"));
        assert!(!mail.html.contains("bad.test"));
    }
    #[test]
    fn only_supplied_cid_can_become_a_resource() {
        let mut inline = BTreeMap::new();
        inline.insert(
            "<hero@example.test>".into(),
            InlineResource::new(png(), "image/png", &Limits::default()).unwrap(),
        );
        let mail=prepare_email(9,r#"<img src="cid:hero%40example.test" alt="hero"><img src="cid:../../etc/passwd" alt="bad"><img src="gpui-resource://42/secret">"#,"plain",&inline,&Limits::default()).unwrap();
        assert_eq!(mail.resources.len(), 1);
        assert!(mail.html.contains("gpui-resource://9/inline-0"));
        assert!(!mail.html.contains("passwd"));
        assert!(!mail.html.contains("gpui-resource://42"));
    }
    #[test]
    fn unknown_and_malicious_image_sources_become_placeholders() {
        let mail = prepare(
            r#"<img src="file:///etc/passwd" alt="file"><img src="//tracker.test/a"><img src="data:image/svg+xml;base64,PHN2Zy8+"><img srcset="https://tracker.test/b 2x" background="https://tracker.test/c">"#,
        );
        assert!(mail.resources.is_empty());
        assert!(mail.remote_images.is_empty());
        assert!(!mail.html.contains("file:"));
        assert!(!mail.html.contains("tracker.test"));
        assert!(!mail.html.contains("image/svg"));
        assert!(mail.html.contains("alt=\"file\""));
    }
    #[test]
    fn external_styles_fonts_and_css_resources_do_not_survive() {
        let mail = prepare(
            r#"<style>@import 'https://bad.test/import';@font-face{src:url(https://bad.test/font)}</style><link rel="stylesheet" href="https://bad.test/css"><p style="font-family:var(--remote);color:blue;background-image:url(https://bad.test/image)">ok</p>"#,
        );
        assert!(!mail.html.contains("bad.test"));
        assert!(!mail.html.contains("<style"));
        assert!(mail.html.contains("color:blue"));
    }
    #[test]
    fn consent_is_scoped_and_revocable() {
        let mut mail = prepare("<img src=\"https://example.test/image\">");
        let fetcher = RemoteFetcher::new(Limits::default());
        assert!(matches!(
            mail.approve_remote_images(&RemoteConsent::for_document(18), &fetcher),
            Err(Error::ConsentMismatch)
        ));
        let consent = RemoteConsent::for_document(17);
        let other = consent.clone();
        other.revoke();
        assert!(matches!(
            mail.approve_remote_images(&consent, &fetcher),
            Err(Error::Cancelled)
        ));
        let replacement = prepare("<p>replacement</p>");
        assert!(replacement.resources.is_empty());
        assert!(replacement.remote_images.is_empty());
    }
    #[test]
    fn plain_text_is_escaped_and_keeps_unicode_boundaries() {
        let html = plain_text_html("<script>שלום👋&", 24);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.is_char_boundary(html.len()));
        let mail = prepare("<p>ok</p>");
        assert_eq!(mail.fallback_text, "fallback <text>");
        assert!(mail.fallback_html.contains("fallback &lt;text&gt;"));
    }
    #[test]
    fn duplicate_remote_urls_share_one_placeholder() {
        let mail = prepare("<img src='https://example.test/a'><img src='https://example.test/a'>");
        assert_eq!(mail.remote_images.len(), 1);
        assert_eq!(mail.html.matches("gpui-resource://17/remote-0").count(), 2);
    }
    #[test]
    fn malformed_or_mismatched_images_are_rejected() {
        assert!(
            InlineResource::new(b"not an image".to_vec(), "image/png", &Limits::default()).is_err()
        );
        assert!(InlineResource::new(png(), "image/jpeg", &Limits::default()).is_err());
        assert!(
            InlineResource::new(b"<svg/>".to_vec(), "image/svg+xml", &Limits::default()).is_err()
        );
    }
    #[test]
    fn decoded_size_and_document_quotas_are_enforced() {
        let mut limits = Limits {
            max_image_pixels: 1,
            ..Limits::default()
        };
        assert!(InlineResource::new(png(), "image/png", &limits).is_err());
        limits = Limits {
            max_html_bytes: 2,
            ..Limits::default()
        };
        assert!(prepare_email(1, "long", "", &BTreeMap::new(), &limits).is_err());
        limits = Limits {
            max_images: 1,
            ..Limits::default()
        };
        assert!(prepare_email(1, "<img><img>", "", &BTreeMap::new(), &limits).is_err());
        limits = Limits {
            max_html_nodes: 1,
            ..Limits::default()
        };
        assert!(prepare_email(1, "<p>one</p><p>two</p>", "", &BTreeMap::new(), &limits).is_err());
    }
    #[test]
    fn forged_public_resource_fields_are_validated_before_use() {
        let inline = BTreeMap::from([(
            "hero".into(),
            InlineResource {
                bytes: b"<svg/>".to_vec(),
                mime: "image/svg+xml".into(),
            },
        )]);
        assert!(prepare_email(1, "<img src='cid:hero'>", "", &inline, &Limits::default()).is_err());
    }
    #[test]
    fn navigation_is_only_an_intent_with_allowed_scheme() {
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "gpui-resource://4/a",
            "//elsewhere.test/a",
            "https://user:pass@example.test",
        ] {
            assert!(allowed_link(bad).is_none(), "{bad}");
        }
        for good in [
            "#section",
            "https://example.test/a",
            "http://example.test",
            "mailto:person@example.test",
        ] {
            assert!(allowed_link(good).is_some(), "{good}");
        }
    }
    #[test]
    fn data_raster_is_normalized_to_a_resource() {
        let data = base64::engine::general_purpose::STANDARD.encode(png());
        let mail = prepare(&format!("<img src='data:image/png;base64,{data}'>"));
        assert_eq!(mail.resources.len(), 1);
        assert!(!mail.html.contains("data:image"));
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    fn image() -> InlineResource {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([10, 20, 30, 255]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        InlineResource::new(bytes.into_inner(), "image/png", &Limits::default()).unwrap()
    }
    #[test]
    fn approved_image_bytes_are_provided_without_restoring_network_urls() {
        let mut message = prepare_email(
            4,
            "<img src='https://example.test/image'>",
            "",
            &BTreeMap::new(),
            &Limits::default(),
        )
        .unwrap();
        let report = message
            .approve_remote_images_with(&RemoteConsent::for_document(4), |_, _| Ok(image()))
            .unwrap();
        assert_eq!(report.loaded, 1);
        assert_eq!(message.resources.len(), 1);
        assert!(message.html.contains("gpui-resource://4/remote-0"));
        assert!(!message.html.contains("https://example.test/image"));
    }
    #[test]
    fn revoked_consent_during_fetch_cannot_install_a_result() {
        let mut message = prepare_email(
            4,
            "<img src='https://example.test/image'>",
            "",
            &BTreeMap::new(),
            &Limits::default(),
        )
        .unwrap();
        let consent = RemoteConsent::for_document(4);
        let revoke = consent.clone();
        assert!(matches!(
            message.approve_remote_images_with(&consent, |_, _| {
                revoke.revoke();
                Ok(image())
            }),
            Err(Error::Cancelled)
        ));
        assert!(message.resources.is_empty());
    }
    #[test]
    fn partial_failure_stays_a_placeholder() {
        let mut message = prepare_email(
            4,
            "<img src='https://example.test/image'>",
            "",
            &BTreeMap::new(),
            &Limits::default(),
        )
        .unwrap();
        let report = message
            .approve_remote_images_with(&RemoteConsent::for_document(4), |_, _| {
                Err(Error::Network("test failure".into()))
            })
            .unwrap();
        assert_eq!(report.loaded, 0);
        assert_eq!(report.failed.len(), 1);
        assert!(message.resources.is_empty());
    }
    #[test]
    fn supplied_mime_plain_text_is_optional() {
        let message = prepare_email(
            4,
            "<h1>Title</h1><p>שלום 中文</p><script>hidden</script>",
            "",
            &BTreeMap::new(),
            &Limits::default(),
        )
        .unwrap();
        assert!(message.fallback_text.contains("Title\nשלום 中文"));
        assert!(!message.fallback_text.contains("hidden"));
    }
    #[test]
    fn huge_layout_dimensions_are_removed() {
        let message=prepare_email(4,"<table width='999999999'><tr><td colspan='9999999' style='width:999999999px;color:red'>safe</td></tr></table>","",&BTreeMap::new(),&Limits::default()).unwrap();
        assert!(!message.html.contains("999999"));
        assert!(message.html.contains("color:red"));
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    fn prepare(id: u64, html: &str) -> Result<PreparedEmail> {
        prepare_email(id, html, "", &BTreeMap::new(), &Limits::default())
    }
    #[test]
    fn deep_dom_is_rejected_before_recursive_serialization() {
        // Below the byte/node quotas, but far beyond safe recursive serialization.
        let html = format!("{}text{}", "<div>".repeat(12_000), "</div>".repeat(12_000));
        assert!(matches!(prepare(1, &html), Err(Error::Limit("HTML depth"))));
        assert!(
            prepare(
                1,
                &format!("{}ok{}", "<div>".repeat(60), "</div>".repeat(60))
            )
            .is_ok()
        );
        for depth in [0, 257, usize::MAX] {
            let limits = Limits {
                max_html_depth: depth,
                ..Limits::default()
            };
            assert!(prepare_email(1, "<p>ok</p>", "", &BTreeMap::new(), &limits).is_err());
        }
    }
    #[test]
    fn template_fragments_never_reach_dom_construction() {
        for html in [
            "<TeMpLaTe><div>x</div></template>".into(),
            "<template>".repeat(12_000),
        ] {
            assert!(matches!(
                prepare(1, &html),
                Err(Error::Limit("unsupported template markup"))
            ));
        }
        assert!(prepare(1, "<!-- <template> --><p title='<template>'>safe</p>").is_ok());
    }
    #[test]
    fn hostile_foreign_markup_and_encoded_urls_leave_no_active_surface() {
        let prepared = prepare(1, include_str!("../../../fixtures/email-security.html")).unwrap();
        let document = kuchiki::parse_html()
            .one(prepared.html.as_str())
            .document_node;
        for tag in [
            "svg", "math", "script", "style", "iframe", "form", "input", "base",
        ] {
            assert_eq!(document.select(tag).unwrap().count(), 0, "{tag}");
        }
        for element in document.select("*").unwrap() {
            let attrs = element.attributes.borrow();
            assert!(
                !attrs
                    .map
                    .keys()
                    .any(|name| name.local.as_ref().starts_with("on"))
            );
            if let Some(src) = attrs.get("src") {
                assert!(src.starts_with("gpui-resource://1/"), "{src}");
            }
            if let Some(href) = attrs.get("href") {
                assert!(allowed_link(href).is_some(), "{href}");
            }
        }
        assert_eq!(prepared.html.matches("id=\"section\"").count(), 1);
        assert_eq!(prepared.html.matches("Content-Security-Policy").count(), 1);
        assert!(prepared.html.contains("שלום 中文 👋"));
        assert!(!prepared.html.contains("tracker.invalid"));
        assert!(prepared.resources.is_empty());
    }
    #[test]
    fn ambiguous_cid_map_is_rejected_independent_of_map_order() {
        let resource = InlineResource {
            bytes: Vec::new(),
            mime: "image/png".into(),
        };
        let inline = BTreeMap::from([
            ("hero@example.test".into(), resource.clone()),
            ("<hero%40example.test>".into(), resource),
        ]);
        assert!(matches!(
            prepare_email(1, "", "", &inline, &Limits::default()),
            Err(Error::InvalidCid)
        ));
        for id in ["../x", "%2e%2e%2fx", "bad%00id", "bad%0aid", "%ff", "x\\y"] {
            assert!(content_id(id).is_err(), "{id}");
        }
    }
    #[test]
    fn normalized_output_writer_and_decode_memory_are_bounded() {
        use std::io::Write;
        let mut output = BoundedImageOutput {
            bytes: Vec::new(),
            limit: 3,
        };
        output.write_all(b"abc").unwrap();
        assert!(output.write_all(b"d").is_err());
        assert_eq!(output.bytes, b"abc");
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::new(64, 64));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let limits = Limits {
            max_decode_bytes: 1,
            ..Limits::default()
        };
        assert!(InlineResource::new(png.into_inner(), "image/png", &limits).is_err());
    }
    #[test]
    fn reprepare_never_inherits_resources_or_old_consent() {
        let mut first = prepare(1, "<img src='https://example.test/first'>").unwrap();
        let consent = RemoteConsent::for_document(1);
        let mut second = prepare(2, "<img src='https://example.test/second'>").unwrap();
        let mut called = false;
        assert!(matches!(
            second.approve_remote_images_with(&consent, |_, _| {
                called = true;
                unreachable!()
            }),
            Err(Error::ConsentMismatch)
        ));
        assert!(!called);
        assert!(second.resources.is_empty());
        consent.revoke();
        assert!(matches!(
            first.approve_remote_images_with(&consent, |_, _| unreachable!()),
            Err(Error::Cancelled)
        ));
        let mut no_images = prepare(1, "<p>no images</p>").unwrap();
        assert!(matches!(
            no_images.approve_remote_images_with(&consent, |_, _| unreachable!()),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn images_discarded_by_sanitizer_do_not_enter_consent_work() {
        let prepared = prepare(1, "<svg><foreignObject><img src='https://example.test/hidden'></foreignObject></svg><script><img src='https://example.test/script'></script>").unwrap();
        assert!(!prepared.html.contains("gpui-resource://1/"));
        assert!(prepared.remote_images.is_empty());
    }
}
