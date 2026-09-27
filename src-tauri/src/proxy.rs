use crate::config::NetworkDnsConfig;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

/// Constructs an RFC 1035 / RFC 8484 DNS Type A query packet.
pub fn build_dns_query_packet(host: &str) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(512);
    // Header: ID=0xbeef, QR=0, Opcode=0, RD=1, QDCOUNT=1
    packet.extend_from_slice(&[0xbe, 0xef, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // Type A, Class IN
    Some(packet)
}

/// Parses an RFC 1035 DNS response packet and extracts the first IPv4 (A) record.
pub fn parse_dns_response_packet(resp: &[u8]) -> Option<IpAddr> {
    if resp.len() < 12 {
        return None;
    }
    let ancount = u16::from_be_bytes([resp[6], resp[7]]);
    if ancount == 0 {
        return None;
    }

    let mut idx = 12;
    while idx < resp.len() && resp[idx] != 0 {
        if resp[idx] & 0xc0 == 0xc0 {
            idx += 1;
            break;
        }
        idx += (resp[idx] as usize) + 1;
    }
    idx += 5;

    for _ in 0..ancount {
        if idx >= resp.len() {
            break;
        }
        if resp[idx] & 0xc0 == 0xc0 {
            idx += 2;
        } else {
            while idx < resp.len() && resp[idx] != 0 {
                idx += (resp[idx] as usize) + 1;
            }
            idx += 1;
        }
        if idx + 10 > resp.len() {
            break;
        }
        let rtype = u16::from_be_bytes([resp[idx], resp[idx + 1]]);
        let rdlength = u16::from_be_bytes([resp[idx + 8], resp[idx + 9]]) as usize;
        idx += 10;
        if rtype == 1 && rdlength == 4 && idx + 4 <= resp.len() {
            let ip = Ipv4Addr::new(resp[idx], resp[idx + 1], resp[idx + 2], resp[idx + 3]);
            return Some(IpAddr::V4(ip));
        }
        idx += rdlength;
    }
    None
}

/// Resolves a hostname upstream using RFC 8484 DNS-over-HTTPS (DoH).
pub async fn resolve_with_adguard_doh(host: &str, doh_endpoint: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let query_packet = build_dns_query_packet(host)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(2000))
        .build()
        .ok()?;

    let resp = client
        .post(doh_endpoint)
        .header("Content-Type", "application/dns-message")
        .header("Accept", "application/dns-message")
        .body(query_packet)
        .send()
        .await
        .ok()?;

    if resp.status().is_success() {
        let bytes = resp.bytes().await.ok()?;
        return parse_dns_response_packet(&bytes);
    }
    None
}

/// Fallback plain DNS resolver querying AdGuard over UDP.
pub async fn resolve_with_adguard_plain_udp(host: &str, plain_dns_ip: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let query_packet = build_dns_query_packet(host)?;

    let server_addr: SocketAddr = if plain_dns_ip.contains(':') {
        plain_dns_ip.parse().ok()?
    } else {
        format!("{}:53", plain_dns_ip).parse().ok()?
    };

    let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect(server_addr).await.ok()?;
    socket.send(&query_packet).await.ok()?;

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_millis(1500), socket.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_dns_response_packet(&buf[..n])
}

/// Resolves host using configured DNS cascade: DoH -> Plain UDP -> None.
pub async fn resolve_host_adguard(host: &str, cfg: &NetworkDnsConfig) -> Option<IpAddr> {
    if let Some(ip) = resolve_with_adguard_doh(host, &cfg.doh_url).await {
        return Some(ip);
    }
    resolve_with_adguard_plain_udp(host, &cfg.plain_dns_ip).await
}

