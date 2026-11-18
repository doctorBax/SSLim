use std::net::{SocketAddr, ToSocketAddrs, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use bytes::{BufMut, Bytes, BytesMut};
use clap::Subcommand;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, TransportConfig};
use ring::hmac;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::auth::read_key;
use crate::protocol::valid_ip;
use crate::session::relay;
use crate::tun::Tun;
use crate::{client_config, client_tls, server_config};

const ALPN: &[u8] = b"sslim/2";
const ALPN_V3: &[u8] = b"sslim/3";
const AUTH_LABEL_V3: &[u8] = b"EXPORTER-SSLim-v3-auth";
const AUTH_LABEL: &[u8] = b"EXPORTER-SSLim-v2-auth";
const MTU: usize = 1280;
const FRAGMENT: usize = 1024;
const HEADER: usize = 5;
const SLOTS: usize = 512;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    Server {
        #[arg(long, default_value = "127.0.0.1:9543")]
        listen: SocketAddr,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        secret_file: PathBuf,
        #[arg(long)]
        tun_name: Option<String>,
    },
    Client {
        #[arg(long)]
        address: SocketAddr,
        #[arg(long)]
        server_name: String,
        #[arg(long)]
        ca: Option<PathBuf>,
        #[arg(long)]
        secret_file: PathBuf,
        #[arg(long, default_value = "labtun0")]
        tun_name: String,
        /// Try v1 on this TCP port if the UDP handshake times out
        #[arg(long)]
        fallback_tcp_port: Option<u16>,
    },
    Probe {
        #[arg(long)]
        address: SocketAddr,
        #[arg(long)]
        server_name: String,
        #[arg(long)]
        ca: Option<PathBuf>,
        #[arg(long)]
        secret_file: PathBuf,
    },
    /// Verify v3 authentication, dynamic IPv4 allocation and ICMP echo
    ProbeV3 {
        #[arg(long)]
        address: SocketAddr,
        #[arg(long)]
        server_name: String,
        #[arg(long)]
        ca: Option<PathBuf>,
        #[arg(long)]
        secret_file: PathBuf,
    },
}

fn transport() -> Arc<TransportConfig> {
    let mut config = TransportConfig::default();
    config
        .datagram_receive_buffer_size(Some(512 * 1024))
        .datagram_send_buffer_size(256 * 1024)
        .max_concurrent_bidi_streams(1_u32.into())
        .max_concurrent_uni_streams(0_u32.into())
        .keep_alive_interval(Some(Duration::from_secs(5)));
    Arc::new(config)
}

fn auth_material(conn: &Connection) -> Result<[u8; 32]> {
    let mut material = [0; 32];
    conn.export_keying_material(&mut material, AUTH_LABEL, ALPN)
        .map_err(|_| anyhow::anyhow!("QUIC keying material unavailable"))?;
    Ok(material)
}

