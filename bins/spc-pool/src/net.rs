//! SOCKS5 egress for Tor (SPEC §3.8, decision 2026-11-02-tor-transport).
//!
//! Tor is a transport option, not a protocol mode: the proxied byte stream
//! carries the SV2 Noise, JDP and (when the in-tree Ark client lands) Ark
//! sessions unchanged. Only `socks5h://` proxies are accepted - remote name
//! resolution is mandatory, because resolving a name locally before handing
//! it to the proxy leaks the query the proxy exists to hide.
//!
//! With `stream_isolation` (the default once a proxy is set) every connection
//! presents freshly drawn SOCKS credentials, so a Tor daemon with
//! `IsolateSOCKSAuth` maps each logical connection to a distinct circuit.
//! Nothing here caches or reuses a stream: a caller that reads per epoch (the
//! Ark mailbox reader, §11.7) gets a fresh circuit per read simply by calling
//! [`Egress::connect`] for each one.
//!
//! Scope: `[storage]` (Postgres/Redis) and the template source never go
//! through this module, and `spc-agent`'s downstream SV1 listener stays
//! plain TCP.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use rand::RngCore;
use serde::Deserialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// `[net]` config block (SPEC §6.3), shared by `spc-pool` (ark role egress)
/// and `spc-agent` (upstream SV2, JDP, Ark/mailbox egress).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// `socks5h://ip:port`; empty string = direct connections.
    #[serde(default)]
    pub proxy: String,
    /// Per-connection SOCKS credentials (Tor `IsolateSOCKSAuth`). Defaults
    /// to true when `proxy` is set; meaningless otherwise.
    #[serde(default)]
    pub stream_isolation: Option<bool>,
}

/// A dial destination that never resolves names locally: a host stays a
/// string until either the OS resolver (direct dial of a clearnet name) or
/// the SOCKS proxy (ATYP=0x03, SOCKS5 "DOMAINNAME") sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Addr(SocketAddr),
    Name { host: String, port: u16 },
}

impl Target {
    /// Parse `IP:port` (including `[v6]:port`) or `name:port`. Anything
    /// malformed fails closed; a name is at most 255 bytes (the SOCKS5
    /// length byte) of ASCII letters, digits, `-` and `.`.
    pub fn parse(s: &str) -> Option<Self> {
        if let Ok(addr) = s.parse::<SocketAddr>() {
            return Some(Self::Addr(addr));
        }
        let (host, port) = s.rsplit_once(':')?;
        let port: u16 = port.parse().ok()?;
        let host = host
            .strip_suffix('.')
            .map(str::to_string)
            .unwrap_or_else(|| host.to_string());
        let valid = !host.is_empty()
            && host.len() <= 255
            && !host.contains(':') // unbracketed IPv6 is rejected above
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
        valid.then_some(Self::Name { host, port })
    }

    pub fn is_onion(&self) -> bool {
        match self {
            Self::Addr(_) => false,
            Self::Name { host, .. } => host_is_onion(host),
        }
    }
}

