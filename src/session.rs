#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::net::Ipv4Addr;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail, ensure};
#[cfg(target_os = "linux")]
use clap::ValueEnum;
use ring::hmac;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use tokio::net::TcpStream;
#[cfg(target_os = "linux")]
use tokio::sync::mpsc;
use tokio::time::timeout;
#[cfg(target_os = "linux")]
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;
#[cfg(target_os = "linux")]
use tokio_rustls::rustls::ProtocolVersion;
use tokio_rustls::rustls::pki_types::ServerName;

use crate::auth::credentials;
#[cfg(target_os = "linux")]
use crate::auth::{KeyRing, ReplayGuard};
#[cfg(target_os = "linux")]
use crate::protocol::{
    CLOSE, FrameReader, MAX_PACKET, PACKET, PING, PONG_FRAME, append_packet, icmp_reply,
};
#[cfg(target_os = "linux")]
use crate::tun::Tun;

#[cfg(target_os = "linux")]
const UPGRADE: &[u8] =
    b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: sync/1\r\n\r\n";
#[cfg(target_os = "linux")]
const PUBLIC_PAGE: &[u8] = b"<!doctype html><title>Service</title><p>Service available.</p>";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Device {
    Echo,
    Icmp,
    Tun,
}

async fn headers(input: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(512);
    while bytes.len() < 8192 {
        bytes.push(input.read_u8().await?);
        if bytes.ends_with(b"\r\n\r\n") {
            ensure!(bytes.is_ascii(), "headers must be ASCII");
            return Ok(bytes);
        }
    }
    bail!("headers too large");
}

#[cfg(target_os = "linux")]
struct Request<'a> {
    method: &'a str,
    path: &'a str,
    version: &'a str,
    headers: HashMap<String, &'a str>,
}

#[cfg(target_os = "linux")]
impl<'a> Request<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut lines = std::str::from_utf8(bytes)?.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let (Some(method), Some(path), Some(version), None) =
            (first.next(), first.next(), first.next(), first.next())
        else {
            bail!("invalid request line");
        };
        ensure!(
            matches!(version, "HTTP/1.0" | "HTTP/1.1"),
            "unsupported HTTP version"
        );
        let mut headers = HashMap::new();
        for line in lines.take_while(|line| !line.is_empty()) {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("invalid header"))?;
            ensure!(!name.is_empty(), "empty header name");
            ensure!(
                headers
                    .insert(name.to_ascii_lowercase(), value.trim())
                    .is_none(),
                "duplicate header"
            );
        }
        Ok(Self {
            method,
            path,
            version,
            headers,
        })
    }

    fn get(&self, name: &str) -> &str {
        self.headers.get(name).copied().unwrap_or_default()
    }

    fn is_upgrade(&self) -> bool {
        self.method == "POST"
            && self.path == "/connect"
            && self.version == "HTTP/1.1"
            && self.get("upgrade").eq_ignore_ascii_case("sync/1")
            && self
                .get("connection")
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
    }
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), data: &[u8]) -> io::Result<()> {
    writer.write_all(data).await?;
    writer.flush().await
}

#[cfg(target_os = "linux")]
async fn answer(
    stream: &mut (impl AsyncWrite + Unpin),
    status: &str,
    body: &[u8],
    head: bool,
) -> Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ).into_bytes();
    if !head {
        response.extend_from_slice(body);
    }
    timeout(IO_TIMEOUT, send(stream, &response)).await??;
    let _ = timeout(IO_TIMEOUT, stream.shutdown()).await;
    Ok(())
}

pub async fn connect(
    host: &str,
    port: u16,
    server_name: &str,
    connector: TlsConnector,
    key: &hmac::Key,
    marker: &str,
) -> Result<BufReader<tokio_rustls::client::TlsStream<TcpStream>>> {
    Ok(
        connect_inner(host, port, server_name, connector, key, marker, false)
            .await?
            .0,
    )
}

#[cfg(windows)]
pub async fn connect_with_address(
    host: &str,
    port: u16,
    server_name: &str,
    connector: TlsConnector,
    key: &hmac::Key,
    marker: &str,
) -> Result<(
    BufReader<tokio_rustls::client::TlsStream<TcpStream>>,
    Ipv4Addr,
)> {
    connect_inner(host, port, server_name, connector, key, marker, true).await
}

