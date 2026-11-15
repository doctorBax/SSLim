mod auth;
mod profile;
mod protocol;
#[cfg(target_os = "linux")]
mod quic;
mod session;
#[cfg(target_os = "linux")]
mod tun;
#[cfg(windows)]
mod windows_client;

#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, ensure};
#[cfg(target_os = "linux")]
use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tokio::io::AsyncWriteExt;
#[cfg(target_os = "linux")]
use tokio::net::TcpListener;
#[cfg(target_os = "linux")]
use tokio::sync::Semaphore;
use tokio::time::timeout;
#[cfg(target_os = "linux")]
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;
#[cfg(target_os = "linux")]
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, version};

use crate::auth::{generate_secret, read_key, write_secret_file};
#[cfg(target_os = "linux")]
use crate::auth::KeyRing;
use crate::profile::Profile;
use crate::protocol::{CLOSE, FrameReader, PACKET, append_packet};
use crate::session::connect;
#[cfg(target_os = "linux")]
use crate::session::{Device, Server, relay};
#[cfg(target_os = "linux")]
use crate::tun::Tun;

#[derive(Parser)]
#[command(version, about = "SSLim TLS packet transport")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Endpoint {
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = profile::DEFAULT_PORT)]
    port: u16,
    #[arg(long)]
    server_name: String,
    #[arg(long)]
    ca: Option<PathBuf>,
    #[arg(long)]
    secret_file: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    #[cfg(windows)]
    /// Blade — the configurable Windows VPN client
    Blade(windows_client::BladeArgs),
    #[cfg(target_os = "linux")]
    /// Experimental QUIC DATAGRAM transport
    Quic(quic::Args),
    #[cfg(target_os = "linux")]
    Server {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = profile::DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        key: PathBuf,
        /// One shared secret file (32+ random bytes)
        #[arg(long, required_unless_present = "secrets_file")]
        secret_file: Option<PathBuf>,
        /// Per-client secrets: lines of `label:base64`
        #[arg(long, conflicts_with = "secret_file")]
        secrets_file: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "echo")]
        device: Device,
        #[arg(long, default_value = "labtun0")]
        tun_name: String,
        /// Linux TCP congestion control algorithm for connections accepted by this listener
        #[arg(long)]
        congestion_control: Option<String>,
    },
    #[cfg(target_os = "linux")]
    Client {
        #[command(flatten)]
        endpoint: Endpoint,
        #[arg(long, default_value = "labtun0")]
        tun_name: String,
    },
    /// Exchange one IPv4 packet with a server in echo mode
    Probe {
        #[command(flatten)]
        endpoint: Endpoint,
    },
    /// Generate a 32-byte secret file for a client or the server
    Keygen {
        #[arg(long, default_value = "session.bin")]
        output: PathBuf,
    },
    /// Print the sslim:// URI and QR code a client should import
    Profile {
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = profile::DEFAULT_PORT)]
        port: u16,
        /// TLS server name announced by the client (defaults to the host)
        #[arg(long)]
        server_name: Option<String>,
        #[arg(long)]
        secret_file: PathBuf,
        /// Profile label shown to the client (defaults to the host)
        #[arg(long)]
        label: Option<String>,
    },
    #[cfg(target_os = "linux")]
    /// Check the local TUN device without setting addresses or routes
    TunCheck {
        #[arg(long, default_value = "labcheck0")]
        tun_name: String,
    },
}