/// Handles SOCKS5 protocol handshake, request parsing, AdGuard resolution, and forwarding.
async fn handle_socks5(
    mut client: TcpStream,
    initial_buf: &[u8],
    cfg: Arc<NetworkDnsConfig>,
) {
    // 1. Negotiation Greeting
    // initial_buf contains [0x05, nmethods, methods...]
    if initial_buf.len() < 2 || initial_buf[0] != 0x05 {
        return;
    }
    let nmethods = initial_buf[1] as usize;
    let mut methods = initial_buf[2..].to_vec();
    while methods.len() < nmethods {
        let mut tmp = [0u8; 64];
        match client.read(&mut tmp).await {
            Ok(n) if n > 0 => methods.extend_from_slice(&tmp[..n]),
            _ => return,
        }
    }

    // Reply: 0x05 (version 5), 0x00 (no authentication required)
    if client.write_all(&[0x05, 0x00]).await.is_err() {
        return;
    }

    // 2. Client Connection Request
    // [VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT]
    let mut header = [0u8; 4];
    if client.read_exact(&mut header).await.is_err() {
        return;
    }
    if header[0] != 0x05 || header[1] != 0x01 {
        // CMD != 1 (CONNECT): command not supported
        let _ = client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
        return;
    }

    let (target_host, target_ip_opt) = match header[3] {
        0x01 => {
            // IPv4: 4 bytes
            let mut ip_bytes = [0u8; 4];
            if client.read_exact(&mut ip_bytes).await.is_err() {
                return;
            }
            let ip = IpAddr::V4(Ipv4Addr::from(ip_bytes));
            (ip.to_string(), Some(ip))
        }
        0x03 => {
            // Domain: 1 byte len + domain bytes
            let mut len_byte = [0u8; 1];
            if client.read_exact(&mut len_byte).await.is_err() {
                return;
            }
            let mut domain_bytes = vec![0u8; len_byte[0] as usize];
            if client.read_exact(&mut domain_bytes).await.is_err() {
                return;
            }
            let host = String::from_utf8_lossy(&domain_bytes).to_string();
            (host, None)
        }
        0x04 => {
            // IPv6: 16 bytes
            let mut ip_bytes = [0u8; 16];
            if client.read_exact(&mut ip_bytes).await.is_err() {
                return;
            }
            let ip = IpAddr::V6(Ipv6Addr::from(ip_bytes));
            (ip.to_string(), Some(ip))
        }
        _ => return,
    };

    let mut port_bytes = [0u8; 2];
    if client.read_exact(&mut port_bytes).await.is_err() {
        return;
    }
    let port = u16::from_be_bytes(port_bytes);

    // 3. Resolve target host via AdGuard
    let resolved_ip = match target_ip_opt {
        Some(ip) => ip,
        None => match resolve_host_adguard(&target_host, &cfg).await {
            Some(ip) => ip,
            None => {
                // Host unreachable
                let _ = client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
                return;
            }
        },
    };

    if resolved_ip.is_unspecified() {
        // Blocked by ruleset: 0x02 = connection not allowed by ruleset
        let _ = client.write_all(&[0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
        return;
    }

    // 4. Connect upstream
    let target_addr = SocketAddr::new(resolved_ip, port);
    match TcpStream::connect(target_addr).await {
        Ok(mut upstream) => {
            // Success reply: 0x00 = succeeded
            if client.write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0]).await.is_ok() {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            }
        }
        Err(_) => {
            // Connection refused: 0x05
            let _ = client.write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
        }
    }
}

/// Parses target host and port from HTTP CONNECT request or absolute-form HTTP request.
fn parse_http_target(first_line: &str, full_headers: &str) -> Option<(String, u16)> {
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    if parts[0].eq_ignore_ascii_case("CONNECT") {
        let target = parts[1];
        let mut hp = target.split(':');
        let host = hp.next()?.to_string();
        let port: u16 = hp.next().and_then(|p| p.parse().ok()).unwrap_or(443);
        return Some((host, port));
    }

    // Plain HTTP request: check absolute URI in request line (e.g. GET http://example.com:8080/path HTTP/1.1)
    let uri = parts[1];
    if let Ok(parsed_url) = url::Url::parse(uri) {
        if let Some(host) = parsed_url.host_str() {
            let port = parsed_url.port().unwrap_or(80);
            return Some((host.to_string(), port));
        }
    }

    // Fallback: parse Host header
    for line in full_headers.lines() {
        if let Some(rest) = line.strip_prefix("Host:").or_else(|| line.strip_prefix("host:")) {
            let trimmed = rest.trim();
            let mut hp = trimmed.split(':');
            let host = hp.next()?.to_string();
            let port: u16 = hp.next().and_then(|p| p.parse().ok()).unwrap_or(80);
            return Some((host, port));
        }
    }

    None
}

