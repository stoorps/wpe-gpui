//! Bounded blocking HTTP image fetches for an application worker, never WebKit.
use crate::{Error, InlineResource, Limits, RemoteConsent, RemoteImage, Result};
use std::{
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{OnceLock, mpsc},
    time::Instant,
};
use url::Url;

#[derive(Debug, Default)]
pub struct RemoteLoadReport {
    pub loaded: usize,
    pub failed: Vec<(String, String)>,
}
pub struct RemoteFetcher {
    limits: Limits,
}
impl RemoteFetcher {
    pub fn new(limits: Limits) -> Self {
        Self { limits }
    }
    pub fn fetch(&self, initial: &Url) -> Result<InlineResource> {
        self.fetch_inner(initial, None)
    }
    pub fn fetch_with_consent(
        &self,
        initial: &Url,
        consent: &RemoteConsent,
    ) -> Result<InlineResource> {
        self.fetch_inner(initial, Some(consent))
    }
    fn fetch_inner(
        &self,
        initial: &Url,
        consent: Option<&RemoteConsent>,
    ) -> Result<InlineResource> {
        let cancelled = || consent.is_some_and(RemoteConsent::is_revoked);
        let deadline = Instant::now() + self.limits.request_timeout;
        let mut url = initial.clone();
        for redirect in 0..=self.limits.max_redirects {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            validate_url(&url)?;
            let addresses = resolve(
                &url,
                self.limits
                    .dns_timeout
                    .min(deadline.saturating_duration_since(Instant::now())),
            )?;
            if addresses.is_empty() || addresses.iter().any(|a| !public_address(a.ip())) {
                return Err(Error::UrlPolicy("non-public network address"));
            }
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err(Error::Network("deadline exceeded".into()));
            }
            let host = url.host_str().ok_or(Error::UrlPolicy("missing host"))?;
            // Pin the validated addresses. No DNS lookup/rebinding after validation,
            // no environment proxy, cookies, referrer, user credentials or TLS bypass.
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .no_gzip()
                .no_brotli()
                .no_zstd()
                .no_deflate()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(timeout)
                .connect_timeout(timeout.min(std::time::Duration::from_secs(5)))
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| Error::Network("could not construct TLS client".into()))?;
            let mut response = client
                .get(url.clone())
                .header(
                    reqwest::header::ACCEPT,
                    "image/png,image/jpeg,image/gif,image/webp",
                )
                .send()
                .map_err(|_| Error::Network("connection/TLS failure or timeout".into()))?;
            if cancelled() {
                return Err(Error::Cancelled);
            }
            if response.status().is_redirection() {
                if redirect == self.limits.max_redirects {
                    return Err(Error::UrlPolicy("redirect limit"));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(Error::UrlPolicy("invalid redirect"))?;
                url = url
                    .join(location)
                    .map_err(|_| Error::UrlPolicy("invalid redirect URL"))?;
                continue;
            }
            if !response.status().is_success() {
                return Err(Error::Network(format!(
                    "HTTP {}",
                    response.status().as_u16()
                )));
            }
            if response
                .content_length()
                .is_some_and(|size| size > self.limits.max_resource_bytes as u64)
            {
                return Err(Error::Limit("remote image bytes"));
            }
            if response
                .headers()
                .contains_key(reqwest::header::CONTENT_ENCODING)
            {
                return Err(Error::UrlPolicy("encoded HTTP response is unsupported"));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .ok_or(Error::UrlPolicy("missing image Content-Type"))?
                .to_owned();
            let mut bytes = Vec::new();
            let mut chunk = [0; 16 * 1024];
            loop {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                let count = response
                    .read(&mut chunk)
                    .map_err(|_| Error::Network("body read failure".into()))?;
                if count == 0 {
                    break;
                }
                if bytes.len().saturating_add(count) > self.limits.max_resource_bytes {
                    return Err(Error::Limit("remote image bytes"));
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            if cancelled() {
                return Err(Error::Cancelled);
            }
            return InlineResource::new(bytes, &mime, &self.limits);
        }
        Err(Error::UrlPolicy("redirect limit"))
    }
    pub fn validate_remote(&self, image: &RemoteImage) -> Result<()> {
        validate_url(&image.url)
    }
}
fn validate_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(Error::UrlPolicy("only HTTPS images are fetched"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::UrlPolicy("URL credentials are forbidden"));
    }
    if url.port().is_some_and(|p| p != 443) {
        return Err(Error::UrlPolicy("only the HTTPS default port is accepted"));
    }
    if url.host_str().is_none() {
        return Err(Error::UrlPolicy("missing host"));
    }
    if let Some(host) = url.host_str() {
        let host = host.trim_matches(['[', ']']);
        if host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".localhost")
            || host.ends_with('.')
        {
            return Err(Error::UrlPolicy("local/ambiguous host"));
        }
        if let Ok(ip) = host.parse::<IpAddr>()
            && !public_address(ip)
        {
            return Err(Error::UrlPolicy("non-public network address"));
        }
    }
    Ok(())
}
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let n = u32::from(ip);
            let blocked = [
                (0x00000000, 8),
                (0x0a000000, 8),
                (0x64400000, 10),
                (0x7f000000, 8),
                (0xa9fe0000, 16),
                (0xac100000, 12),
                (0xc0000000, 24),
                (0xc0000200, 24),
                (0xc0a80000, 16),
                (0xc0586300, 24),
                (0xc6120000, 15),
                (0xc6336400, 24),
                (0xcb007100, 24),
                (0xe0000000, 4),
                (0xf0000000, 4),
            ];
            !blocked
                .iter()
                .any(|(network, bits)| n & (!0u32 << (32 - bits)) == *network)
        }
        IpAddr::V6(ip) => {
            let n = u128::from(ip);
            // Accept global-unicast space only, excluding special-purpose/documentation
            // ranges and 6to4 transition addresses that can encode local IPv4 targets.
            n >> 125 == 1
                && n >> 96 != 0x20010db8
                && n >> 112 != 0x2002
                && n >> 108 != 0x3fff0
                && n >> 105 != (0x2001u128 << 112) >> 105
        }
    }
}
struct DnsRequest {
    host: String,
    port: u16,
    deadline: Instant,
    reply: mpsc::Sender<std::io::Result<Vec<SocketAddr>>>,
}
fn dns_worker() -> &'static mpsc::SyncSender<DnsRequest> {
    static WORKER: OnceLock<mpsc::SyncSender<DnsRequest>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<DnsRequest>(8);
        std::thread::Builder::new()
            .name("email-image-dns".into())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    if Instant::now() >= request.deadline {
                        continue;
                    }
                    let result = (request.host.as_str(), request.port)
                        .to_socket_addrs()
                        .map(Iterator::collect);
                    let _ = request.reply.send(result);
                }
            })
            .expect("start bounded DNS worker");
        tx
    })
}
fn resolve(url: &Url, timeout: std::time::Duration) -> Result<Vec<SocketAddr>> {
    if timeout.is_zero() {
        return Err(Error::Network("DNS deadline exceeded".into()));
    }
    let host = url
        .host_str()
        .ok_or(Error::UrlPolicy("missing host"))?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .ok_or(Error::UrlPolicy("missing port"))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let (tx, rx) = mpsc::channel();
    dns_worker()
        .try_send(DnsRequest {
            host: host.into(),
            port,
            deadline: Instant::now() + timeout,
            reply: tx,
        })
        .map_err(|_| Error::Network("DNS worker busy".into()))?;
    rx.recv_timeout(timeout)
        .map_err(|_| Error::Network("DNS timeout".into()))?
        .map_err(|_| Error::Network("DNS resolution failed".into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_local_and_special_networks() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "3fff::1",
            "192.88.99.1",
            "2002:7f00:1::",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn permits_public_addresses() {
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn denies_before_any_network_io() {
        let fetch = RemoteFetcher::new(Limits::default());
        for u in [
            "http://example.com/a.png",
            "https://localhost/a.png",
            "https://127.0.0.1/a.png",
            "https://[::1]/a.png",
            "https://user:pass@example.com/a.png",
            "https://example.com:8443/a.png",
            "file:///tmp/a.png",
        ] {
            assert!(fetch.fetch(&Url::parse(u).unwrap()).is_err(), "{u}");
        }
    }
    #[test]
    fn alternate_ip_spellings_and_redirect_targets_reuse_the_same_policy() {
        // URL parsing canonicalizes unusual IPv4 forms before address checks.
        for target in [
            "https://2130706433/a",
            "https://0x7f000001/a",
            "https://0177.0.0.1/a",
            "https://127.1/a",
            "https://%6cocalhost/a",
            "https://example.test./a",
            "https://[::ffff:7f00:1]/a",
            "https://u%73er@example.test/a",
            "http://example.test/a",
            "//localhost/a",
            "https://example.test:444/a",
        ] {
            let redirected = Url::parse("https://example.test/original")
                .unwrap()
                .join(target)
                .unwrap();
            assert!(validate_url(&redirected).is_err(), "{target}");
        }
    }
    #[test]
    fn revoked_token_cancels_before_dns_or_http() {
        let consent = RemoteConsent::for_document(1);
        consent.revoke();
        assert!(matches!(
            RemoteFetcher::new(Limits::default())
                .fetch_with_consent(&Url::parse("https://example.test/image").unwrap(), &consent),
            Err(Error::Cancelled)
        ));
    }
}
