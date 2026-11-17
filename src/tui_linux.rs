//! Linux server administration, client profiles and network tools.
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, version};

use crate::profile::{self, Profile};
use crate::session::{connect, relay};
use crate::tui::{argv, choose_profile, confirm, invoke, pause, pick, profiles_menu, prompt, required};
use crate::tun::Tun;

fn systemctl(action: &str) -> Result<()> {
    ensure!(Path::new("/etc/systemd/system/sslim.service").exists(),
        "SSLim systemd service не установлен на этом сервере");
    let exit = std::process::Command::new("systemctl")
        .arg("--no-pager")
        .arg(action)
        .arg("sslim.service")
        .status().context("не удалось запустить systemctl")?;
    if action != "status" {
        ensure!(exit.success(), "systemctl {action} завершился с ошибкой");
    }
    Ok(())
}

async fn manual_server() -> Result<()> {
    println!("Ручной сервер работает до прерывания Ctrl+C; сетевой интерфейс и NAT должны быть настроены.");
    let host = prompt("IP для прослушивания", "0.0.0.0")?;
    let port = prompt("Порт TCP", "9443")?;
    let cert = required("Путь к сертификату PEM", "/etc/sslim/server.pem")?;
    let key = required("Путь к TLS ключу PEM", "/etc/sslim/server.key")?;
    let secrets = required("Список клиентов", "/etc/sslim/clients.txt")?;
    let tun = prompt("Имя TUN", "sslim0")?;
    let mut args = argv(&["server", "--host", &host, "--port", &port, "--cert", &cert,
        "--key", &key, "--secrets-file", &secrets, "--device", "tun",
        "--tun-name", &tun, "--multi-tun"]);
    let cc = prompt("TCP congestion control (Enter — системный)", "")?;
    if !cc.is_empty() { args.extend(argv(&["--congestion-control", &cc])); }
    invoke(args).await
}

async fn service_menu() -> Result<()> {
    let items = ["Статус sslim.service", "Запустить службу", "Перезапустить службу",
        "Остановить службу", "Запустить сервер вручную"];
    while let Some(n) = pick("Linux · сервер", &items)? {
        let result = match n {
            0 => systemctl("status"),
            1 => systemctl("start"),
            2 => if confirm("Перезапустить службу? VPN-сеансы оборвутся")? {
                systemctl("restart")
            } else { Ok(()) },
            3 => if confirm("Остановить службу? VPN-сеансы оборвутся")? {
                systemctl("stop")
            } else { Ok(()) },
            4 => manual_server().await,
            _ => Ok(()),
        };
        if let Err(e) = result { eprintln!("Ошибка: {e:#}"); }
        pause();
    }
    Ok(())
}

fn client_args(label: &str, extra: &str) -> Result<Vec<String>> {
    let default_host = std::fs::read_to_string("/etc/sslim/hostname")
        .unwrap_or_default().trim().to_owned();
    let host = required("Адрес сервера, включаемый в профиль", &default_host)?;
    let mut args = argv(&["clients", "add", "--label", label, "--host", &host,
        "--secrets-file", "/etc/sslim/clients.txt"]);
    if Path::new("/etc/sslim/ca.pem").is_file() {
        args.extend(argv(&["--ca-file", "/etc/sslim/ca.pem"]));
    }
    if extra == "show" { args.push("--show".to_owned()); }
    if extra == "rotate" { args.push("--rotate".to_owned()); }
    Ok(args)
}

async fn clients_menu() -> Result<()> {
    let items = ["Список клиентов", "Добавить клиента", "Показать ссылку / QR",
        "Перевыпустить секрет", "Отозвать клиента"];
    while let Some(n) = pick("Linux · ключи клиентов", &items)? {
        let result = match n {
            0 => invoke(argv(&["clients", "list", "--secrets-file", "/etc/sslim/clients.txt"])).await,
            1 => {
                let name = required("Имя нового клиента", "")?;
                invoke(client_args(&name, "add")?).await
            }
            2 => {
                let name = required("Имя существующего клиента", "")?;
                if confirm("Показать приватную ссылку с секретом")? {
                    invoke(client_args(&name, "show")?).await
                } else { Ok(()) }
            }
            3 => {
                let name = required("Имя клиента", "")?;
                if confirm(&format!("Перевыпустить секрет для {name}? Старый ключ будет отозван"))? {
                    invoke(client_args(&name, "rotate")?).await
                } else { Ok(()) }
            }
            4 => {
                let name = required("Имя клиента", "")?;
                if confirm(&format!("Отозвать ключ клиента {name}"))? {
                    invoke(argv(&["clients", "remove", &name,
                        "--secrets-file", "/etc/sslim/clients.txt"])).await
                } else { Ok(()) }
            }
            _ => Ok(()),
        };
        if let Err(e) = result { eprintln!("Ошибка: {e:#}"); }
        pause();
    }
    Ok(())
}

