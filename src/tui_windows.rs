//! Windows Blade terminal dashboard. Noninteractive flags still use clap unchanged.
use anyhow::Result;
use crate::profile;
use crate::tui::{choose_profile, invoke, pause, pick, profiles_menu};

async fn connect_profile() -> Result<()> {
    let Some(profile) = choose_profile()? else { return Ok(()); };
    profile::set_last(&profile.name)?;
    println!("Подключение к {}:{}. Для остановки нажмите Ctrl+C.",
        profile.host, profile.port);
    // Keep the existing Wintun, TLS, routes and reconnection implementation.
    invoke(vec!["blade".into(), "--profile".into(), profile.name,
        "--stats".into(), "true".into()]).await
}

pub(crate) async fn run() -> Result<()> {
    let items = ["Подключиться к VPN", "Профили серверов", "О программе", "Выход"];
    loop {
        match pick("Главное меню · Windows Blade", &items)? {
            Some(0) => {
                if let Err(error) = connect_profile().await {
                    eprintln!("Ошибка подключения: {error:#}");
                }
                pause();
            }
            Some(1) => profiles_menu().await?,
            Some(2) => crate::tui::about_menu().await?,
            _ => break,
        }
    }
    Ok(())
}