async fn authenticate(conn: &Connection, key: &hmac::Key) -> Result<()> {
    timeout(OPERATION_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        let tag = hmac::sign(key, &auth_material(conn)?);
        send.write_all(tag.as_ref()).await?;
        send.finish()?;
        let mut result = [0];
        recv.read_exact(&mut result).await?;
        ensure!(result == [1], "QUIC server declined the tunnel");
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

fn encode(sequence: u32, kind: u8, packet: &[u8]) -> Bytes {
    let mut data = BytesMut::with_capacity(HEADER + packet.len());
    data.put_u32(sequence);
    data.put_u8(kind);
    data.extend_from_slice(packet);
    data.freeze()
}

async fn send_packet(conn: &Connection, sequence: &mut u32, packet: &[u8]) -> Result<()> {
    let max = conn
        .max_datagram_size()
        .context("peer does not support QUIC DATAGRAM")?;
    ensure!(max >= HEADER + FRAGMENT, "QUIC path payload is too small");
    let id = *sequence;
    *sequence = sequence.wrapping_add(1);
    if HEADER + packet.len() <= max {
        conn.send_datagram_wait(encode(id, 0, packet)).await?;
    } else {
        conn.send_datagram_wait(encode(id, 1, &packet[..FRAGMENT]))
            .await?;
        conn.send_datagram_wait(encode(id, 2, &packet[FRAGMENT..]))
            .await?;
    }
    Ok(())
}

struct Pending {
    id: u32,
    since: Instant,
    pieces: [Option<Bytes>; 2],
}

struct DatagramReader {
    slots: Vec<Option<Pending>>,
}

impl DatagramReader {
    fn new() -> Self {
        Self {
            slots: (0..SLOTS).map(|_| None).collect(),
        }
    }

    fn packet(&mut self, frame: Bytes) -> Option<Bytes> {
        if !(HEADER + 1..=HEADER + MTU).contains(&frame.len()) {
            return None;
        }
        let id = u32::from_be_bytes(frame[..4].try_into().ok()?);
        let kind = frame[4];
        let data = frame.slice(HEADER..);
        if kind == 0 {
            return valid_ip(&data).then_some(data);
        }
        if !matches!((kind, data.len()), (1, FRAGMENT) | (2, 1..=256)) {
            return None;
        }
        let slot = &mut self.slots[id as usize % SLOTS];
        if slot
            .as_ref()
            .is_none_or(|p| p.id != id || p.since.elapsed() >= Duration::from_secs(2))
        {
            *slot = Some(Pending {
                id,
                since: Instant::now(),
                pieces: [None, None],
            });
        }
        let pending = slot.as_mut()?;
        pending.pieces[usize::from(kind - 1)] = Some(data);
        let [Some(first), Some(last)] = &pending.pieces else {
            return None;
        };
        let mut packet = BytesMut::with_capacity(first.len() + last.len());
        packet.extend_from_slice(first);
        packet.extend_from_slice(last);
        *slot = None;
        valid_ip(&packet).then(|| packet.freeze())
    }

    async fn read(&mut self, conn: &Connection) -> Result<Bytes> {
        loop {
            if let Some(packet) = self.packet(conn.read_datagram().await?) {
                return Ok(packet);
            }
        }
    }
}

async fn pump(conn: &Connection, tun: Tun) -> Result<()> {
    let uplink = async {
        let mut sequence = 0;
        // Read one extra byte so an oversized IPv4 or IPv6 frame cannot be accepted truncated.
        let mut packet = [0; MTU + 1];
        loop {
            let size = tun.read(&mut packet).await?;
            if size <= MTU && valid_ip(&packet[..size]) {
                send_packet(conn, &mut sequence, &packet[..size]).await?;
            }
        }
    };
    let downlink = async {
        let mut reader = DatagramReader::new();
        loop {
            let packet = reader.read(conn).await?;
            tun.write(&packet).await?;
        }
    };
    tokio::select! {
        result = uplink => result,
        result = downlink => result,
    }
}

async fn handle(conn: Connection, key: Arc<hmac::Key>, tun_name: Option<String>) -> Result<()> {
    let (tun, mut send) = timeout(OPERATION_TIMEOUT, async {
        let (send, mut recv) = conn.accept_bi().await?;
        let mut proof = [0; 32];
        recv.read_exact(&mut proof).await?;
        hmac::verify(&key, &auth_material(&conn)?, &proof)
            .map_err(|_| anyhow::anyhow!("QUIC authentication failed"))?;
        let tun = tun_name.as_deref().map(Tun::open).transpose()?;
        Ok::<_, anyhow::Error>((tun, send))
    })
    .await??;
    send.write_all(&[1]).await?;
    send.finish()?;
    match tun {
        Some(tun) => pump(&conn, tun).await,
        None => {
            let mut reader = DatagramReader::new();
            let mut sequence = 0;
            loop {
                let packet = reader.read(&conn).await?;
                send_packet(&conn, &mut sequence, &packet).await?;
            }
        }
    }
}

async fn server(
    listen: SocketAddr,
    cert: PathBuf,
    key: PathBuf,
    secret_file: PathBuf,
    tun_name: Option<String>,
) -> Result<()> {
    let mut tls = server_config(&cert, &key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    config.transport_config(transport());
    let endpoint = Endpoint::server(config, listen)?;
    let key = Arc::new(read_key(&secret_file)?);
    let permits = Arc::new(Semaphore::new(64));
    println!("QUIC listening on {}", endpoint.local_addr()?);
    while let Some(incoming) = endpoint.accept().await {
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let key = key.clone();
        let tun_name = tun_name.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(Ok(conn)) = timeout(OPERATION_TIMEOUT, incoming).await {
                let _ = handle(conn, key, tun_name).await;
            }
        });
    }
    Ok(())
}

fn client(address: SocketAddr, ca: Option<&std::path::Path>) -> Result<Endpoint> {
    let bind = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut tls = client_config(ca)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    config.transport_config(transport());
    let mut endpoint = Endpoint::client(bind.parse()?)?;
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

pub async fn run(args: Args) -> Result<()> {
    match args.action {
        Action::Server {
            listen,
            cert,
            key,
            secret_file,
            tun_name,
        } => server(listen, cert, key, secret_file, tun_name).await,
        Action::Client {
            address,
            server_name,
            ca,
            secret_file,
            tun_name,
            fallback_tcp_port,
        } => {
            let key = read_key(&secret_file)?;
            let tun = Tun::open(&tun_name)?;
            let endpoint = client(address, ca.as_deref())?;
            let handshake = endpoint.connect(address, &server_name)?;
            match timeout(OPERATION_TIMEOUT, handshake).await {
                Ok(Ok(conn)) => {
                    authenticate(&conn, &key).await?;
                    println!("QUIC DATAGRAM session established");
                    pump(&conn, tun).await
                }
                Err(_) | Ok(Err(quinn::ConnectionError::TimedOut)) => {
                    let Some(port) = fallback_tcp_port else {
                        bail!("UDP handshake timed out")
                    };
                    let host = address.ip().to_string();
                    let connector = client_tls(ca.as_deref())?;
                    let stream =
                        crate::session::connect(&host, port, &server_name, connector, &key, crate::profile::DEFAULT_MARKER).await?;
                    println!("UDP handshake timed out; using TCP v1");
                    relay(stream, tun).await
                }
                Ok(Err(error)) => Err(error.into()),
            }
        }
        Action::Probe {
            address,
            server_name,
            ca,
            secret_file,
        } => {
            let key = read_key(&secret_file)?;
            let endpoint = client(address, ca.as_deref())?;
            let conn =
                timeout(OPERATION_TIMEOUT, endpoint.connect(address, &server_name)?).await??;
            authenticate(&conn, &key).await?;
            let mut reader = DatagramReader::new();
            let mut sample = vec![0; MTU];
            sample[0] = 0x60;
            sample[4..6].copy_from_slice(&((MTU - 40) as u16).to_be_bytes());
            let mut sequence = 0;
            send_packet(&conn, &mut sequence, &sample).await?;
            ensure!(
                timeout(OPERATION_TIMEOUT, reader.read(&conn)).await?? == sample,
                "QUIC echo mismatch"
            );
            println!(
                "QUIC DATAGRAM echo: OK; RTT {:.1} ms",
                conn.rtt().as_secs_f64() * 1000.0
            );
            conn.close(0_u32.into(), b"done");
            endpoint.wait_idle().await;
            Ok(())
        }
        Action::ProbeV3 {
            address,
            server_name,
            ca,
            secret_file,
        } => probe_v3(address, &server_name, ca.as_deref(), &secret_file).await,
    }
}

fn checksum(bytes: &[u8]) -> [u8; 2] {
    let mut sum = 0u32;
    for chunk in bytes.chunks(2) {
        sum += u32::from(chunk[0]) << 8
            | u32::from(*chunk.get(1).unwrap_or(&0));
    }
    while sum >> 16 != 0 { sum = (sum & 0xffff) + (sum >> 16); }
    (!(sum as u16)).to_be_bytes()
}

/// QUIC v3 integration probe. Used by the Linux network namespace smoke
/// test to verify the full transport, not merely that UDP can bind.
async fn probe_v3(
    address: SocketAddr,
    server_name: &str,
    ca: Option<&std::path::Path>,
    secret_file: &std::path::Path,
) -> Result<()> {
    let key = read_key(secret_file)?;
    let mut tls = client_config(ca)?;
    tls.alpn_protocols = vec![ALPN_V3.to_vec()];
    let mut config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    config.transport_config(transport());
    let bind = if address.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let mut endpoint = Endpoint::client(bind.parse()?)?;
    endpoint.set_default_client_config(config);
    let conn = timeout(OPERATION_TIMEOUT, endpoint.connect(address, server_name)?).await??;
    let mut material = [0u8; 32];
    conn.export_keying_material(&mut material, AUTH_LABEL_V3, ALPN_V3)
        .map_err(|_| anyhow::anyhow!("QUIC v3 exporter unavailable"))?;
    let proof = hmac::sign(&key, &material);
    let (mut writer, mut reader) = timeout(OPERATION_TIMEOUT, conn.open_bi()).await??;
    writer.write_all(proof.as_ref()).await?;
    writer.finish()?;
    let mut response = [0u8; 5];
    timeout(OPERATION_TIMEOUT, reader.read_exact(&mut response)).await??;
    ensure!(response[0] == 1, "QUIC v3 authentication rejected");
    let assigned = Ipv4Addr::from([response[1], response[2], response[3], response[4]]);
    ensure!(response[1] == 10 && response[2] == 253,
        "QUIC v3 returned unexpected assigned IP");

    let mut packet = [0u8; 28];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(28u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 1;
    packet[12..16].copy_from_slice(&assigned.octets());
    packet[16..20].copy_from_slice(&[10, 253, 0, 1]);
    packet[20] = 8;
    packet[24..26].copy_from_slice(&25u16.to_be_bytes());
    packet[26..28].copy_from_slice(&1u16.to_be_bytes());
    let icmp_sum = checksum(&packet[20..]);
    packet[22..24].copy_from_slice(&icmp_sum);
    let ip_sum = checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&ip_sum);

    let mut seq = 0;
    send_packet(&conn, &mut seq, &packet).await?;
    let mut datagrams = DatagramReader::new();
    let reply = timeout(OPERATION_TIMEOUT, datagrams.read(&conn)).await??;
    ensure!(reply.len() == 28 && reply[16..20] == assigned.octets()
        && reply[20] == 0, "QUIC v3 IPv4 echo reply mismatch");
    println!("QUIC v3 authenticated echo OK, assigned IP: {assigned}");
    conn.close(0u32.into(), b"done");
    endpoint.wait_idle().await;
    Ok(())
}

/// Parallel QUIC v3 listener for the ordinary TCP server.
///
/// v2 experimental standalone QUIC is intentionally unchanged. v3 shares the
/// TCP server's key ring and address allocator, so a profile can be used on
/// both transports without colliding peer TUN interfaces.
pub fn start_shared(
    host: &str,
    port: u16,
    cert: &std::path::Path,
    key: &std::path::Path,
    shared: Arc<crate::session::Server>,
) -> Result<tokio::task::JoinHandle<()>> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .context("UDP listen address not resolved")?;
    let mut tls = server_config(cert, key)?;
    tls.alpn_protocols = vec![ALPN_V3.to_vec()];
    let mut config = quinn::ServerConfig::with_crypto(
        Arc::new(QuicServerConfig::try_from(tls)?),
    );
    config.transport_config(transport());
    // Validate binding at startup so a requested UDP service cannot silently
    // fail while an operator assumes it is available.
    let endpoint = Endpoint::server(config, addr)?;
    println!("SSLim QUIC v3 listening on UDP {}", endpoint.local_addr()?);
    let permits = Arc::new(Semaphore::new(128));
    Ok(tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                incoming.refuse();
                continue;
            };
            let server = shared.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match timeout(OPERATION_TIMEOUT, incoming).await {
                    Ok(Ok(conn)) => {
                        if let Err(error) = shared_session(conn, server).await {
                            eprintln!("QUIC v3 session error: {error:#}");
                        }
                    }
                    Ok(Err(error)) => eprintln!("QUIC v3 handshake error: {error}"),
                    Err(_) => {}
                }
            });
        }
    }))
}