impl From<SocketAddr> for Target {
    fn from(addr: SocketAddr) -> Self {
        Self::Addr(addr)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Addr(a) => write!(f, "{a}"),
            Self::Name { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

/// RFC 1035-style name check: a `.onion` host has no direct route, ever.
pub fn host_is_onion(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    host.to_ascii_lowercase().ends_with(".onion")
}

/// Strictly extract the normalized host of an `http(s)://` URL, for the
/// startup check that a `.onion` Ark server has a proxy (SPEC §3.8, §11.3).
/// This check gates egress, so it fails closed rather than guessing: more
/// than one `@`, an empty host, whitespace or control bytes, a non-HTTP
/// scheme, or a bad port are all errors. The returned host is lowercased
/// with at most one trailing dot removed; bracketed IPv6 is supported.
pub fn url_host(url: &str) -> Result<String, NetError> {
    let bad = || NetError::Url(url.to_string());
    let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(NetError::UrlScheme(url.to_string()));
    }
    let authority = rest.split(['/', '?', '#']).next().ok_or_else(bad)?;
    if authority.is_empty()
        || authority.bytes().any(|b| b <= b' ' || b == 0x7f)
        || authority.matches('@').count() > 1
    {
        return Err(bad());
    }
    // One optional `userinfo@`; the host portion itself never contains '@'.
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = if let Some(bracketed) = hostport.strip_prefix('[') {
        let end = bracketed.find(']').ok_or_else(bad)?;
        match bracketed[end + 1..].strip_prefix(':') {
            None if end + 1 == bracketed.len() => {}
            Some(port) if port.parse::<u16>().is_ok() => {}
            _ => return Err(bad()),
        }
        bracketed[..end].to_ascii_lowercase()
    } else {
        let named = match hostport.rsplit_once(':') {
            Some((h, port)) => {
                // A bad port or a second ':' (unbracketed IPv6) is malformed.
                if h.contains(':') || port.parse::<u16>().is_err() {
                    return Err(bad());
                }
                h
            }
            None => hostport,
        };
        let lowered = named.to_ascii_lowercase();
        let lowered = lowered.strip_suffix('.').unwrap_or(&lowered);
        if lowered.is_empty() {
            return Err(bad());
        }
        lowered.to_string()
    };
    if host.is_empty() {
        return Err(bad());
    }
    Ok(host)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NetError {
    #[error("[net] proxy uses scheme '{0}'; only socks5h:// is accepted (remote name resolution is mandatory, SPEC §3.8)")]
    ProxyScheme(String),
    #[error(
        "[net] proxy must be socks5h://<IP-literal>:<port> with no userinfo, path or query: '{0}'"
    )]
    ProxyAddress(String),
    #[error("a .onion target ({0}) has no direct route; set [net] proxy (SPEC §3.8)")]
    OnionWithoutProxy(String),
    #[error("malformed URL authority: '{0}'")]
    Url(String),
    #[error("URL must use http(s)://: '{0}'")]
    UrlScheme(String),
}

impl Net {
    /// Validate the block once (startup) and return the runtime dialer.
    pub fn egress(&self) -> Result<Egress, NetError> {
        if self.proxy.trim().is_empty() {
            return Ok(Egress::DIRECT);
        }
        Ok(Egress {
            proxy: Some(parse_socks5h(&self.proxy)?),
            stream_isolation: self.stream_isolation.unwrap_or(true),
            handshake_timeout: HANDSHAKE_TIMEOUT,
        })
    }
}

/// Parse a `socks5h://IP:port` URL, rejecting every other scheme (including
/// `socks5://`, which resolves names locally and leaks the query).
pub fn parse_socks5h(url: &str) -> Result<SocketAddr, NetError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| NetError::ProxyAddress(url.to_string()))?;
    if !scheme.eq_ignore_ascii_case("socks5h") {
        return Err(NetError::ProxyScheme(scheme.to_string()));
    }
    if rest.is_empty()
        || rest
            .bytes()
            .any(|b| matches!(b, b'/' | b'@' | b'?' | b'#' | b' '))
    {
        return Err(NetError::ProxyAddress(url.to_string()));
    }
    rest.parse::<SocketAddr>()
        .map_err(|_| NetError::ProxyAddress(url.to_string()))
}

/// Validated egress policy: how the process reaches miner-facing and Ark
/// hosts. Holds no sockets and no state, so no two logical connections can
/// ever share a circuit through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Egress {
    proxy: Option<SocketAddr>,
    stream_isolation: bool,
    /// Bounds the proxy TCP connect AND the SOCKS5 handshake, so a dead but
    /// accepting proxy cannot pin a role's startup or reconnect loop.
    handshake_timeout: Duration,
}

/// Default deadline for connecting to the proxy and finishing the handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

impl Egress {
    pub const DIRECT: Self = Self {
        proxy: None,
        stream_isolation: true,
        handshake_timeout: HANDSHAKE_TIMEOUT,
    };

    pub fn proxy(&self) -> Option<SocketAddr> {
        self.proxy
    }

