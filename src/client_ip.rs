use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

pub const CANONICAL_CLIENT_IP_HEADER: HeaderName = HeaderName::from_static("x-monoize-client-ip");
const DEFAULT_TRUSTED_PROXY_CIDRS: &str = "127.0.0.0/8,::1/128";

#[derive(Clone, Debug, Default)]
pub struct TrustedProxyConfig {
    networks: Arc<Vec<IpNet>>,
}

impl TrustedProxyConfig {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("MONOIZE_TRUSTED_PROXY_CIDRS") {
            Ok(raw) => Self::from_configured_value(Some(&raw)),
            Err(std::env::VarError::NotPresent) => Self::from_configured_value(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err("MONOIZE_TRUSTED_PROXY_CIDRS is not valid Unicode".to_string())
            }
        }
    }

    fn from_configured_value(raw: Option<&str>) -> Result<Self, String> {
        Self::parse(raw.unwrap_or(DEFAULT_TRUSTED_PROXY_CIDRS))
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let networks = raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                entry
                    .parse::<IpNet>()
                    .map_err(|error| format!("invalid trusted proxy CIDR {entry:?}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            networks: Arc::new(networks),
        })
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        self.networks
            .iter()
            .any(|network| network.contains(&address))
    }
}

pub async fn canonical_client_ip_middleware(
    State(trusted_proxies): State<TrustedProxyConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    request.headers_mut().remove(&CANONICAL_CLIENT_IP_HEADER);

    if let Some(peer) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip())
    {
        let canonical = canonical_client_ip(peer, request.headers(), &trusted_proxies);
        if let Ok(value) = HeaderValue::from_str(&canonical.to_string()) {
            request
                .headers_mut()
                .insert(CANONICAL_CLIENT_IP_HEADER.clone(), value);
        }
    }

    next.run(request).await
}

pub fn canonical_client_ip_from_headers(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(&CANONICAL_CLIENT_IP_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn canonical_client_ip(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &TrustedProxyConfig,
) -> IpAddr {
    if !trusted_proxies.contains(peer) {
        return peer;
    }

    let mut chain = forwarded_chain(headers).unwrap_or_default();
    chain.push(peer);
    while chain
        .last()
        .is_some_and(|address| trusted_proxies.contains(*address))
    {
        chain.pop();
    }
    chain.last().copied().unwrap_or(peer)
}

fn forwarded_chain(headers: &HeaderMap) -> Option<Vec<IpAddr>> {
    if let Some(value) = headers
        .get("forwarded")
        .and_then(|value| value.to_str().ok())
    {
        let mut addresses = Vec::new();
        for element in value.split(',') {
            let Some(raw) = element.split(';').find_map(|part| {
                let (name, value) = part.trim().split_once('=')?;
                name.eq_ignore_ascii_case("for").then_some(value.trim())
            }) else {
                return None;
            };
            addresses.push(parse_forwarded_address(raw)?);
        }
        return (!addresses.is_empty()).then_some(addresses);
    }

    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        let addresses = value
            .split(',')
            .map(str::trim)
            .map(str::parse::<IpAddr>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        return (!addresses.is_empty()).then_some(addresses);
    }

    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .map(|address| vec![address])
}

fn parse_forwarded_address(raw: &str) -> Option<IpAddr> {
    let raw = raw.trim_matches('"');
    if raw.eq_ignore_ascii_case("unknown") || raw.starts_with('_') {
        return None;
    }
    if let Ok(address) = raw.parse::<IpAddr>() {
        return Some(address);
    }
    if let Ok(socket) = raw.parse::<SocketAddr>() {
        return Some(socket.ip());
    }
    raw.strip_prefix('[')
        .and_then(|value| value.split_once(']'))
        .and_then(|(address, _)| address.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn untrusted_peer_ignores_forwarding_headers() {
        let trusted = TrustedProxyConfig::parse("10.0.0.0/8").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(
            canonical_client_ip("192.0.2.10".parse().unwrap(), &headers, &trusted),
            "192.0.2.10".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn trusted_chain_selects_nearest_untrusted_address() {
        let trusted = TrustedProxyConfig::parse("10.0.0.0/8,192.0.2.0/24").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.8, 10.2.0.3"),
        );
        assert_eq!(
            canonical_client_ip("192.0.2.10".parse().unwrap(), &headers, &trusted),
            "198.51.100.8".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn malformed_trusted_proxy_configuration_fails() {
        assert!(TrustedProxyConfig::parse("not-a-cidr").is_err());
    }

    #[test]
    fn default_trusts_loopback_reverse_proxies() {
        let trusted = TrustedProxyConfig::from_configured_value(None).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));

        assert_eq!(
            canonical_client_ip("127.0.0.1".parse().unwrap(), &headers, &trusted),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            canonical_client_ip("::1".parse().unwrap(), &headers, &trusted),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn explicitly_empty_configuration_trusts_no_proxy() {
        let trusted = TrustedProxyConfig::from_configured_value(Some("")).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));

        assert_eq!(
            canonical_client_ip("127.0.0.1".parse().unwrap(), &headers, &trusted),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }
}
