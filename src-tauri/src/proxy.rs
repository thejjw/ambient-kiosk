use crate::config::NetworkDnsConfig;
use std::net::{IpAddr, SocketAddr};
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
/// Returns Some(IpAddr) on success, or None if no valid A record is present.
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
            let ip = std::net::Ipv4Addr::new(resp[idx], resp[idx + 1], resp[idx + 2], resp[idx + 3]);
            return Some(IpAddr::V4(ip));
        }
        idx += rdlength;
    }
    None
}

/// Resolves a hostname upstream using RFC 8484 DNS-over-HTTPS (DoH).
/// Returns Some(IpAddr) if resolved, or None if lookup fails.
/// Note: AdGuard returns 0.0.0.0 for blocked ad/tracker domains.
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
/// Handles both "ip:port" (e.g. "94.140.14.14:53") and bare "ip" formats.
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

/// Starts an embedded loopback proxy on 127.0.0.1:0.
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

                let req_str = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_str.lines().next().unwrap_or("");

                if first_line.starts_with("CONNECT ") {
                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let target = parts[1];
                        let mut host_port = target.split(':');
                        let host = host_port.next().unwrap_or("");
                        let port: u16 = host_port.next().and_then(|p| p.parse().ok()).unwrap_or(443);

                        let target_ip = match resolve_host_adguard(host, &cfg_clone).await {
                            Some(ip) => {
                                if ip.is_unspecified() {
                                    // Blocked tracker/ad domain
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
                        match TcpStream::connect(connect_addr).await {
                            Ok(mut upstream) => {
                                if client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_ok() {
                                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                                }
                            }
                            Err(_) => {
                                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                            }
                        }
                    }
                } else if first_line.starts_with("GET ") {
                    let _ = client.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nOK").await;
                } else {
                    let _ = client.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n").await;
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
    async fn test_proxy_lifecycle() {
        let cfg = NetworkDnsConfig::default();
        let port = start_adguard_proxy(cfg).await.expect("Proxy should start");
        assert!(port > 0, "Proxy port must be non-zero");

        // Verify TCP connect to proxy
        let stream = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
        assert!(stream.is_ok(), "Should connect to proxy TCP port");
    }
}