    /// Open one connection to `target`. Proxied dials carry freshly drawn
    /// SOCKS credentials when `stream_isolation` is on; nothing is reused
    /// between calls (one connect = one circuit, §11.7 mailbox reads).
    ///
    /// The `.onion`-without-proxy rule is enforced HERE, at the dial: the
    /// config-time checks make the same rejection early and legible, but
    /// this refusal is the single source of truth - no caller can bypass it
    /// by constructing a target itself.
    pub async fn connect(&self, target: &Target) -> io::Result<TcpStream> {
        let Some(proxy) = self.proxy else {
            if target.is_onion() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    NetError::OnionWithoutProxy(target.to_string()).to_string(),
                ));
            }
            return match target {
                Target::Addr(addr) => TcpStream::connect(*addr).await,
                Target::Name { host, port } => TcpStream::connect((host.as_str(), *port)).await,
            };
        };
        let mut stream = match timeout(self.handshake_timeout, TcpStream::connect(proxy)).await {
            Ok(inner) => inner?,
            Err(_) => return Err(timed_out("SOCKS5 proxy connect", self.handshake_timeout)),
        };
        let auth = self.stream_isolation.then(draw_credentials);
        match timeout(
            self.handshake_timeout,
            socks5_connect(&mut stream, target, auth),
        )
        .await
        {
            Ok(inner) => inner?,
            Err(_) => return Err(timed_out("SOCKS5 handshake", self.handshake_timeout)),
        }
        Ok(stream)
    }
}

fn timed_out(phase: &str, deadline: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{phase} timed out ({deadline:?} deadline)"),
    )
}

/// 32 hex characters per field: distinct per connection, which is the whole
/// point (`IsolateSOCKSAuth` keys the circuit on user+password).
fn draw_credentials() -> ([u8; 32], [u8; 32]) {
    let mut user_seed = [0u8; 16];
    let mut pass_seed = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut user_seed);
    rand::rngs::OsRng.fill_bytes(&mut pass_seed);
    (hex_encode(&user_seed), hex_encode(&pass_seed))
}

fn hex_encode(src: &[u8; 16]) -> [u8; 32] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut dst = [0u8; 32];
    for (i, b) in src.iter().enumerate() {
        dst[2 * i] = HEX[(b >> 4) as usize];
        dst[2 * i + 1] = HEX[(b & 0x0f) as usize];
    }
    dst
}

const SOCKS_VERSION: u8 = 0x05;
const METHOD_NONE: u8 = 0x00;
const METHOD_USERPASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xff;

fn protocol(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Reply codes, RFC 1928 §6. Only used for error messages.
fn reply_meaning(rep: u8) -> &'static str {
    match rep {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown failure",
    }
}

