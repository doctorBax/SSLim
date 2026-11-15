use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use clap::ValueEnum;
use ring::hmac;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_rustls::rustls::{ProtocolVersion, pki_types::ServerName};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::auth::{ReplayGuard, credentials};
use crate::protocol::{
    CLOSE, FrameReader, MAX_PACKET, PACKET, PING, PONG_FRAME, append_packet, icmp_reply,
};
use crate::tun::Tun;

const UPGRADE: &[u8] =
    b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: lab-packet/1\r\n\r\n";
const PUBLIC_PAGE: &[u8] = b"<!doctype html><title>Service</title><p>Service available.</p>";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

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

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    version: &'a str,
    headers: HashMap<String, &'a str>,
}

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
            && self.get("upgrade").eq_ignore_ascii_case("lab-packet/1")
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
) -> Result<BufReader<tokio_rustls::client::TlsStream<TcpStream>>> {
    let name = ServerName::try_from(server_name.to_owned())?;
    let tcp = timeout(Duration::from_secs(10), TcpStream::connect((host, port))).await??;
    tcp.set_nodelay(true)?;
    let mut stream = BufReader::with_capacity(
        32 * 1024,
        timeout(IO_TIMEOUT, connector.connect(name, tcp)).await??,
    );
    let (time, nonce, tag) = credentials(key)?;
    let request = format!(
        "POST /connect HTTP/1.1\r\nHost: {server_name}\r\nConnection: Upgrade\r\nUpgrade: lab-packet/1\r\nX-Session-Time: {time}\r\nX-Session-Nonce: {nonce}\r\nX-Session-Tag: {tag}\r\n\r\n"
    );
    timeout(IO_TIMEOUT, send(&mut stream, request.as_bytes())).await??;
    let response = timeout(IO_TIMEOUT, headers(&mut stream)).await??;
    ensure!(
        response.starts_with(b"HTTP/1.1 101 Switching Protocols\r\n"),
        "server rejected the session"
    );
    Ok(stream)
}

pub struct Server {
    pub acceptor: TlsAcceptor,
    pub key: hmac::Key,
    pub replay: Mutex<ReplayGuard>,
    pub device: Device,
    pub tun_name: String,
}

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
        let authorized = tls13
            && request.is_upgrade()
            && self.replay.lock().expect("replay lock poisoned").verify(
                &self.key,
                request.get("x-session-time"),
                request.get("x-session-nonce"),
                request.get("x-session-tag"),
            );
        if !authorized {
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
        let tun = match self.device {
            Device::Tun => Some(Tun::open(&self.tun_name)?),
            _ => None,
        };
        timeout(IO_TIMEOUT, send(&mut stream, UPGRADE)).await??;
        match tun {
            Some(tun) => relay(stream, tun).await,
            None => echo(stream, self.device).await,
        }
    }
}

pub async fn relay<S: AsyncRead + AsyncWrite + Unpin>(stream: S, tun: Tun) -> Result<()> {
    let (reader, mut writer) = io::split(stream);
    let mut reader = FrameReader::new(reader);
    let (pongs, mut control) = mpsc::channel::<()>(16);
    let from_tls = async {
        loop {
            match reader.next().await? {
                (PACKET, packet) => tun.write(packet).await?,
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
                result = tun.batch(&mut packet, &mut frames) => {
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