/// Handles HTTP CONNECT tunneling and plain absolute-form HTTP forwarding.
async fn handle_http(
    mut client: TcpStream,
    initial_buf: &[u8],
    n: usize,
    cfg: Arc<NetworkDnsConfig>,
) {
    let req_str = String::from_utf8_lossy(&initial_buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");

    let (host, port) = match parse_http_target(first_line, &req_str) {
        Some(target) => target,
        None => {
            let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return;
        }
    };

    let target_ip = match resolve_host_adguard(&host, &cfg).await {
        Some(ip) => {
            if ip.is_unspecified() {
                // AdGuard blocked tracker
                let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await;
                return;
            }
            ip
        }
        None => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return;
        }
    };

    let connect_addr = SocketAddr::new(target_ip, port);
    let mut upstream = match TcpStream::connect(connect_addr).await {
        Ok(s) => s,
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return;
        }
    };

    if first_line.starts_with("CONNECT ") {
        // HTTPS tunnel establishment
        if client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_ok() {
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        }
    } else {
        // Plain HTTP forwarding: rewrite absolute-form URI to origin-form URI
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        let payload_to_send = if parts.len() >= 3 && parts[1].starts_with("http://") {
            if let Ok(u) = url::Url::parse(parts[1]) {
                let path_and_query = match u.query() {
                    Some(q) => format!("{}?{}", u.path(), q),
                    None => u.path().to_string(),
                };
                let new_first_line = format!("{} {} {}", parts[0], path_and_query, parts[2]);
                let rest_of_request = &req_str[first_line.len()..];
                format!("{}{}", new_first_line, rest_of_request).into_bytes()
            } else {
                initial_buf[..n].to_vec()
            }
        } else {
            initial_buf[..n].to_vec()
        };

        if upstream.write_all(&payload_to_send).await.is_ok() {
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        }
    }
}

