mod auth;
mod protocol;
mod session;
mod tun;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig, version};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::auth::read_key;
use crate::protocol::{CLOSE, FrameReader, PACKET, append_packet};
use crate::session::{Device, Server, connect, relay};
use crate::tun::Tun;

#[derive(Parser)]
#[command(version, about = "GoVPN Lab Linux TLS packet transport")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Endpoint {
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 9443)]
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
    Server {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 9443)]
        port: u16,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        secret_file: PathBuf,
        #[arg(long, value_enum, default_value = "echo")]
        device: Device,
        #[arg(long, default_value = "labtun0")]
        tun_name: String,
    },
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
    /// Check the local TUN device without setting addresses or routes
    TunCheck {
        #[arg(long, default_value = "labcheck0")]
        tun_name: String,
    },
}

fn server_tls(cert: &Path, key: &Path) -> Result<TlsAcceptor> {
    let certs = rustls_pemfile::certs(&mut BufReader::new(File::open(cert)?))
        .collect::<std::io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(File::open(key)?))?
        .context("private key not found in PEM file")?;
    let mut config =
        ServerConfig::builder_with_protocol_versions(&[&version::TLS13, &version::TLS12])
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn client_tls(ca: Option<&Path>) -> Result<TlsConnector> {
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
    Ok(TlsConnector::from(Arc::new(config)))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Server {
            host,
            port,
            cert,
            key,
            secret_file,
            device,
            tun_name,
        } => {
            let server = Arc::new(Server {
                acceptor: server_tls(&cert, &key)?,
                key: read_key(&secret_file)?,
                replay: Mutex::default(),
                device,
                tun_name,
            });
            let listener = TcpListener::bind((host.as_str(), port)).await?;
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
        Command::TunCheck { tun_name } => {
            let _tun = Tun::open(&tun_name)?;
            println!("Linux TUN create/close: OK");
            Ok(())
        }
    }
}
