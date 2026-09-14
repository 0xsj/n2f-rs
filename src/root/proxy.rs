use crate::shared::errors::{Failure, Kind};
use axum::http::HeaderMap;
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

#[derive(Clone, Debug)]
pub struct TrustedProxy {
    network: IpAddr,
    bits: u8,
}

pub fn parse_trusted_proxies(raw: &str) -> Result<Vec<TrustedProxy>, Failure> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_proxy)
        .collect()
}
fn parse_proxy(raw: &str) -> Result<TrustedProxy, Failure> {
    let (address, bits) = if let Some((address, bits)) = raw.split_once('/') {
        let ip = IpAddr::from_str(address).map_err(|_| invalid())?;
        let bits: u8 = bits.parse().map_err(|_| invalid())?;
        (ip, bits)
    } else {
        let ip = IpAddr::from_str(raw).map_err(|_| invalid())?;
        (ip, if ip.is_ipv4() { 32 } else { 128 })
    };
    let maximum = if address.is_ipv4() { 32 } else { 128 };
    if bits > maximum {
        return Err(invalid());
    }
    Ok(TrustedProxy {
        network: mask(address, bits),
        bits,
    })
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid trusted proxy configuration").with_type("env.invalid")
}
fn mask(address: IpAddr, bits: u8) -> IpAddr {
    match address {
        IpAddr::V4(value) => {
            let raw = u32::from(value);
            let masked = if bits == 0 {
                0
            } else {
                raw & u32::MAX << (32 - bits)
            };
            IpAddr::V4(masked.into())
        }
        IpAddr::V6(value) => {
            let raw = u128::from(value);
            let masked = if bits == 0 {
                0
            } else {
                raw & u128::MAX << (128 - bits)
            };
            IpAddr::V6(masked.into())
        }
    }
}
fn contains(proxy: &TrustedProxy, address: IpAddr) -> bool {
    if proxy.network.is_ipv4() != address.is_ipv4() {
        return false;
    }
    mask(address, proxy.bits) == proxy.network
}

/// Derive the auth source key. Forwarded addresses are trusted only after the
/// immediate peer matches the configured proxy network list.
pub fn trusted_source(
    proxies: &[TrustedProxy],
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
) -> String {
    let Some(peer) = peer else {
        return String::new();
    };
    let peer = peer.ip();
    let peer_value = peer.to_string();
    if proxies.is_empty() || !proxies.iter().any(|proxy| contains(proxy, peer)) {
        return peer_value;
    }
    let mut chain = Vec::new();
    for value in headers.get_all("x-forwarded-for").iter() {
        let Ok(value) = value.to_str() else {
            return peer_value;
        };
        for part in value.split(',') {
            let Ok(address) = IpAddr::from_str(part.trim()) else {
                return peer_value;
            };
            chain.push(address);
        }
    }
    if chain.is_empty() {
        return peer_value;
    }
    chain.push(peer);
    for address in chain.iter().rev() {
        if !proxies.iter().any(|proxy| contains(proxy, *address)) {
            return address.to_string();
        }
    }
    chain[0].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn requires_trusted_peer_and_walks_chain() {
        let proxies = parse_trusted_proxies("10.0.0.0/8").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.7, 10.0.0.2"),
        );
        let peer = "10.0.0.1:80".parse().unwrap();
        assert_eq!(
            trusted_source(&proxies, Some(peer), &headers),
            "198.51.100.7"
        );
        let untrusted = "203.0.113.9:80".parse().unwrap();
        assert_eq!(
            trusted_source(&proxies, Some(untrusted), &headers),
            "203.0.113.9"
        );
    }

    #[test]
    fn malformed_header_falls_back() {
        let proxies = parse_trusted_proxies("10.0.0.0/8").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        let peer = "10.0.0.1:80".parse().unwrap();
        assert_eq!(trusted_source(&proxies, Some(peer), &headers), "10.0.0.1");
    }
}
