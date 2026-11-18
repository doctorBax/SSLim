//! SSLim QUIC v3 Android transport. The legacy Java/TCP engine is untouched.
//! JNI boundary: nativeRun blocks on the existing Blade VPN worker thread;
//! service.protect(fd) is called before any UDP packets are transmitted.
use std::io::{self, Cursor};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use bytes::{BufMut, Bytes, BytesMut};
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jint, jlong};
use jni::JNIEnv;
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{Connection, Endpoint, EndpointConfig, TransportConfig};
use ring::hmac;
use tokio::io::unix::AsyncFd;
use tokio::time::{interval, timeout};

const ALPN: &[u8] = b"sslim/3";
const EXPORTER: &[u8] = b"EXPORTER-SSLim-v3-auth";
const MTU: usize = 1280;
const HEADER: usize = 5;
const FRAGMENT: usize = 1024;
const SLOTS: usize = 512;
static STOP: AtomicBool = AtomicBool::new(false);

struct RawFd(i32);
impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> i32 { self.0 }
}

struct Pending {
    id: u32,
    since: Instant,
    parts: [Option<Bytes>; 2],
}
struct Reader { slots: Vec<Option<Pending>> }
impl Reader {
    fn new() -> Self {
        Self { slots: (0..SLOTS).map(|_| None).collect() }
    }
    fn packet(&mut self, frame: Bytes) -> Option<Bytes> {
        if !(HEADER + 1..=HEADER + MTU).contains(&frame.len()) { return None; }
        let id = u32::from_be_bytes(frame[..4].try_into().ok()?);
        let kind = frame[4];
        let data = frame.slice(HEADER..);
        if kind == 0 { return valid_ipv4(&data).then_some(data); }
        if !matches!((kind, data.len()), (1, FRAGMENT) | (2, 1..=256)) {
            return None;
        }
        let slot = &mut self.slots[id as usize % SLOTS];
        if slot.as_ref().is_none_or(|p| p.id != id
            || p.since.elapsed() >= Duration::from_secs(2))
        {
            *slot = Some(Pending {
                id,
                since: Instant::now(),
                parts: [None, None],
            });
        }
        let pending = slot.as_mut()?;
        pending.parts[usize::from(kind - 1)] = Some(data);
        let [Some(first), Some(last)] = &pending.parts else { return None; };
        let mut result = BytesMut::with_capacity(first.len() + last.len());
        result.extend_from_slice(first);
        result.extend_from_slice(last);
        *slot = None;
        valid_ipv4(&result).then(|| result.freeze())
    }
    async fn read(&mut self, conn: &Connection) -> Result<Bytes> {
        loop {
            let datagram = conn.read_datagram().await?;
            if let Some(packet) = self.packet(datagram) { return Ok(packet); }
        }
    }
}

fn valid_ipv4(data: &[u8]) -> bool {
    if data.len() < 20 || data.len() > MTU || data[0] >> 4 != 4 {
        return false;
    }
    let header = usize::from(data[0] & 15) * 4;
    header >= 20 && header <= data.len()
        && u16::from_be_bytes([data[2], data[3]]) as usize == data.len()
}

fn encode(id: u32, kind: u8, data: &[u8]) -> Bytes {
    let mut packet = BytesMut::with_capacity(HEADER + data.len());
    packet.put_u32(id);
    packet.put_u8(kind);
    packet.extend_from_slice(data);
    packet.freeze()
}

async fn send_packet(conn: &Connection, id: &mut u32, packet: &[u8]) -> Result<()> {
    let max = conn.max_datagram_size().context("server lacks QUIC DATAGRAM support")?;
    ensure!(max >= HEADER + FRAGMENT, "QUIC datagram path too small");
    let sequence = *id;
    *id = id.wrapping_add(1);
    if HEADER + packet.len() <= max {
        conn.send_datagram_wait(encode(sequence, 0, packet)).await?;
    } else {
        conn.send_datagram_wait(encode(sequence, 1, &packet[..FRAGMENT])).await?;
        conn.send_datagram_wait(encode(sequence, 2, &packet[FRAGMENT..])).await?;
    }
    Ok(())
}