/// Starts an embedded loopback proxy multiplexing HTTP CONNECT, plain HTTP, and SOCKS5 on 127.0.0.1:0.
/// Returns the bound ephemeral port.
pub async fn start_adguard_proxy(cfg: NetworkDnsConfig) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local adblocking proxy: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get proxy local address: {}", e))?
        .port();

    let cfg_arc = Arc::new(cfg);

    tokio::spawn(async move {
        loop {
            let (mut client, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };

            let cfg_clone = cfg_arc.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = match client.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => return,
                };

                if buf[0] == 0x05 {
                    // SOCKS5 protocol
                    handle_socks5(client, &buf[..n], cfg_clone).await;
                } else {
                    // HTTP CONNECT / Plain HTTP forward proxy
                    handle_http(client, &buf, n, cfg_clone).await;
                }
            });
        }
    });

    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_adguard_doh_rfc8484_resolution() {
        let cfg = NetworkDnsConfig::default();
        let res = resolve_with_adguard_doh("biztoc.com", &cfg.doh_url).await;
        assert!(res.is_some(), "biztoc.com should resolve via default AdGuard DoH (rfc8484)");
        let ip = res.unwrap();
        assert!(!ip.is_unspecified(), "biztoc.com IP must not be 0.0.0.0");
    }

    #[tokio::test]
    async fn test_adguard_plain_udp_resolution() {
        let cfg = NetworkDnsConfig::default();
        let res = resolve_with_adguard_plain_udp("biztoc.com", &cfg.plain_dns_ip).await;
        assert!(res.is_some(), "biztoc.com should resolve via default AdGuard plain UDP ({})", cfg.plain_dns_ip);
        let ip = res.unwrap();
        assert!(!ip.is_unspecified(), "biztoc.com IP must not be 0.0.0.0");
    }

    #[tokio::test]
    async fn test_adguard_tracker_blocking() {
        let cfg = NetworkDnsConfig::default();
        // Blocked domain: adservice.google.com
        let res_doh = resolve_with_adguard_doh("adservice.google.com", &cfg.doh_url).await;
        assert!(res_doh.is_some(), "AdGuard DoH should return response for tracker");
        assert!(res_doh.unwrap().is_unspecified(), "Tracker domain must resolve to 0.0.0.0");

        let res_udp = resolve_with_adguard_plain_udp("adservice.google.com", &cfg.plain_dns_ip).await;
        assert!(res_udp.is_some(), "AdGuard plain UDP should return response for tracker");
        assert!(res_udp.unwrap().is_unspecified(), "Tracker domain must resolve to 0.0.0.0 via plain UDP");
    }

    #[tokio::test]
    async fn test_proxy_plain_http_forwarding_end_to_end() {
        // 1. Start mock HTTP upstream
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = upstream_listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("GET /test HTTP/1.1"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nHELLO_UPSTREAM").await.unwrap();
        });

        // 2. Start proxy
        let cfg = NetworkDnsConfig::default();
        let proxy_port = start_adguard_proxy(cfg).await.unwrap();

        // 3. Connect client to proxy and send absolute-form HTTP request
        let mut client = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port))).await.unwrap();
        let req = format!("GET http://127.0.0.1:{}/test HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", upstream_port, upstream_port);
        client.write_all(req.as_bytes()).await.unwrap();

        let mut resp = Vec::new();
        client.read_to_end(&mut resp).await.unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.contains("HTTP/1.1 200 OK"), "Must receive 200 OK from upstream");
        assert!(resp_str.contains("HELLO_UPSTREAM"), "Must receive body payload from upstream");
    }

    #[tokio::test]
    async fn test_proxy_http_connect_tunneling_end_to_end() {
        // 1. Start mock TCP upstream
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = upstream_listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"PING_TUNNEL");
            socket.write_all(b"PONG_TUNNEL").await.unwrap();
        });

        // 2. Start proxy
        let cfg = NetworkDnsConfig::default();
        let proxy_port = start_adguard_proxy(cfg).await.unwrap();

        // 3. Connect to proxy and establish CONNECT tunnel
        let mut client = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port))).await.unwrap();
        let connect_req = format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", upstream_port);
        client.write_all(connect_req.as_bytes()).await.unwrap();

        let mut buf = [0u8; 1024];
        let n = client.read(&mut buf).await.unwrap();
        let status = String::from_utf8_lossy(&buf[..n]);
        assert!(status.contains("200 Connection Established"), "Must establish CONNECT tunnel");

        // Send payload through tunnel
        client.write_all(b"PING_TUNNEL").await.unwrap();
        let n = client.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"PONG_TUNNEL", "Must receive piped tunnel bytes");
    }

    #[tokio::test]
    async fn test_proxy_socks5_forwarding_end_to_end() {
        // 1. Start mock TCP upstream
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = upstream_listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"SOCKS5_PING");
            socket.write_all(b"SOCKS5_PONG").await.unwrap();
        });

        // 2. Start proxy
        let cfg = NetworkDnsConfig::default();
        let proxy_port = start_adguard_proxy(cfg).await.unwrap();

        // 3. Connect client and perform SOCKS5 handshake
        let mut client = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port))).await.unwrap();
        // Client Greeting: [VER=5, NMETHODS=1, METHOD=0 (No Auth)]
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();

        let mut greet_resp = [0u8; 2];
        client.read_exact(&mut greet_resp).await.unwrap();
        assert_eq!(greet_resp, [0x05, 0x00], "SOCKS5 server must accept no-auth");

        // 4. Client Request: CONNECT to 127.0.0.1:<upstream_port> (IPv4 ATYP=0x01)
        let mut req = vec![0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1];
        req.extend_from_slice(&upstream_port.to_be_bytes());
        client.write_all(&req).await.unwrap();

        let mut conn_resp = [0u8; 10];
        client.read_exact(&mut conn_resp).await.unwrap();
        assert_eq!(conn_resp[0], 0x05);
        assert_eq!(conn_resp[1], 0x00, "SOCKS5 connect must succeed (REP=0)");

        // 5. Send payload through SOCKS5 tunnel
        client.write_all(b"SOCKS5_PING").await.unwrap();
        let mut buf = [0u8; 64];
        let n = client.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"SOCKS5_PONG", "Must receive piped SOCKS5 bytes");
    }

    #[tokio::test]
    async fn test_proxy_tracker_blocking_enforcement() {
        let cfg = NetworkDnsConfig::default();
        let proxy_port = start_adguard_proxy(cfg).await.unwrap();

        // 1. HTTP CONNECT to blocked tracker must receive 403
        let mut client_http = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port))).await.unwrap();
        client_http.write_all(b"CONNECT adservice.google.com:443 HTTP/1.1\r\n\r\n").await.unwrap();
        let mut buf = [0u8; 512];
        let n = client_http.read(&mut buf).await.unwrap();
        let resp = String::from_utf8_lossy(&buf[..n]);
        assert!(resp.contains("403 Forbidden"), "HTTP proxy must return 403 for blocked tracker");

        // 2. SOCKS5 CONNECT to blocked tracker must receive REP=0x02
        let mut client_socks = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port))).await.unwrap();
        client_socks.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut greet = [0u8; 2];
        client_socks.read_exact(&mut greet).await.unwrap();

        // SOCKS5 request for domain "adservice.google.com" (ATYP=0x03)
        let domain = b"adservice.google.com";
        let mut req = vec![0x05, 0x01, 0x00, 0x03, domain.len() as u8];
        req.extend_from_slice(domain);
        req.extend_from_slice(&443u16.to_be_bytes());
        client_socks.write_all(&req).await.unwrap();

        let mut resp = [0u8; 10];
        let n = client_socks.read(&mut resp).await.unwrap();
        assert!(n >= 2);
        assert_eq!(resp[0], 0x05);
        assert_eq!(resp[1], 0x02, "SOCKS5 must return REP=0x02 (connection not allowed) for blocked tracker");
    }
}