async fn connect_inner(
    host: &str,
    port: u16,
    server_name: &str,
    connector: TlsConnector,
    key: &hmac::Key,
    marker: &str,
    dynamic_address: bool,
) -> Result<(
    BufReader<tokio_rustls::client::TlsStream<TcpStream>>,
    Ipv4Addr,
)> {
    let name = ServerName::try_from(server_name.to_owned())?;
    let tcp = timeout(Duration::from_secs(10), TcpStream::connect((host, port))).await??;
    tcp.set_nodelay(true)?;
    let mut stream = BufReader::with_capacity(
        32 * 1024,
        timeout(IO_TIMEOUT, connector.connect(name, tcp)).await??,
    );
    let (time, nonce, tag) = credentials(key)?;
    let request = format!(
        "POST /connect HTTP/1.1\r\nHost: {server_name}\r\nConnection: Upgrade\r\nUpgrade: {marker}\r\nX-Session-Time: {time}\r\nX-Session-Nonce: {nonce}\r\nX-Session-Tag: {tag}\r\n{}\r\n",
        if dynamic_address {
            "X-Session-Address: auto\r\n"
        } else {
            ""
        }
    );
    timeout(IO_TIMEOUT, send(&mut stream, request.as_bytes())).await??;
    let response = timeout(IO_TIMEOUT, headers(&mut stream)).await??;
    ensure!(
        response.starts_with(b"HTTP/1.1 101 Switching Protocols\r\n"),
        "server rejected the session"
    );
    let address = std::str::from_utf8(&response)?
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Session-Address"))
        .map(|(_, value)| value.trim().parse::<Ipv4Addr>())
        .transpose()?
        .unwrap_or(Ipv4Addr::new(10, 253, 0, 2));
    Ok((stream, address))
}

#[cfg(target_os = "linux")]
pub struct Server {
    pub acceptor: TlsAcceptor,
    pub ring: KeyRing,
    pub secrets_file: Option<PathBuf>,
    pub replay: Mutex<HashMap<String, ReplayGuard>>,
    pub device: Device,
    pub tun_name: String,
    pub multi_tun: bool,
    pub next_address: std::sync::atomic::AtomicU32,
}

#[cfg(target_os = "linux")]
impl Server {
    pub async fn handle(self: Arc<Self>, tcp: TcpStream) -> Result<()> {
        tcp.set_nodelay(true)?;
        let tls = timeout(IO_TIMEOUT, self.acceptor.accept(tcp)).await??;
        let tls13 = tls.get_ref().1.protocol_version() == Some(ProtocolVersion::TLSv1_3);
        let mut stream = BufReader::with_capacity(32 * 1024, tls);
        let raw = timeout(IO_TIMEOUT, headers(&mut stream)).await??;
        let Ok(request) = Request::parse(&raw) else {
            return answer(&mut stream, "400 Bad Request", b"Bad request", false).await;
        };
        let verified = if tls13 && request.is_upgrade() {
            let loaded_ring = self
                .secrets_file
                .as_deref()
                .map(KeyRing::from_file)
                .transpose()?;
            loaded_ring
                .as_ref()
                .unwrap_or(&self.ring)
                .verify(
                    request.get("x-session-time"),
                    request.get("x-session-nonce"),
                    request.get("x-session-tag"),
                )
                .is_some_and(|verified| {
                    let mut guards = self.replay.lock().expect("replay lock poisoned");
                    guards
                        .entry(verified.label.to_owned())
                        .or_default()
                        .record(verified.nonce)
                })
        } else {
            false
        };
        if !verified {
            return if matches!(request.method, "GET" | "HEAD") && request.path == "/" {
                answer(&mut stream, "200 OK", PUBLIC_PAGE, request.method == "HEAD").await
            } else {
                answer(
                    &mut stream,
                    "404 Not Found",
                    b"Not found",
                    request.method == "HEAD",
                )
                .await
            };
        }
        if matches!(self.device, Device::Tun)
            && self.multi_tun
            && request.get("x-session-address") == "auto"
        {
            let id = self
                .next_address
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if id > u16::MAX as u32 - 1 {
                return answer(
                    &mut stream,
                    "503 Service Unavailable",
                    b"Address pool exhausted",
                    false,
                )
                .await;
            }
            let address = Ipv4Addr::from(0x0afd_0000 | id);
            let name = format!("sslimd{id:04x}");
            let tun = Tun::open(&name)?;
            let peer = format!("{address}/32");
            for args in [
                [
                    "address",
                    "replace",
                    "10.253.0.1",
                    "peer",
                    &peer,
                    "dev",
                    &name,
                ]
                .as_slice(),
                ["link", "set", "dev", &name, "mtu", "1280", "up"].as_slice(),
            ] {
                let status = tokio::process::Command::new("ip")
                    .args(args)
                    .status()
                    .await?;
                ensure!(status.success(), "failed to configure tunnel {name}");
            }
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: sync/1\r\nX-Session-Address: {address}\r\n\r\n"
            );
            timeout(IO_TIMEOUT, send(&mut stream, response.as_bytes())).await??;
            return relay_checked(stream, tun, Some(address)).await;
        }
        let tun = match self.device {
            Device::Tun => Some(Tun::open(&self.tun_name)?),
            _ => None,
        };
        timeout(IO_TIMEOUT, send(&mut stream, UPGRADE)).await??;
        match tun {
            Some(tun) if self.multi_tun => {
                relay_checked(stream, tun, Some(Ipv4Addr::new(10, 253, 0, 2))).await
            }
            Some(tun) => relay(stream, tun).await,
            None => echo(stream, self.device).await,
        }
    }
}