/// RFC 1928 CONNECT, with RFC 1929 username/password auth when `creds` is
/// set. Domain targets go on the wire *unresolved* (ATYP=0x03) - resolution
/// is the proxy's job, which is the entire point of `socks5h`.
async fn socks5_connect(
    stream: &mut TcpStream,
    target: &Target,
    creds: Option<([u8; 32], [u8; 32])>,
) -> io::Result<()> {
    // ---- greeting ----
    let methods: &[u8] = match creds {
        Some(_) => &[METHOD_USERPASS],
        None => &[METHOD_NONE],
    };
    let mut greeting = Vec::with_capacity(2 + methods.len());
    greeting.push(SOCKS_VERSION);
    greeting.push(methods.len() as u8);
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != SOCKS_VERSION || choice[1] == METHOD_NONE_ACCEPTABLE {
        return Err(protocol("SOCKS proxy offers no acceptable auth method"));
    }
    match &creds {
        Some((user, pass)) => {
            // Stream isolation is the reason auth exists here; a proxy that
            // silently skips it would defeat it, so fail closed (§3.8).
            if choice[1] != METHOD_USERPASS {
                return Err(protocol(
                    "SOCKS proxy skipped username/password auth; stream isolation cannot be guaranteed",
                ));
            }
            let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
            auth.push(0x01); // RFC 1929 sub-negotiation version
            auth.push(user.len() as u8);
            auth.extend_from_slice(user);
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass);
            stream.write_all(&auth).await?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status != [0x01, 0x00] {
                return Err(protocol(
                    "SOCKS proxy rejected the stream-isolation credentials",
                ));
            }
        }
        None => {
            if choice[1] != METHOD_NONE {
                return Err(protocol(
                    "SOCKS proxy demands auth this process never configures",
                ));
            }
        }
    }

    // ---- CONNECT ----
    let mut req = vec![SOCKS_VERSION, 0x01, 0x00];
    match target {
        Target::Addr(SocketAddr::V4(a)) => {
            req.push(0x01);
            req.extend_from_slice(&a.ip().octets());
            req.extend_from_slice(&a.port().to_be_bytes());
        }
        Target::Addr(SocketAddr::V6(a)) => {
            req.push(0x04);
            req.extend_from_slice(&a.ip().octets());
            req.extend_from_slice(&a.port().to_be_bytes());
        }
        Target::Name { host, port } => {
            req.push(0x03);
            req.push(
                u8::try_from(host.len())
                    .map_err(|_| protocol("SOCKS5 target name exceeds 255 bytes"))?,
            );
            req.extend_from_slice(host.as_bytes());
            req.extend_from_slice(&port.to_be_bytes());
        }
    }
    stream.write_all(&req).await?;

    // ---- reply ----
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != SOCKS_VERSION {
        return Err(protocol("garbage from the SOCKS proxy"));
    }
    if head[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!(
                "SOCKS connect to {target} failed: {:#04x} ({})",
                head[1],
                reply_meaning(head[1])
            ),
        ));
    }
    // Consume BND.ADDR/BND.PORT; the bound address is advisory.
    let skip = match head[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            usize::from(len[0])
        }
        0x04 => 16,
        _ => {
            return Err(protocol(
                "SOCKS proxy answered with an unknown address type",
            ))
        }
    };
    let mut bound = vec![0u8; skip + 2];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn proxy_url_requires_socks5h_and_an_ip_literal() {
        assert_eq!(
            parse_socks5h("socks5h://127.0.0.1:9050").unwrap(),
            "127.0.0.1:9050".parse().unwrap()
        );
        assert_eq!(
            parse_socks5h("socks5h://[::1]:9050").unwrap(),
            "[::1]:9050".parse().unwrap()
        );
        for (url, scheme) in [
            ("socks5://127.0.0.1:9050", "socks5"),
            ("http://127.0.0.1:9050", "http"),
            ("https://127.0.0.1:9050", "https"),
            ("tor://127.0.0.1:9050", "tor"),
        ] {
            assert_eq!(
                parse_socks5h(url),
                Err(NetError::ProxyScheme(scheme.to_string())),
                "{url}"
            );
        }
        for url in [
            "socks5h://localhost:9050", // proxy host is an IP literal only
            "socks5h://user@127.0.0.1:9050",
            "socks5h://127.0.0.1:9050/",
            "socks5h://127.0.0.1",
            "127.0.0.1:9050",
            "socks5h://",
        ] {
            assert!(matches!(parse_socks5h(url), Err(NetError::ProxyAddress(_))));
        }
    }

    #[test]
    fn net_defaults_to_direct_and_isolation_when_proxied() {
        assert_eq!(Net::default().egress().unwrap(), Egress::DIRECT);
        let net: Net = toml::from_str(r#"proxy = "socks5h://127.0.0.1:9050""#).unwrap();
        let egress = net.egress().unwrap();
        assert_eq!(egress.proxy(), Some("127.0.0.1:9050".parse().unwrap()));
        assert!(egress.stream_isolation);
        let net: Net =
            toml::from_str("proxy = \"socks5h://127.0.0.1:9050\"\nstream_isolation = false")
                .unwrap();
        assert!(!net.egress().unwrap().stream_isolation);
        // An unknown key is a config error, not a silent default.
        assert!(toml::from_str::<Net>(r#"proxies = "socks5h://127.0.0.1:9050""#).is_err());
    }

    #[test]
    fn target_parsing_accepts_names_and_ips() {
        assert_eq!(
            Target::parse("127.0.0.1:34254"),
            Some(Target::Addr("127.0.0.1:34254".parse().unwrap()))
        );
        assert_eq!(
            Target::parse("[::1]:9050"),
            Some(Target::Addr("[::1]:9050".parse().unwrap()))
        );
        assert_eq!(
            Target::parse("pool.example:34254"),
            Some(Target::Name {
                host: "pool.example".into(),
                port: 34254
            })
        );
        for bad in [
            "pool.example",
            "a:b:c",
            "host:notaport",
            ":9050",
            "bad host:1",
            "",
        ] {
            assert_eq!(Target::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn onion_detection_is_robust_to_case_and_trailing_dot() {
        assert!(host_is_onion("abc.onion"));
        assert!(host_is_onion("ABC.ONION"));
        assert!(host_is_onion("abc.onion."));
        assert!(!host_is_onion("onion.example"));
        assert!(!host_is_onion("127.0.0.1"));
        assert!(Target::parse("abc.onion:9050").unwrap().is_onion());
        assert!(!Target::parse("127.0.0.1:9050").unwrap().is_onion());
    }

    #[test]
    fn url_host_parses_strictly_and_normalizes() {
        assert_eq!(
            url_host("https://ark.example.tech").unwrap(),
            "ark.example.tech"
        );
        // One userinfo '@' is fine: the host is still the authority host.
        assert_eq!(
            url_host("https://user:pw@app.onion:443/path?q=1").unwrap(),
            "app.onion"
        );
        assert_eq!(url_host("https://USER@good.onion").unwrap(), "good.onion");
        assert_eq!(url_host("http://[::1]:8080/x").unwrap(), "::1");
        // Case and one trailing dot normalize away.
        assert_eq!(url_host("HTTPS://GOOD.ONION./").unwrap(), "good.onion");
        assert!(host_is_onion(&url_host("https://GOOD.ONION.:443").unwrap()));
    }

    #[test]
    fn url_host_rejects_ambiguous_or_malformed_authorities() {
        for bad in [
            "https://a@b.onion@c.onion", // two '@': the authority is ambiguous
            "https://",
            "https:///x",
            "https://:443/",
            "https://@:443/",
            "https://good onion/",
            "https://go\tod.onion/",
            "https://example:70000/",
            "https://example:notaport/",
            "https://::1:443/", // unbracketed IPv6
            "https://[::1",
            "https://[::1]x",
            "not-a-url",
            "",
        ] {
            assert!(
                matches!(url_host(bad), Err(NetError::Url(_))),
                "{bad} must fail closed"
            );
        }
        for scheme in ["socks5h://good.onion:9050", "ftp://good.onion"] {
            assert!(
                matches!(url_host(scheme), Err(NetError::UrlScheme(_))),
                "{scheme}"
            );
        }
    }

    #[tokio::test]
    async fn direct_egress_refuses_onion_targets() {
        let target = Target::parse("abc.onion:34254").unwrap();
        let err = Egress::DIRECT.connect(&target).await.unwrap_err();
        assert!(err.to_string().contains("[net] proxy"));
    }

    /// A fake SOCKS5 server: performs the RFC 1928/1929 handshake, records
    /// the username and the CONNECT target, then echoes one payload byte.
    /// Returns per-connection `(username, atyp, target_bytes, port)`.
    async fn run_fake_socks(
        listener: TcpListener,
        conns: usize,
    ) -> io::Result<Vec<(Option<String>, u8, Vec<u8>, u16)>> {
        let mut out = Vec::with_capacity(conns);
        for _ in 0..conns {
            let (mut s, _) = listener.accept().await?;
            let mut head = [0u8; 2];
            s.read_exact(&mut head).await?;
            assert_eq!(head[0], SOCKS_VERSION);
            let mut methods = vec![0u8; head[1] as usize];
            s.read_exact(&mut methods).await?;
            let user = if methods.contains(&METHOD_USERPASS) {
                s.write_all(&[SOCKS_VERSION, METHOD_USERPASS]).await?;
                let mut sub = [0u8; 2];
                s.read_exact(&mut sub).await?;
                assert_eq!(sub[0], 0x01);
                let mut u = vec![0u8; sub[1] as usize];
                s.read_exact(&mut u).await?;
                let mut plen = [0u8; 1];
                s.read_exact(&mut plen).await?;
                let mut p = vec![0u8; plen[0] as usize];
                s.read_exact(&mut p).await?;
                s.write_all(&[0x01, 0x00]).await?;
                Some(String::from_utf8(u).unwrap())
            } else {
                s.write_all(&[SOCKS_VERSION, METHOD_NONE]).await?;
                None
            };
            let mut req = [0u8; 4];
            s.read_exact(&mut req).await?;
            assert_eq!(req, [SOCKS_VERSION, 0x01, 0x00, req[3]]);
            let atyp = req[3];
            let addr = match atyp {
                0x01 => {
                    let mut a = [0u8; 4];
                    s.read_exact(&mut a).await?;
                    a.to_vec()
                }
                0x03 => {
                    let mut len = [0u8; 1];
                    s.read_exact(&mut len).await?;
                    let mut a = vec![0u8; len[0] as usize];
                    s.read_exact(&mut a).await?;
                    a
                }
                other => panic!("unexpected ATYP {other:#04x}"),
            };
            let mut port = [0u8; 2];
            s.read_exact(&mut port).await?;
            s.write_all(&[SOCKS_VERSION, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                .await?;
            let mut byte = [0u8; 1];
            s.read_exact(&mut byte).await?;
            s.write_all(&byte).await?; // prove the stream is usable end to end
            out.push((user, atyp, addr, u16::from_be_bytes(port)));
        }
        Ok(out)
    }

    #[tokio::test]
    async fn isolation_draws_fresh_credentials_and_ships_unresolved_names() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(run_fake_socks(listener, 3));
        let egress = Egress {
            proxy: Some(addr),
            stream_isolation: true,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        };
        let target = Target::parse("pool-example.onion:34254").unwrap();
        for _ in 0..3 {
            let mut s = egress.connect(&target).await.unwrap();
            s.write_all(&[0x42]).await.unwrap();
            let mut echo = [0u8; 1];
            s.read_exact(&mut echo).await.unwrap();
            assert_eq!(echo, [0x42]);
        }
        let seen = server.await.unwrap().unwrap();
        let users: Vec<&str> = seen
            .iter()
            .map(|(u, _, _, _)| u.as_deref().unwrap())
            .collect();
        assert_ne!(users[0], users[1]);
        assert_ne!(users[1], users[2]);
        assert_ne!(users[0], users[2]);
        for (_, atyp, addr, port) in &seen {
            assert_eq!(*atyp, 0x03, "the name must arrive unresolved");
            assert_eq!(addr, b"pool-example.onion");
            assert_eq!(*port, 34254);
        }
    }

    #[tokio::test]
    async fn without_isolation_the_proxy_sees_no_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(run_fake_socks(listener, 2));
        let egress = Egress {
            proxy: Some(addr),
            stream_isolation: false,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        };
        let target = Target::from("127.0.0.1:34254".parse::<SocketAddr>().unwrap());
        for _ in 0..2 {
            let mut s = egress.connect(&target).await.unwrap();
            s.write_all(&[0x01]).await.unwrap();
            let mut echo = [0u8; 1];
            s.read_exact(&mut echo).await.unwrap();
        }
        let seen = server.await.unwrap().unwrap();
        for (user, atyp, addr, _) in &seen {
            assert_eq!(*user, None);
            assert_eq!(*atyp, 0x01, "an IP target stays an IP");
            assert_eq!(addr, &[127, 0, 0, 1]);
        }
    }

    #[tokio::test]
    async fn handshake_failures_are_errors_not_hangs_or_fallbacks() {
        // A server that rejects every method.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let n = tokio::io::AsyncReadExt::read(&mut s, &mut buf)
                .await
                .unwrap();
            assert!(n >= 3);
            s.write_all(&[SOCKS_VERSION, METHOD_NONE_ACCEPTABLE])
                .await
                .unwrap();
        });
        let egress = Egress {
            proxy: Some(addr),
            stream_isolation: true,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        };
        let target = Target::parse("127.0.0.1:1").unwrap();
        let err = egress.connect(&target).await.unwrap_err();
        assert!(err.to_string().contains("no acceptable auth method"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_stalled_proxy_fails_at_the_deadline_not_never() {
        use std::time::Instant;

        // Accepts the TCP connection and then says nothing at all.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_silent, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let egress = Egress {
            proxy: Some(addr),
            stream_isolation: true,
            handshake_timeout: Duration::from_millis(150),
        };
        let target = Target::parse("pool.example.onion:34254").unwrap();
        let start = Instant::now();
        let err = egress.connect(&target).await.unwrap_err();
        assert!(
            err.to_string().contains("SOCKS5 handshake timed out"),
            "{err}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "the deadline must actually bound the wait"
        );
        server.abort();
    }
}
