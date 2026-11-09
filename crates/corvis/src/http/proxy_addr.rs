//! Express' `trust proxy` setting with the semantics of the `proxy-addr` package: which address
//! is the client (`req.ip`), which proxies forwarded the request (`req.ips`) and whether
//! `X-Forwarded-Host` is honored (`req.hostname`).
//!
//! The addresses of a request are the socket address followed by the `X-Forwarded-For` entries
//! from right to left. Walking that list, every address the setting trusts lets the next one in;
//! the first untrusted address is the client. With the default ([`TrustProxy::None`]) the socket
//! address is the client and `X-Forwarded-For` is ignored, so it cannot be forged.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use http::HeaderMap;

/// `app.set('trust proxy', ...)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrustProxy {
    /// `false` (default): no proxy is trusted, `req.ip` is the socket address.
    #[default]
    None,
    /// `true`: every hop is trusted, `req.ip` is the left-most `X-Forwarded-For` entry. Only safe
    /// when every request goes through a proxy that overwrites `X-Forwarded-For`.
    All,
    /// A number: trust the `n` hops closest to the server (the socket address is hop 0), e.g.
    /// `Hops(1)` behind a single reverse proxy.
    Hops(usize),
    /// A list of trusted proxy addresses and subnets (see [`TrustProxy::parse`]).
    Subnets(Arc<[Subnet]>),
}

/// An invalid `trust proxy` address or subnet (the `TypeError` of `proxy-addr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTrustProxy(pub String);

impl std::fmt::Display for InvalidTrustProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidTrustProxy {}

impl From<bool> for TrustProxy {
    fn from(trust: bool) -> Self {
        if trust { TrustProxy::All } else { TrustProxy::None }
    }
}

impl From<usize> for TrustProxy {
    fn from(hops: usize) -> Self {
        TrustProxy::Hops(hops)
    }
}

impl TrustProxy {
    /// `app.set('trust proxy', 'loopback, 10.0.0.0/8')`: a comma-separated list of addresses,
    /// subnets (`10.0.0.0/8`, `10.0.0.0/255.0.0.0`, `fc00::/7`) and the predefined names
    /// `loopback`, `linklocal` and `uniquelocal`.
    pub fn parse(list: &str) -> Result<Self, InvalidTrustProxy> {
        Self::subnets(list.split(',').map(str::trim))
    }

    /// `app.set('trust proxy', ['loopback', '10.0.0.0/8'])`.
    pub fn subnets<I, S>(items: I) -> Result<Self, InvalidTrustProxy>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Vec::new();
        for item in items {
            let item = item.as_ref();
            match named_range(item) {
                Some(range) => {
                    for note in range {
                        out.push(Subnet::parse(note)?);
                    }
                }
                None => out.push(Subnet::parse(item)?),
            }
        }
        Ok(TrustProxy::Subnets(out.into()))
    }

    /// The compiled trust function: is the address at hop `hop` (0 = socket) a trusted proxy?
    pub fn trusts(&self, addr: &str, hop: usize) -> bool {
        match self {
            TrustProxy::None => false,
            TrustProxy::All => true,
            TrustProxy::Hops(n) => hop < *n,
            TrustProxy::Subnets(list) => match addr.parse::<IpAddr>() {
                Ok(ip) => list.iter().any(|s| s.contains(ip)),
                Err(_) => false,
            },
        }
    }
}

/// The predefined ranges of `proxy-addr`.
fn named_range(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "linklocal" => &["169.254.0.0/16", "fe80::/10"],
        "loopback" => &["127.0.0.1/8", "::1/128"],
        "uniquelocal" => &["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "fc00::/7"],
        _ => return None,
    })
}

/// A trusted address or subnet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    ip: IpAddr,
    prefix: u8,
}