#[cfg(target_os = "linux")]
pub async fn relay<S: AsyncRead + AsyncWrite + Unpin>(stream: S, tun: Tun) -> Result<()> {
    relay_checked(stream, tun, None).await
}

#[cfg(target_os = "linux")]
async fn relay_checked<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    tun: Tun,
    source: Option<Ipv4Addr>,
) -> Result<()> {
    let (reader, mut writer) = io::split(stream);
    let mut reader = FrameReader::new(reader);
    let (pongs, mut control) = mpsc::channel::<()>(16);
    let from_tls = async {
        loop {
            match reader.next().await? {
                (PACKET, packet)
                    if source.is_none_or(|ip| {
                        packet.len() >= 20 && packet[0] >> 4 == 4 && packet[12..16] == ip.octets()
                    }) =>
                {
                    tun.write(packet).await?
                }
                (PING, _) => {
                    pongs.send(()).await?;
                }
                (CLOSE, _) => return Ok(()),
                _ => {}
            }
        }
    };
    let to_tls = async {
        let mut packet = vec![0; MAX_PACKET];
        let mut frames = Vec::with_capacity(64 * 1024);
        loop {
            tokio::select! {
                biased;
                Some(()) = control.recv() => send(&mut writer, PONG_FRAME).await?,
                result = tun.batch(&mut packet, &mut frames, source) => {
                    result?;
                    send(&mut writer, &frames).await?;
                    frames.clear();
                }
            }
        }
    };
    tokio::select! {
        result = from_tls => result,
        result = to_tls => result,
    }
}

#[cfg(target_os = "linux")]
async fn echo<S: AsyncRead + AsyncWrite + Unpin>(stream: S, device: Device) -> Result<()> {
    let (read, mut write) = io::split(stream);
    let mut reader = FrameReader::new(read);
    let mut frame = Vec::with_capacity(1536);
    loop {
        match reader.next().await? {
            (PACKET, packet) => {
                if !matches!(device, Device::Echo) && !icmp_reply(packet) {
                    continue;
                }
                append_packet(&mut frame, packet);
                send(&mut write, &frame).await?;
                frame.clear();
            }
            (PING, _) => send(&mut write, PONG_FRAME).await?,
            (CLOSE, _) => {
                let _ = timeout(IO_TIMEOUT, write.shutdown()).await;
                return Ok(());
            }
            _ => {}
        }
    }
}