fn tls_config(trust: &[u8]) -> Result<quinn::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    let mut count = 0;
    for cert in rustls_pemfile::certs(&mut Cursor::new(trust)) {
        if roots.add(cert?).is_ok() { count += 1; }
    }
    ensure!(count > 0, "No valid Android trust roots or custom CA certificate");
    let mut tls = rustls::ClientConfig::builder_with_provider(
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut client = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    let mut transport = TransportConfig::default();
    transport.datagram_receive_buffer_size(Some(512 * 1024))
        .datagram_send_buffer_size(256 * 1024)
        .keep_alive_interval(Some(Duration::from_secs(5)));
    client.transport_config(Arc::new(transport));
    Ok(client)
}

fn read_fd(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    let result = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if result >= 0 { Ok(result as usize) } else { Err(io::Error::last_os_error()) }
}
fn write_fd(fd: i32, data: &[u8]) -> io::Result<usize> {
    let result = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
    if result >= 0 { Ok(result as usize) } else { Err(io::Error::last_os_error()) }
}

struct SessionConfig {
    host: String,
    port: u16,
    sni: String,
    trust: Vec<u8>,
    secret: Vec<u8>,
}

enum Outcome { Closed, Unavailable }

/// A single unbuffered async UDP socket is protected with Android VpnService.
/// This ensures it cannot recursively route through its own VPN TUN.
async fn session(
    env: &mut JNIEnv<'_>,
    service: &JObject<'_>,
    config: SessionConfig,
) -> Result<Outcome> {
    ensure!(config.secret.len() >= 32, "VPN secret must be at least 32 bytes");
    let addr = (config.host.as_str(), config.port)
        .to_socket_addrs()?.next().context("server address could not be resolved")?;
    let bind = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind)?;
    socket.set_nonblocking(true)?;
    let protected = env.call_method(service, "protect", "(I)Z",
        &[JValue::Int(socket.as_raw_fd())])?.z()?;
    ensure!(protected, "VpnService refused UDP socket protection");
    let mut endpoint = Endpoint::new(
        EndpointConfig::default(), None, socket, Arc::new(quinn::TokioRuntime),
    )?;
    endpoint.set_default_client_config(tls_config(&config.trust)?);
    let connecting = endpoint.connect(addr, &config.sni)?;
    let conn = match timeout(Duration::from_secs(4), connecting).await {
        Err(_) | Ok(Err(quinn::ConnectionError::TimedOut)) =>
            return Ok(Outcome::Unavailable),
        Ok(Err(error)) => bail!("UDP QUIC handshake failed: {error}"),
        Ok(Ok(conn)) => conn,
    };

    let mut material = [0u8; 32];
    conn.export_keying_material(&mut material, EXPORTER, ALPN)
        .map_err(|_| anyhow::anyhow!("QUIC exporter unavailable"))?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &config.secret);
    let proof = hmac::sign(&key, &material);
    let (mut tx, mut rx) = timeout(Duration::from_secs(5), conn.open_bi()).await??;
    tx.write_all(proof.as_ref()).await?;
    tx.finish()?;
    let mut response = [0u8; 5];
    timeout(Duration::from_secs(5), rx.read_exact(&mut response)).await??;
    ensure!(response[0] == 1, "server rejected QUIC v3 authorization");
    ensure!(response[1] == 10 && response[2] == 253,
        "server assigned an unexpected VPN address");
    let client_ip = response[1..5].to_vec();
    let address = format!("{}.{}.{}.{}", response[1], response[2], response[3], response[4]);
    let java_address = env.new_string(address)?;
    let object = JObject::from(java_address);
    let fd = env.call_method(service, "onNativeQuicConnected",
        "(Ljava/lang/String;)I", &[JValue::Object(&object)])?.i()?;
    ensure!(fd >= 0, "Android refused to establish QUIC VPN TUN");

    let tun = AsyncFd::new(RawFd(fd))?;
    let mut sequence = 0u32;
    let mut reader = Reader::new();
    let mut upload: u64 = 0;
    let mut download: u64 = 0;
    let mut buf = [0u8; MTU + 1];
    let mut ticker = interval(Duration::from_millis(300));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if STOP.load(Ordering::SeqCst) {
                    conn.close(0u32.into(), b"closed");
                    return Ok(Outcome::Closed);
                }
                let rtt = conn.rtt().as_millis().min(i64::MAX as u128) as i64;
                env.call_method(service, "onNativeQuicStats", "(JJJ)V",
                    &[JValue::Long(upload as jlong),
                        JValue::Long(download as jlong), JValue::Long(rtt)])?;
            }
            event = tun.readable() => {
                let mut ready = event?;
                match ready.try_io(|fd| read_fd(fd.get_ref().0, &mut buf)) {
                    Ok(Ok(size)) if size == 0 => bail!("Android TUN closed"),
                    Ok(Ok(size)) => {
                        if size <= MTU && valid_ipv4(&buf[..size])
                            && buf[12..16] == client_ip
                        {
                            send_packet(&conn, &mut sequence, &buf[..size]).await?;
                            upload += size as u64;
                        }
                    }
                    Ok(Err(error)) => return Err(error.into()),
                    Err(_) => {}
                }
            }
            packet = reader.read(&conn) => {
                let packet = packet?;
                // The server must send packets exclusively addressed to the
                // client-assigned IPv4; do not forward malformed packets to TUN.
                if packet[16..20] != client_ip { continue; }
                let mut left = packet.as_ref();
                while !left.is_empty() {
                    let mut ready = tun.writable().await?;
                    match ready.try_io(|fd| write_fd(fd.get_ref().0, left)) {
                        Ok(Ok(0)) => bail!("Android TUN write returned zero"),
                        Ok(Ok(n)) => left = &left[n..],
                        Ok(Err(error)) => return Err(error.into()),
                        Err(_) => {}
                    }
                }
                download += packet.len() as u64;
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_ru_doctorbax_blade_BladeQuic_stopNative(
    _env: JNIEnv<'_>, _class: JClass<'_>,
) {
    STOP.store(true, Ordering::SeqCst);
}

#[no_mangle]
pub extern "system" fn Java_ru_doctorbax_blade_BladeQuic_runNative(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    service: JObject<'_>,
    host: JString<'_>,
    port: jint,
    sni: JString<'_>,
    trust: JByteArray<'_>,
    secret: JByteArray<'_>,
) -> jint {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Outcome> {
        ensure!(port > 0 && port <= 65535, "invalid server UDP port");
        let config = SessionConfig {
            host: env.get_string(&host)?.into(),
            port: port as u16,
            sni: env.get_string(&sni)?.into(),
            trust: env.convert_byte_array(&trust)?,
            secret: env.convert_byte_array(&secret)?,
        };
        STOP.store(false, Ordering::SeqCst);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all().build()?;
        runtime.block_on(session(&mut env, &service, config))
    }));
    match outcome {
        Ok(Ok(Outcome::Closed)) => 0,
        Ok(Ok(Outcome::Unavailable)) => 1,
        Ok(Err(error)) => {
            if !env.exception_check().unwrap_or(false) {
                let _ = env.throw_new("java/io/IOException", format!("{error:#}"));
            }
            -1
        }
        Err(_) => {
            let _ = env.throw_new("java/io/IOException",
                "Native QUIC engine panicked; connection stopped");
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmentation_and_validation() {
        let mut ipv4 = vec![0u8; 1280];
        ipv4[0] = 0x45;
        ipv4[2..4].copy_from_slice(&(1280u16).to_be_bytes());
        assert!(valid_ipv4(&ipv4));
        let mut reader = Reader::new();
        assert!(reader.packet(encode(42, 2, &ipv4[1024..])).is_none());
        assert_eq!(reader.packet(encode(42, 1, &ipv4[..1024])).unwrap(), ipv4);
        assert!(!valid_ipv4(&ipv4[..512]));
    }
}