impl Subnet {
    /// `parseipNotation`: `ip`, `ip/prefix` or (IPv4) `ip/netmask`.
    pub fn parse(note: &str) -> Result<Subnet, InvalidTrustProxy> {
        let (addr, range) = match note.rfind('/') {
            Some(i) => (&note[..i], Some(&note[i + 1..])),
            None => (note, None),
        };
        let mut ip: IpAddr = addr.parse().map_err(|_| InvalidTrustProxy(format!("invalid IP address: {addr}")))?;
        if let (IpAddr::V6(v6), None) = (ip, range)
            && let Some(v4) = v6.to_ipv4_mapped()
        {
            ip = IpAddr::V4(v4);
        }
        let max: u32 = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = match range {
            None => Some(max),
            Some(r) if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) => Some(r.parse::<u32>().unwrap_or(u32::MAX)),
            Some(r) if ip.is_ipv4() => r.parse::<Ipv4Addr>().ok().and_then(|m| netmask_prefix(u32::from(m))),
            Some(_) => None,
        };
        match prefix {
            Some(p) if p > 0 && p <= max => Ok(Subnet { ip, prefix: p as u8 }),
            _ => Err(InvalidTrustProxy(format!("invalid range on address: {note}"))),
        }
    }

    /// `trustSingle`: an IPv4-mapped IPv6 address matches IPv4 subnets and vice versa.
    pub fn contains(&self, addr: IpAddr) -> bool {
        match (self.ip, addr) {
            (IpAddr::V4(net), IpAddr::V4(a)) => v4_match(net, a, self.prefix),
            (IpAddr::V4(net), IpAddr::V6(a)) => a.to_ipv4_mapped().is_some_and(|a| v4_match(net, a, self.prefix)),
            (IpAddr::V6(net), IpAddr::V6(a)) => v6_match(net, a, self.prefix),
            (IpAddr::V6(net), IpAddr::V4(a)) => v6_match(net, a.to_ipv6_mapped(), self.prefix),
        }
    }
}

fn v4_match(net: Ipv4Addr, a: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix as u32).unwrap_or(0);
    u32::from(net) & mask == u32::from(a) & mask
}

fn v6_match(net: Ipv6Addr, a: Ipv6Addr, prefix: u8) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix as u32).unwrap_or(0);
    u128::from(net) & mask == u128::from(a) & mask
}

/// The prefix length of a contiguous netmask (`255.255.0.0` → 16).
fn netmask_prefix(mask: u32) -> Option<u32> {
    let ones = mask.leading_ones();
    (mask.checked_shl(ones).unwrap_or(0) == 0).then_some(ones)
}

/// `forwarded(req)`: the socket address, then the `X-Forwarded-For` entries from right to left
/// (several headers are joined with `, ` like Node; values are read as latin1; empty entries and
/// the spaces around each entry are dropped).
fn forwarded(socket: String, headers: &HeaderMap) -> Vec<String> {
    let mut addrs = vec![socket];
    let header: Vec<String> = headers.get_all("x-forwarded-for").iter().map(|v| super::body_parser::latin1(v.as_bytes())).collect();
    for entry in header.join(", ").split(',').rev() {
        let entry = entry.trim_matches(' ');
        if !entry.is_empty() {
            addrs.push(entry.to_string());
        }
    }
    addrs
}

/// Drops the spaces around an `X-Forwarded-For` entry.
fn trim_spaces(entry: &[u8]) -> &[u8] {
    let start = entry.iter().position(|b| *b != b' ').unwrap_or(entry.len());
    let end = entry.iter().rposition(|b| *b != b' ').map_or(start, |i| i + 1);
    &entry[start..end]
}

/// The `X-Forwarded-For` entries from right to left as raw bytes (several headers count as joined with
/// `, ` like Node; empty entries and the spaces around each entry are dropped).
fn entries_rev(headers: &HeaderMap) -> impl Iterator<Item = &[u8]> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .rev()
        .flat_map(|v| v.as_bytes().rsplit(|b| *b == b','))
        .map(trim_spaces)
        .filter(|e| !e.is_empty())
}

