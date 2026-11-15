use std::net::{Ipv4Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use ring::hmac;
use tokio::io::{self, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, timeout};
use wintun::{Adapter, Session};

use crate::auth::key_from_bytes;
use crate::client_tls;
use crate::profile::{self, Profile};
use crate::protocol::{CLOSE, FrameReader, PACKET, PING, PONG_FRAME, append_packet, valid_ip};
use crate::session::connect;

const KEEPALIVE: Duration = Duration::from_secs(20);
const READ_IDLE: Duration = Duration::from_secs(90);
const RECONNECT_MAX_SECS: u64 = 30;

const OK_MARK: &str = "\x1b[32mOK\x1b[0m";
const WARN_MARK: &str = "\x1b[33m!\x1b[0m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

#[derive(clap::Args)]
pub struct BladeArgs {
    #[command(subcommand)]
    pub command: Option<BladeCommand>,
    #[command(flatten)]
    pub connect: ConnectArgs,
}

#[derive(Subcommand)]
pub enum BladeCommand {
    /// Import a profile from an sslim:// URI or a profile .toml file
    Import {
        source: String,
        /// Store the profile under a different name
        #[arg(long)]
        name: Option<String>,
    },
    /// List stored profiles
    List,
    /// Show profile details and its sslim:// URI
    Show {
        name: Option<String>,
        /// Print the profile as a terminal QR code
        #[arg(long)]
        qr: bool,
    },
    /// Delete a stored profile
    Remove { name: String },
}

#[derive(clap::Args)]
pub struct ConnectArgs {
    /// Profile name to connect with (see `sslim blade list`)
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub server_name: Option<String>,
    #[arg(long)]
    pub ca: Option<PathBuf>,
    #[arg(long)]
    pub secret_file: Option<PathBuf>,
    #[arg(long)]
    pub wintun_dll: Option<PathBuf>,
    #[arg(long)]
    pub route_script: Option<PathBuf>,
    /// Print per-second traffic counters
    #[arg(long, num_args(0..=1), default_missing_value = "true")]
    pub stats: Option<bool>,
}

struct Resolved {
    profile: Profile,
    server_ip: Ipv4Addr,
    key: hmac::Key,
    wintun_dll: PathBuf,
    route_script: PathBuf,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn resolve(args: &BladeArgs) -> Result<Resolved> {
    let flags = &args.connect;
    let mut profile = if let Some(name) = &flags.profile {
        profile::load(name)?
    } else if flags.host.is_some() || flags.secret_file.is_some() {
        Profile {
            name: "cli".to_owned(),
            host: flags
                .host
                .clone()
                .context("--host is required with --secret-file")?,
            port: profile::DEFAULT_PORT,
            server_name: String::new(),
            marker: profile::DEFAULT_MARKER.to_owned(),
            secret_base64: String::new(),
            secret_file: None,
            ca_file: None,
            ca_base64: None,
            stats: false,
        }
    } else {
        profile::default_profile()?.context(
            "no profiles yet: import one with `sslim blade import \"sslim://...\"` or pass --host/--secret-file",
        )?
    };
    if let Some(host) = &flags.host {
        profile.host = host.clone();
    }
    if let Some(port) = flags.port {
        profile.port = port;
    }
    if profile.server_name.is_empty() {
        profile.server_name = profile.host.clone();
    }
    if let Some(name) = &flags.server_name {
        profile.server_name = name.clone();
    }
    if let Some(ca) = &flags.ca {
        profile.ca_file = Some(ca.clone());
    }
    if let Some(secret_file) = &flags.secret_file {
        profile.secret_file = Some(secret_file.clone());
        profile.secret_base64.clear();
    }
    if let Some(stats) = flags.stats {
        profile.stats = stats;
    }
    profile.validate()?;

    let server_ip = match profile.host.parse::<Ipv4Addr>() {
        Ok(ip) => ip,
        Err(_) => {
            let addresses = (profile.host.as_str(), profile.port)
                .to_socket_addrs()
                .with_context(|| format!("resolving {}", profile.host))?
                .find_map(|addr| match addr {
                    std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
                    std::net::SocketAddr::V6(_) => None,
                });
            addresses.with_context(|| format!("{} resolved to no IPv4 address", profile.host))?
        }
    };

    let key = key_from_bytes(&profile.secret()?);
    let wintun_dll = match &flags.wintun_dll {
        Some(path) => path.clone(),
        None => exe_dir()
            .map(|dir| dir.join("wintun.dll"))
            .filter(|path| path.exists())
            .with_context(
                || "wintun.dll not found: pass --wintun-dll or place it next to sslim.exe",
            )?
            .clone(),
    };
    let route_script = match &flags.route_script {
        Some(path) => path.clone(),
        None => exe_dir()
            .map(|dir| dir.join("blade-routes.ps1"))
            .filter(|path| path.exists())
            .with_context(
                || "blade-routes.ps1 not found: pass --route-script or place it next to sslim.exe",
            )?
            .clone(),
    };

    profile::set_last(&profile.name)?;
    Ok(Resolved {
        profile,
        server_ip,
        key,
        wintun_dll: wintun_dll.canonicalize()?,
        route_script: route_script.canonicalize()?,
    })
}

fn save_imported_profile(mut profile: Profile) -> Result<PathBuf> {
    profile.validate()?;
    if let Some(ca) = profile.ca_base64.take() {
        let ca_path = profile::profiles_dir()?.join(format!("{}.ca.pem", profile.name));
        std::fs::write(&ca_path, ca)?;
        profile.ca_file = Some(ca_path);
    }
    profile::save(&profile)
}

#[derive(Default)]
struct Counters {
    up: AtomicU64,
    down: AtomicU64,
    ring_full: AtomicU64,
}

struct Reader {
    session: Arc<Session>,
    batch: mpsc::Receiver<Result<(Vec<u8>, u64)>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Reader {
    fn start(session: Arc<Session>) -> Result<Self> {
        let (queue, batch) = mpsc::channel(16);
        let read = session.clone();
        let thread = thread::Builder::new()
            .name("blade-wintun".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    loop {
                        let mut frames = Vec::with_capacity(40 * 1024);
                        let mut size = 0;
                        let mut packet = read.receive_blocking()?;
                        for index in 0..32 {
                            let data = packet.bytes();
                            if !data.is_empty() && data[0] >> 4 == 4 && valid_ip(data) {
                                append_packet(&mut frames, data);
                                size += data.len() as u64;
                            }
                            if index == 31 || frames.len() >= 32 * 1024 {
                                break;
                            }
                            let Some(next) = read.try_receive()? else {
                                break;
                            };
                            packet = next;
                        }
                        if !frames.is_empty() && queue.blocking_send(Ok((frames, size))).is_err() {
                            return Ok(());
                        }
                    }
                })();
                if let Err(error) = result {
                    let _ = queue.blocking_send(Err(error));
                }
            })?;
        Ok(Self {
            session,
            batch,
            thread: Some(thread),
        })
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.batch.close();
        let _ = self.session.shutdown();
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn public_prefixes(server: Ipv4Addr) -> String {
    let ip = u32::from(server);
    (1..=32)
        .map(|prefix| {
            let bit = 32 - prefix;
            let next = (ip ^ (1 << bit)) & (u32::MAX << bit);
            format!("{}/{prefix}", Ipv4Addr::from(next))
        })
        .collect::<Vec<_>>()
        .join(",")
}

async fn configure(script: &Path, index: u32, server: Ipv4Addr) -> Result<()> {
    run_route_script(
        script,
        [
            "-InterfaceIndex",
            &index.to_string(),
            "-ServerIP",
            &server.to_string(),
            "-Prefixes",
            &public_prefixes(server),
            "-BlockIPv6",
        ],
    )
    .await
}

async fn revert_routes(script: &Path) {
    let _ = run_route_script(script, ["-Revert"]).await;
}

async fn run_route_script<const N: usize>(script: &Path, args: [&str; N]) -> Result<()> {
    let script = script.canonicalize()?;
    let status = tokio::process::Command::new("powershell.exe")
        .kill_on_drop(true)
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(args)
        .status()
        .await?;
    ensure!(status.success(), "Blade route setup failed ({status})");
    Ok(())
}

async fn send_wintun(session: &Arc<Session>, packet: &[u8], counters: &Counters) -> Result<()> {
    if packet[0] >> 4 != 4 {
        return Ok(());
    }
    loop {
        match session.allocate_send_packet(packet.len() as u16) {
            Ok(mut frame) => {
                frame.bytes_mut().copy_from_slice(packet);
                session.send_packet(frame);
                counters
                    .down
                    .fetch_add(packet.len() as u64, Ordering::Relaxed);
                return Ok(());
            }
            Err(wintun::Error::Io(error)) if error.raw_os_error() == Some(111) => {
                counters.ring_full.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn verify_exit() -> Result<String> {
    let response = tokio::process::Command::new("curl.exe")
        .kill_on_drop(true)
        .args([
            "-4fsS",
            "--noproxy",
            "*",
            "--max-time",
            "20",
            "https://1.1.1.1/cdn-cgi/trace",
        ])
        .output()
        .await?;
    let output = std::str::from_utf8(&response.stdout)?;
    let exit_ip = output
        .lines()
        .find_map(|line| line.strip_prefix("ip="))
        .map(str::to_owned);
    ensure!(
        response.status.success() && exit_ip.is_some(),
        "could not verify the tunnel exit IP"
    );
    Ok(exit_ip.expect("checked above"))
}

async fn report(counters: &Counters, enabled: bool) -> Result<()> {
    if !enabled {
        std::future::pending::<()>().await;
    }
    let mut previous_up = 0;
    let mut previous_down = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.tick().await;
    loop {
        interval.tick().await;
        let up = counters.up.load(Ordering::Relaxed);
        let down = counters.down.load(Ordering::Relaxed);
        let full = counters.ring_full.load(Ordering::Relaxed);
        println!(
            "{DIM}up={:.2} Mbps down={:.2} Mbps wintun_backpressure={full}{RESET}",
            (up - previous_up) as f64 * 8e-6,
            (down - previous_down) as f64 * 8e-6
        );
        previous_up = up;
        previous_down = down;
    }
}

async fn connect_once(resolved: &Resolved) -> Result<()> {
    let profile = &resolved.profile;
    println!(
        "connecting to {DIM}{}:{}{RESET} (SNI {DIM}{}{RESET})",
        profile.host, profile.port, profile.server_name
    );
    let tls = client_tls(profile.ca_file.as_deref())?;
    let mut stream = connect(
        &profile.host,
        profile.port,
        &profile.server_name,
        tls,
        &resolved.key,
        &profile.marker,
    )
    .await?;
    println!("{OK_MARK} tunnel established, exit will be checked shortly");

    let file = &resolved.wintun_dll;
    // SAFETY: the launcher supplies the absolute path of the locally installed Wintun DLL.
    let wintun = unsafe { wintun::load_from_path(file)? };
    let adapter = Adapter::create(&wintun, "Blade", "SSLim", None)
        .context("could not create Wintun adapter; run from an elevated terminal")?;
    let session = Arc::new(adapter.start_session(4 * 1024 * 1024)?);
    let route_script = resolved.route_script.clone();
    configure(
        &route_script,
        adapter.get_adapter_index()?,
        resolved.server_ip,
    )
    .await?;
    let mut reader = Reader::start(session.clone())?;
    let counters = Counters::default();
    let (input, mut output) = io::split(&mut stream);
    let (pong, mut control) = mpsc::channel::<()>(16);
    let uplink = async {
        let mut keepalive = tokio::time::interval(KEEPALIVE);
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
        keepalive.tick().await;
        loop {
            tokio::select! {
                biased;
                Some(()) = control.recv() => output.write_all(PONG_FRAME).await?,
                _ = keepalive.tick() => output.write_all(&[PING, 0, 0]).await?,
                next = reader.batch.recv() => {
                    let (frames, size) = next.context("Wintun reader ended")??;
                    output.write_all(&frames).await?;
                    counters.up.fetch_add(size, Ordering::Relaxed);
                }
            }
            output.flush().await?;
        }
    };
    let downlink = async {
        let mut frames = FrameReader::new(input);
        loop {
            let frame = timeout(READ_IDLE, frames.next()).await.map_err(|_| {
                std::io::Error::new(io::ErrorKind::TimedOut, "no data from the server")
            })??;
            match frame {
                (PACKET, packet) => send_wintun(&session, packet, &counters).await?,
                (PING, _) => pong.send(()).await?,
                (CLOSE, _) => return Ok(()),
                _ => {}
            }
        }
    };
    let check = async {
        let ip = verify_exit().await?;
        println!("{OK_MARK} exit IP: {ip}. Press Ctrl+C to disconnect.");
        std::future::pending::<Result<()>>().await
    };
    let result = tokio::select! {
        result = uplink => result,
        result = downlink => result,
        result = check => result,
        result = report(&counters, profile.stats) => result,
        result = tokio::signal::ctrl_c() => result.map_err(Into::into),
    };
    drop(reader);
    revert_routes(&route_script).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        stream.write_all(&[CLOSE, 0, 0]).await?;
        stream.shutdown().await
    })
    .await;
    result
}

pub async fn run(args: BladeArgs) -> Result<()> {
    match args.command {
        Some(BladeCommand::Import { source, name }) => {
            let mut profile = if source.starts_with("sslim://") {
                Profile::from_uri(&source)?
            } else {
                let text = std::fs::read_to_string(&source)
                    .with_context(|| format!("reading {}", source))?;
                toml::from_str(&text).context("profile file is not valid TOML")?
            };
            if let Some(name) = name {
                profile.name = name;
            }
            let name = profile.name.clone();
            let path = save_imported_profile(profile)?;
            println!("{OK_MARK} profile {:?} saved to {}", name, path.display());
        }
        Some(BladeCommand::List) => {
            let profiles = profile::list()?;
            if profiles.is_empty() {
                println!(
                    "{DIM}no profiles yet; import one with `sslim blade import \"sslim://...\"`{RESET}"
                );
                return Ok(());
            }
            let width = profiles
                .iter()
                .map(|profile| profile.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            println!(
                "{DIM}{:<width$}  {:<26} {:<5} server name{RESET}",
                "name",
                "host",
                "port",
                width = width
            );
            for profile in profiles {
                println!(
                    "{:<width$}  {:<26} {:<5} {}",
                    profile.name,
                    profile.host,
                    profile.port,
                    profile.server_name,
                    width = width
                );
            }
        }
        Some(BladeCommand::Show { name, qr }) => {
            let profile = match name {
                Some(name) => profile::load(&name)?,
                None => profile::default_profile()?.context("no profiles yet; import one first")?,
            };
            println!("name        {}", profile.name);
            println!("host        {}:{}", profile.host, profile.port);
            println!("server name {}", profile.server_name);
            println!(
                "CA          {}",
                profile
                    .ca_file
                    .as_deref()
                    .map_or("system".to_owned(), |path| path.display().to_string())
            );
            if qr {
                print!("{}", crate::render_qr(&profile.to_uri()?)?);
            } else {
                println!("URI         {}", profile.to_uri()?);
            }
        }
        Some(BladeCommand::Remove { name }) => {
            if profile::remove(&name)? {
                println!("{OK_MARK} profile {name:?} removed");
            } else {
                println!("{WARN_MARK} no profile named {name:?}");
            }
        }
        None => {
            let resolved = resolve(&args)?;
            let mut delay: u64 = 1;
            loop {
                match connect_once(&resolved).await {
                    Ok(()) => {
                        println!("{OK_MARK} session closed by the server; exiting");
                        return Ok(());
                    }
                    Err(error) => {
                        eprintln!(
                            "{WARN_MARK} {error}{RESET}\n{DIM}reconnecting in {delay}s (Ctrl+C to quit){RESET}"
                        );
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                            _ = tokio::signal::ctrl_c() => return Ok(()),
                        }
                        delay = (delay.saturating_mul(2)).min(RECONNECT_MAX_SECS);
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn console() -> Result<()> {
    use std::io::{self, Write};

    println!("Blade · SSLim");
    println!("Введите /help или !help. Вставьте sslim:// ссылку, чтобы добавить сервер.");
    loop {
        print!("blade> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "/help" | "!help" | "help" | "/?") {
            println!("/help             показать команды");
            println!("list              показать сохранённые серверы");
            println!("add <sslim://…>   добавить сервер по ссылке");
            println!("connect [имя]     подключиться или вставить ссылку");
            println!("exit              закрыть Blade");
            continue;
        }
        if matches!(line, "exit" | "quit" | "/exit") {
            return Ok(());
        }
        if matches!(line, "list" | "/list") {
            let profiles = profile::list()?;
            if profiles.is_empty() {
                println!("Серверов пока нет. Вставьте sslim:// ссылку или выполните add <ссылка>.");
            } else {
                for profile in profiles {
                    println!("{}  {}:{}", profile.name, profile.host, profile.port);
                }
            }
            continue;
        }

        let source = if let Some(source) = line.strip_prefix("add ") {
            Some((source.trim(), false))
        } else if line.starts_with("sslim://") {
            Some((line, true))
        } else {
            None
        };
        let profile_name = if let Some((source, connect_now)) = source {
            match Profile::from_uri(source).and_then(|profile| {
                let name = profile.name.clone();
                save_imported_profile(profile)?;
                Ok(name)
            }) {
                Ok(name) => {
                    println!("Сервер «{name}» сохранён.");
                    if !connect_now {
                        continue;
                    }
                    name
                }
                Err(error) => {
                    eprintln!("Не удалось прочитать ссылку: {error:#}");
                    continue;
                }
            }
        } else if line == "connect" || line.starts_with("connect ") {
            let rest = line.strip_prefix("connect").unwrap_or_default();
            let name = rest.trim();
            if !name.is_empty() {
                name.to_owned()
            } else {
                match profile::default_profile()? {
                    Some(profile) => profile.name,
                    None => {
                        print!("Вставьте ссылку сервера: ");
                        io::stdout().flush()?;
                        let mut uri = String::new();
                        io::stdin().read_line(&mut uri)?;
                        match Profile::from_uri(uri.trim()) {
                            Ok(profile) => {
                                let name = profile.name.clone();
                                save_imported_profile(profile)?;
                                name
                            }
                            Err(error) => {
                                eprintln!("Не удалось прочитать ссылку: {error:#}");
                                continue;
                            }
                        }
                    }
                }
            }
        } else {
            println!("Команда не распознана. Введите /help.");
            continue;
        };

        if let Err(error) = run(BladeArgs {
            command: None,
            connect: ConnectArgs {
                profile: Some(profile_name),
                host: None,
                port: None,
                server_name: None,
                ca: None,
                secret_file: None,
                wintun_dll: None,
                route_script: None,
                stats: Some(true),
            },
        })
        .await
        {
            eprintln!("Не удалось подключиться: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_route_prefixes_have_no_overlap_with_server() {
        let server = Ipv4Addr::new(203, 0, 113, 10);
        let prefixes = public_prefixes(server);
        let all: Vec<_> = prefixes.split(',').collect();
        assert_eq!(all.len(), 32);
        for entry in all {
            let (network, prefix) = entry.split_once('/').unwrap();
            let network = u32::from(network.parse::<Ipv4Addr>().unwrap());
            let mask = u32::MAX << (32 - prefix.parse::<u32>().unwrap());
            assert_ne!(u32::from(server) & mask, network);
        }
    }
}