async fn shared_session(
    conn: Connection,
    server: Arc<crate::session::Server>,
) -> Result<()> {
    let (mut response, mut proof_stream) = timeout(OPERATION_TIMEOUT, conn.accept_bi()).await??;
    let mut proof = [0u8; 32];
    timeout(OPERATION_TIMEOUT, proof_stream.read_exact(&mut proof)).await??;
    let mut material = [0u8; 32];
    conn.export_keying_material(&mut material, AUTH_LABEL_V3, ALPN_V3)
        .map_err(|_| anyhow::anyhow!("QUIC v3 TLS exporter unavailable"))?;
    let ring = server
        .secrets_file
        .as_deref()
        .map(crate::auth::KeyRing::from_file)
        .transpose()?;
    let ring = ring.as_ref().unwrap_or(&server.ring);
    ensure!(ring.verify_quic_exporter(&material, &proof),
        "QUIC v3 authentication denied");
    let (address, tun) = server.allocate_peer_tun().await?;
    let mut ack = [0u8; 5];
    ack[0] = 1;
    ack[1..].copy_from_slice(&address.octets());
    response.write_all(&ack).await?;
    response.finish()?;
    pump_addressed(&conn, tun, address).await
}

async fn pump_addressed(conn: &Connection, tun: Tun, address: Ipv4Addr) -> Result<()> {
    let ip = address.octets();
    let client_to_server = async {
        let mut packets = DatagramReader::new();
        loop {
            let packet = packets.read(conn).await?;
            // Restrict each peer to its dynamically allocated source IP.
            ensure!(packet.len() >= 20 && (packet[0] >> 4) == 4,
                "QUIC v3 supports IPv4 peer TUN packets only");
            if packet[12..16] == ip {
                tun.write(&packet).await?;
            }
        }
    };
    let server_to_client = async {
        let mut seq = 0u32;
        let mut buffer = [0u8; MTU + 1];
        loop {
            let size = tun.read(&mut buffer).await?;
            if size <= MTU && size >= 20
                && (buffer[0] >> 4) == 4
                && buffer[16..20] == ip && valid_ip(&buffer[..size])
            {
                send_packet(conn, &mut seq, &buffer[..size]).await?;
            }
        }
    };
    tokio::select! {
        result = client_to_server => result,
        result = server_to_client => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv6() -> Vec<u8> {
        let mut packet = vec![0; MTU];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&((MTU - 40) as u16).to_be_bytes());
        packet
    }

    #[test]
    fn reassembles_out_of_order_and_other_packet_is_independent() {
        let sample = ipv6();
        let mut reader = DatagramReader::new();
        assert!(reader.packet(encode(1, 2, &sample[FRAGMENT..])).is_none());
        assert_eq!(reader.packet(encode(2, 0, &sample)).unwrap(), sample);
        assert_eq!(
            reader.packet(encode(1, 1, &sample[..FRAGMENT])).unwrap(),
            sample
        );
        assert!(reader.packet(encode(3, 1, &[0; 16])).is_none());
    }
}