fn tls_for_profile(profile: &Profile) -> Result<TlsConnector> {
    if let Some(ca) = &profile.ca_base64 {
        let pem = STANDARD.decode(ca).context("CA в профиле не является base64")?;
        let mut roots = RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut std::io::BufReader::new(pem.as_slice())) {
            roots.add(cert?)?;
        }
        ensure!(!roots.is_empty(), "не найдены сертификаты в CA профиля");
        let mut config = ClientConfig::builder_with_protocol_versions(&[&version::TLS13])
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(TlsConnector::from(Arc::new(config)))
    } else {
        crate::client_tls(profile.ca_file.as_deref())
    }
}

async fn connect_profile() -> Result<()> {
    let Some(profile) = choose_profile()? else { return Ok(()); };
    let tun_name = prompt("Имя уже настроенного TUN", "labtun0")?;
    profile::set_last(&profile.name)?;
    let key = profile.secret()?;
    let tls = tls_for_profile(&profile)?;
    let tun = Tun::open(&tun_name)?;
    println!("Соединение {}:{}. Ctrl+C — завершить.", profile.host, profile.port);
    let stream = connect(&profile.host, profile.port, &profile.server_name,
        tls, &key, &profile.marker).await?;
    relay(stream, tun).await
}

async fn quic_menu() -> Result<()> {
    let items = ["Запустить QUIC-сервер", "Подключить QUIC-клиент", "QUIC probe"];
    while let Some(n) = pick("Linux · экспериментальный QUIC", &items)? {
        let result = match n {
            0 => {
                let listen = prompt("IP:port для сервера", "0.0.0.0:9543")?;
                let cert = required("Сертификат PEM", "/etc/sslim/server.pem")?;
                let key = required("Ключ PEM", "/etc/sslim/server.key")?;
                let secret = required("Файл секрета", "session.bin")?;
                let tun = prompt("Имя TUN (пусто — echo)", "")?;
                let mut args = argv(&["quic", "server", "--listen", &listen, "--cert", &cert,
                    "--key", &key, "--secret-file", &secret]);
                if !tun.is_empty() { args.extend(argv(&["--tun-name", &tun])); }
                invoke(args).await
            }
            1 | 2 => {
                let address = required("IP:port сервера", "")?;
                let name = required("Имя TLS SNI", "")?;
                let secret = required("Файл секрета", "session.bin")?;
                let ca = prompt("CA PEM (пусто — системные)", "")?;
                let sub = if n == 1 { "client" } else { "probe" };
                let mut args = argv(&["quic", sub, "--address", &address,
                    "--server-name", &name, "--secret-file", &secret]);
                if !ca.is_empty() { args.extend(argv(&["--ca", &ca])); }
                if n == 1 {
                    let tun = prompt("Имя TUN", "labtun0")?;
                    args.extend(argv(&["--tun-name", &tun]));
                }
                invoke(args).await
            }
            _ => Ok(()),
        };
        if let Err(e) = result { eprintln!("Ошибка: {e:#}"); }
        pause();
    }
    Ok(())
}

async fn tools_menu() -> Result<()> {
    let items = ["Создать случайный секрет", "Сформировать URI и QR",
        "Проверить Linux TUN", "TLS Echo Probe"];
    while let Some(n) = pick("Linux · инструменты и диагностика", &items)? {
        let result = match n {
            0 => {
                let path = prompt("Выходной файл", "session.bin")?;
                invoke(argv(&["keygen", "--output", &path])).await
            }
            1 => {
                let host = required("Хост", "")?;
                let secret = required("Файл секрета", "session.bin")?;
                let port = prompt("Порт", "9443")?;
                invoke(argv(&["profile", "--host", &host, "--secret-file", &secret,
                    "--port", &port])).await
            }
            2 => {
                let tun = prompt("Имя TUN", "labcheck0")?;
                invoke(argv(&["tun-check", "--tun-name", &tun])).await
            }
            3 => {
                let host = required("Хост", "")?;
                let name = required("TLS server name", &host)?;
                let port = prompt("Порт", "9443")?;
                let secret = required("Файл секрета", "session.bin")?;
                let ca = prompt("CA PEM (пусто — системные)", "")?;
                let mut args = argv(&["probe", "--host", &host, "--server-name", &name,
                    "--secret-file", &secret, "--port", &port]);
                if !ca.is_empty() { args.extend(argv(&["--ca", &ca])); }
                invoke(args).await
            }
            _ => Ok(()),
        };
        if let Err(e) = result { eprintln!("Ошибка: {e:#}"); }
        pause();
    }
    Ok(())
}

pub(crate) async fn run() -> Result<()> {
    let menu = ["Сервер и служба", "Ключи клиентов", "VPN-клиент (TUN)",
        "Профили", "QUIC (экспериментально)", "Диагностика",
        "О программе", "Выход"];
    loop {
        match pick("Главное меню · Linux", &menu)? {
            Some(0) => service_menu().await?,
            Some(1) => clients_menu().await?,
            Some(2) => {
                if let Err(e) = connect_profile().await { eprintln!("Ошибка: {e:#}"); }
                pause();
            }
            Some(3) => crate::tui::profiles_menu().await?,
            Some(4) => quic_menu().await?,
            Some(5) => tools_menu().await?,
            Some(6) => crate::tui::about_menu().await?,
            _ => break,
        }
    }
    Ok(())
}
