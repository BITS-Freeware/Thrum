//! Client address for rate limits.
//!
//! The deployment guide puts a reverse proxy in front of the server, so the
//! socket address is the proxy, not the person. Forwarded headers are
//! honored only when that socket address is on the operator's trusted-proxy
//! list. Otherwise they are ignored, because anyone can set them.

use std::net::IpAddr;

use axum::http::HeaderMap;

/// One trusted proxy, as an address or a CIDR block.
#[derive(Debug, Clone)]
pub struct TrustedNet {
    kind: NetKind,
}

#[derive(Debug, Clone)]
enum NetKind {
    V4 { network: u32, mask: u32 },
    V6 { network: u128, mask: u128 },
}

impl TrustedNet {
    /// Parse `127.0.0.1` or `10.0.0.0/8`. Returns none for a bad entry.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let (addr, bits) = match text.split_once('/') {
            Some((addr, bits)) => {
                let bits: u32 = bits.parse().ok()?;
                (addr, Some(bits))
            }
            None => (text, None),
        };
        if let Ok(IpAddr::V4(ip)) = addr.parse() {
            let bits = bits.unwrap_or(32);
            if bits > 32 {
                return None;
            }
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            let network = u32::from(ip) & mask;
            return Some(Self {
                kind: NetKind::V4 { network, mask },
            });
        }
        if let Ok(IpAddr::V6(ip)) = addr.parse() {
            let bits = bits.unwrap_or(128);
            if bits > 128 {
                return None;
            }
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            let network = u128::from(ip) & mask;
            return Some(Self {
                kind: NetKind::V6 { network, mask },
            });
        }
        None
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (&self.kind, ip) {
            (NetKind::V4 { network, mask }, IpAddr::V4(ip)) => (u32::from(ip) & mask) == *network,
            (NetKind::V6 { network, mask }, IpAddr::V6(ip)) => (u128::from(ip) & mask) == *network,
            _ => false,
        }
    }
}

/// The address a rate limit should key on.
///
/// When `peer` is not in `trusted`, the result is `peer` and forwarded
/// headers are ignored. When it is, the client is the rightmost address in
/// `X-Forwarded-For` (or `Forwarded`) that is not itself a trusted proxy.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[TrustedNet]) -> IpAddr {
    if !trusted.iter().any(|net| net.contains(peer)) {
        return peer;
    }
    let forwarded = forwarded_addrs(headers);
    if forwarded.is_empty() {
        return peer;
    }
    for ip in forwarded.iter().rev() {
        if !trusted.iter().any(|net| net.contains(*ip)) {
            return *ip;
        }
    }
    forwarded[0]
}

fn forwarded_addrs(headers: &HeaderMap) -> Vec<IpAddr> {
    let mut from_xff = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(text) = value.to_str() else {
            continue;
        };
        for part in text.split(',') {
            if let Some(ip) = parse_ip_token(part) {
                from_xff.push(ip);
            }
        }
    }
    if !from_xff.is_empty() {
        return from_xff;
    }
    let mut from_forwarded = Vec::new();
    for value in headers.get_all("forwarded") {
        let Ok(text) = value.to_str() else {
            continue;
        };
        for part in text.split(',') {
            let Some(for_value) = part.split(';').find_map(|param| {
                let param = param.trim();
                param
                    .get(..4)
                    .filter(|prefix| prefix.eq_ignore_ascii_case("for="))
                    .map(|_| param[4..].trim())
            }) else {
                continue;
            };
            if let Some(ip) = parse_ip_token(for_value) {
                from_forwarded.push(ip);
            }
        }
    }
    from_forwarded
}

fn parse_ip_token(token: &str) -> Option<IpAddr> {
    let token = token.trim().trim_matches('"');
    if token.is_empty() || token == "unknown" {
        return None;
    }
    if let Some(rest) = token.strip_prefix('[') {
        let host = rest.split(']').next()?;
        return host.parse().ok();
    }
    if token.matches(':').count() == 1 {
        let (host, _) = token.split_once(':')?;
        return host.parse().ok();
    }
    token.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn nets(entries: &[&str]) -> Vec<TrustedNet> {
        entries
            .iter()
            .map(|entry| TrustedNet::parse(entry).expect(entry))
            .collect()
    }

    fn headers(xff: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(xff).unwrap());
        headers
    }

    #[test]
    fn untrusted_peers_ignore_forwarded_headers() {
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let ip = client_ip(peer, &headers("203.0.113.5"), &[]);
        assert_eq!(ip, peer);
    }

    #[test]
    fn a_trusted_proxy_yields_the_rightmost_untrusted_address() {
        let peer: IpAddr = "10.0.0.2".parse().unwrap();
        let trusted = nets(&["10.0.0.0/8"]);
        // The client claimed to be 1.1.1.1; the proxy appended the real one.
        let ip = client_ip(peer, &headers("1.1.1.1, 203.0.113.9"), &trusted);
        assert_eq!(ip, "203.0.113.9".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn forwarded_header_is_used_when_xff_is_absent() {
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "forwarded",
            HeaderValue::from_static("for=192.0.2.43;proto=https"),
        );
        let ip = client_ip(peer, &headers, &nets(&["127.0.0.1"]));
        assert_eq!(ip, "192.0.2.43".parse::<IpAddr>().unwrap());
    }
}