fn render_qr(text: &str) -> Result<String> {
    use qrcode::Color;
    use qrcode::types::EcLevel;
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)?;
    let width = code.width();
    let mut out = String::new();
    for y in (0..width).step_by(2) {
        for x in 0..width {
            let top = code[(x, y)] == Color::Dark;
            let bottom = if y + 1 < width {
                code[(x, y + 1)] == Color::Dark
            } else {
                false
            };
            out.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn server_config(cert: &Path, key: &Path) -> Result<ServerConfig> {
    let certs = rustls_pemfile::certs(&mut BufReader::new(File::open(cert)?))
        .collect::<std::io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(File::open(key)?))?
        .context("private key not found in PEM file")?;
    let mut config =
        ServerConfig::builder_with_protocol_versions(&[&version::TLS13, &version::TLS12])
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

#[cfg(target_os = "linux")]
fn server_tls(cert: &Path, key: &Path) -> Result<TlsAcceptor> {
    Ok(TlsAcceptor::from(Arc::new(server_config(cert, key)?)))
}

fn client_config(ca: Option<&Path>) -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    match ca {
        Some(path) => {
            for cert in rustls_pemfile::certs(&mut BufReader::new(File::open(path)?)) {
                roots.add(cert?)?;
            }
        }
        None => {
            let native = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(native.certs);
            ensure!(
                !roots.is_empty(),
                "no usable system certificates: {:?}",
                native.errors
            );
        }
    }
    ensure!(!roots.is_empty(), "no certificates loaded");
    let mut config = ClientConfig::builder_with_protocol_versions(&[&version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

fn client_tls(ca: Option<&Path>) -> Result<TlsConnector> {
    Ok(TlsConnector::from(Arc::new(client_config(ca)?)))
}

#[cfg(target_os = "linux")]
fn set_congestion_control(listener: &TcpListener, name: &str) -> Result<()> {
    let algorithm = CString::new(name)?;
    // SAFETY: the listener owns the socket and the C string stays alive for the syscall.
    let result = unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_CONGESTION,
            algorithm.as_ptr().cast(),
            algorithm.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("setting TCP congestion control to {name}"));
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    match Cli::parse().command {
        #[cfg(windows)]
        Command::Blade(args) => windows_client::run(args).await,
        #[cfg(target_os = "linux")]
        Command::Quic(args) => quic::run(args).await,
        #[cfg(target_os = "linux")]
        Command::Server {
            host,
            port,
            cert,
            key,
            secret_file,
            secrets_file,
            device,
            tun_name,
            congestion_control,
        } => {
            let ring = match &secrets_file {
                Some(list) => KeyRing::from_file(list)?,
                None => KeyRing::single(
                    secret_file.as_deref().expect("clap guarantees one source"),
                )?,
            };
            println!(
                "{} accepted secret(s): {}",
                ring.len(),
                if secrets_file.is_some() { "per-client" } else { "single" }
            );
            let server = Arc::new(Server {
                acceptor: server_tls(&cert, &key)?,
                ring,
                replay: Mutex::new(HashMap::new()),
                device,
                tun_name,
            });
            let listener = TcpListener::bind((host.as_str(), port)).await?;
            if let Some(algorithm) = congestion_control {
                set_congestion_control(&listener, &algorithm)?;
            }
            let permits = Arc::new(Semaphore::new(256));
            println!("listening on {} ({device:?})", listener.local_addr()?);
            loop {
                let (tcp, _) = listener.accept().await?;
                if let Ok(permit) = permits.clone().try_acquire_owned() {
                    let server = server.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ = server.handle(tcp).await;
                    });
                }
            }
        }
        #[cfg(target_os = "linux")]
        Command::Client { endpoint, tun_name } => {
            let key = read_key(&endpoint.secret_file)?;
            let tls = client_tls(endpoint.ca.as_deref())?;
            let tun = Tun::open(&tun_name)?;
            let stream = connect(
                &endpoint.host,
                endpoint.port,
                &endpoint.server_name,
                tls,
                &key,
            )
            .await?;
            relay(stream, tun).await
        }
        Command::Probe { endpoint } => {
            let mut stream = connect(
                &endpoint.host,
                endpoint.port,
                &endpoint.server_name,
                client_tls(endpoint.ca.as_deref())?,
                &read_key(&endpoint.secret_file)?,
            )
            .await?;
            let sample = [
                0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1,
            ];
            let mut frame = Vec::new();
            append_packet(&mut frame, &sample);
            timeout(Duration::from_secs(5), async {
                stream.write_all(&frame).await?;
                stream.flush().await?;
                let mut reader = FrameReader::new(&mut stream);
                let (kind, packet) = reader.next().await?;
                ensure!(kind == PACKET && packet == sample, "echo packet mismatch");
                stream.write_all(&[CLOSE, 0, 0]).await?;
                stream.shutdown().await?;
                Ok::<_, anyhow::Error>(())
            })
            .await??;
            println!("TLS echo: OK");
            Ok(())
        }
        Command::Keygen { output } => {
            let bytes = generate_secret()?;
            write_secret_file(&output, &bytes)?;
            println!(
                "wrote {} random bytes to {}; give it to one client and keep a copy for the server",
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Command::Profile {
            host,
            port,
            server_name,
            secret_file,
            label,
        } => {
            let profile = Profile {
                name: label.unwrap_or_else(|| host.clone()),
                host: host.clone(),
                port,
                server_name: server_name.unwrap_or(host),
                secret_base64: String::new(),
                secret_file: Some(secret_file.clone()),
                ca_file: None,
                stats: false,
            };
            let uri = profile.to_uri()?;
            println!("{uri}");
            println!();
            print!("{}", render_qr(&uri)?);
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::TunCheck { tun_name } => {
            let _tun = Tun::open(&tun_name)?;
            println!("Linux TUN create/close: OK");
            Ok(())
        }
    }
}