/// The client address (`proxyaddr(req, trust)`): the same result as the last element of
/// [`all_addrs`], without building the chain. It walks the entries from the right and stops at the
/// first untrusted address; with `All` it takes the left-most entry. Only the entries it examines are
/// converted to strings, so a long `X-Forwarded-For` costs nothing past the answer.
pub(crate) fn client_addr(socket: String, headers: &HeaderMap, trust: &TrustProxy) -> String {
    match trust {
        TrustProxy::None => socket,
        TrustProxy::All => headers
            .get_all("x-forwarded-for")
            .iter()
            .flat_map(|v| v.as_bytes().split(|b| *b == b','))
            .map(trim_spaces)
            .find(|e| !e.is_empty())
            .map(super::body_parser::latin1)
            .unwrap_or(socket),
        _ => {
            if !trust.trusts(&socket, 0) {
                return socket;
            }
            let mut last = socket;
            for (i, entry) in entries_rev(headers).enumerate() {
                last = super::body_parser::latin1(entry);
                if !trust.trusts(&last, i + 1) {
                    break;
                }
            }
            last
        }
    }
}

/// `proxyaddr.all(req, trust)`: the socket address followed by the trusted chain, ending with
/// the client address.
pub(crate) fn all_addrs(socket: String, headers: &HeaderMap, trust: &TrustProxy) -> Vec<String> {
    if *trust == TrustProxy::None {
        return vec![socket];
    }
    let mut addrs = forwarded(socket, headers);
    let mut i = 0;
    while i + 1 < addrs.len() {
        if !trust.trusts(&addrs[i], i) {
            addrs.truncate(i + 1);
            break;
        }
        i += 1;
    }
    addrs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(xff: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in xff {
            h.append("x-forwarded-for", v.parse().unwrap());
        }
        h
    }

    fn ip(socket: &str, xff: &[&str], trust: &TrustProxy) -> String {
        all_addrs(socket.into(), &headers(xff), trust).pop().unwrap()
    }

    #[test]
    fn default_ignores_forwarded_for() {
        assert_eq!(ip("127.0.0.1", &["1.2.3.4"], &TrustProxy::None), "127.0.0.1");
        assert_eq!(all_addrs("127.0.0.1".into(), &headers(&["1.2.3.4"]), &TrustProxy::None), vec!["127.0.0.1"]);
    }

    #[test]
    fn trust_all_and_hops() {
        let xff = ["10.0.0.1, 10.0.0.2", "10.0.0.3"];
        assert_eq!(ip("127.0.0.1", &xff, &true.into()), "10.0.0.1");
        assert_eq!(ip("127.0.0.1", &xff, &TrustProxy::Hops(0)), "127.0.0.1");
        assert_eq!(ip("127.0.0.1", &xff, &1.into()), "10.0.0.3");
        assert_eq!(ip("127.0.0.1", &xff, &2.into()), "10.0.0.2");
        assert_eq!(ip("127.0.0.1", &xff, &10.into()), "10.0.0.1");
        assert_eq!(ip("127.0.0.1", &[" , 10.0.0.9 ,,  "], &true.into()), "10.0.0.9");
        assert_eq!(ip("127.0.0.1", &[], &true.into()), "127.0.0.1");
    }

    #[test]
    fn trust_subnets() {
        let trust = TrustProxy::parse("loopback, 10.0.0.0/8").unwrap();
        // The right-most untrusted entry is the client, whatever the client prepends.
        assert_eq!(ip("127.0.0.1", &["6.6.6.6, 1.2.3.4, 10.1.1.1"], &trust), "1.2.3.4");
        assert_eq!(ip("::ffff:127.0.0.1", &["1.2.3.4"], &trust), "1.2.3.4");
        assert_eq!(ip("::1", &["1.2.3.4"], &trust), "1.2.3.4");
        // An untrusted socket stops the walk.
        assert_eq!(ip("8.8.8.8", &["1.2.3.4"], &trust), "8.8.8.8");
        // Invalid entries are never trusted, but can be the client address (like proxy-addr).
        assert_eq!(ip("127.0.0.1", &["1.2.3.4, garbage"], &trust), "garbage");
        let mask = TrustProxy::parse("192.168.0.0/255.255.0.0, fc00::/7, ::ffff:10.0.0.1").unwrap();
        assert!(mask.trusts("192.168.7.7", 0));
        assert!(!mask.trusts("192.169.0.1", 0));
        assert!(mask.trusts("fd12::1", 0));
        assert!(mask.trusts("10.0.0.1", 0));
        assert!(mask.trusts("::ffff:192.168.1.1", 0));
        let v6 = TrustProxy::parse("::ffff:0:0/96").unwrap();
        assert!(v6.trusts("1.2.3.4", 0));
    }

    #[test]
    fn invalid_notations() {
        assert_eq!(TrustProxy::parse("nope").unwrap_err().to_string(), "invalid IP address: nope");
        assert_eq!(TrustProxy::parse("10.0.0.0/33").unwrap_err().to_string(), "invalid range on address: 10.0.0.0/33");
        assert!(TrustProxy::parse("10.0.0.0/0").is_err());
        assert!(TrustProxy::parse("10.0.0.0/255.0.255.0").is_err());
        assert!(TrustProxy::parse("::1/1.2.3.4").is_err());
        assert!(TrustProxy::subnets(["linklocal", "uniquelocal", "1.2.3.4"]).is_ok());
    }

    /// `client_addr` must give what the last element of `all_addrs` gives, for every setting.
    fn assert_same(socket: &str, xff: &[&str]) {
        let h = headers(xff);
        let subnets = TrustProxy::parse("loopback, 10.0.0.0/8, fc00::/7").unwrap();
        let settings = [
            TrustProxy::None,
            TrustProxy::All,
            TrustProxy::Hops(0),
            TrustProxy::Hops(1),
            TrustProxy::Hops(2),
            TrustProxy::Hops(50),
            subnets,
        ];
        for trust in settings {
            let expected = all_addrs(socket.into(), &h, &trust).pop().unwrap();
            assert_eq!(client_addr(socket.into(), &h, &trust), expected, "{socket} {xff:?} {trust:?}");
        }
    }

    #[test]
    fn the_lazy_client_address_matches_the_full_chain() {
        let cases: &[&[&str]] = &[
            &[],
            &[""],
            &[" "],
            &[",,,"],
            &[" , 10.0.0.9 ,,  "],
            &["1.2.3.4"],
            &["1.2.3.4, 10.0.0.1"],
            &["6.6.6.6, 1.2.3.4, 10.1.1.1"],
            &["10.0.0.1, 10.0.0.2", "10.0.0.3"],
            &["10.0.0.1,", ",10.0.0.2", " 10.0.0.3 "],
            &["2001:db8::1, fd00::1, ::1"],
            &["[2001:db8::1]:4000, 10.0.0.1"],
            &["1.2.3.4, garbage"],
            &["garbage, 10.0.0.1, 10.0.0.2"],
            &["10.0.0.1", "", "10.0.0.2"],
            &["a\tb , 10.0.0.1"],
        ];
        for socket in ["127.0.0.1", "10.9.9.9", "8.8.8.8", "::1", "::ffff:127.0.0.1", "fd00::5"] {
            for xff in cases {
                assert_same(socket, xff);
            }
        }
    }

    #[test]
    fn hops_beyond_the_header_length_give_the_left_most_entry() {
        assert_eq!(ip("127.0.0.1", &["1.1.1.1, 2.2.2.2"], &TrustProxy::Hops(10)), "1.1.1.1");
        assert_eq!(client_addr("127.0.0.1".into(), &headers(&["1.1.1.1, 2.2.2.2"]), &TrustProxy::Hops(10)), "1.1.1.1");
        assert_eq!(client_addr("127.0.0.1".into(), &headers(&[]), &TrustProxy::Hops(10)), "127.0.0.1");
    }

    #[test]
    fn long_headers_resolve_like_short_ones() {
        let long = vec!["10.0.0.1"; 4000].join(", ");
        let one_hop = TrustProxy::Hops(1);
        assert_same("127.0.0.1", &[&long]);
        assert_same("127.0.0.1", &[&format!("6.6.6.6, {long}")]);
        assert_eq!(client_addr("127.0.0.1".into(), &headers(&[&format!("{long}, 203.0.113.7")]), &one_hop), "203.0.113.7");
        assert_eq!(client_addr("127.0.0.1".into(), &headers(&[&format!("203.0.113.7, {long}")]), &TrustProxy::All), "203.0.113.7");
        let subnets = TrustProxy::parse("10.0.0.0/8, loopback").unwrap();
        assert_eq!(client_addr("127.0.0.1".into(), &headers(&[&format!("5.5.5.5, {long}")]), &subnets), "5.5.5.5");
    }
}
