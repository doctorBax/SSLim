//! Cross-platform interactive terminal menus. Existing clap commands remain usable.
use std::io::{self, IsTerminal, Read, Write};
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use crate::{Cli, dispatch_command};

enum Key { Up, Down, Select, Back, Digit(usize), Other }

#[cfg(unix)]
struct Raw(libc::termios);

#[cfg(unix)]
impl Raw {
    fn new() -> io::Result<Self> {
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(0, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(original))
    }
}
#[cfg(unix)]
impl Drop for Raw {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.0) };
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> Handle;
        fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: Handle, mode: u32) -> i32;
        fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    }
    pub struct Raw {
        input: Handle,
        input_mode: u32,
        output: Handle,
        output_mode: u32,
    }
    impl Raw {
        pub fn new() -> io::Result<Self> {
            unsafe {
                let input = GetStdHandle((-10i32) as u32);
                let output = GetStdHandle((-11i32) as u32);
                let mut imode = 0;
                let mut omode = 0;
                if GetConsoleMode(input, &mut imode) == 0
                    || GetConsoleMode(output, &mut omode) == 0 {
                    return Err(io::Error::last_os_error());
                }
                // VT input yields standard ANSI arrow sequences. Fall back to digits
                // when a legacy Windows host cannot enable virtual-terminal mode.
                if SetConsoleMode(output, omode | 0x0004) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if SetConsoleMode(input, (imode & !(0x0002 | 0x0004)) | 0x0200) == 0 {
                    let _ = SetConsoleMode(output, omode);
                    return Err(io::Error::last_os_error());
                }
                Ok(Self { input, input_mode: imode, output, output_mode: omode })
            }
        }
    }
    impl Drop for Raw {
        fn drop(&mut self) {
            unsafe {
                let _ = SetConsoleMode(self.input, self.input_mode);
                let _ = SetConsoleMode(self.output, self.output_mode);
            }
        }
    }
    pub fn ready(ms: u32) -> bool {
        unsafe { WaitForSingleObject(GetStdHandle((-10i32) as u32), ms) == 0 }
    }
}
#[cfg(windows)]
use win::Raw;

fn ready(ms: i32) -> bool {
    #[cfg(unix)] {
        let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut fd, 1, ms) > 0 && fd.revents & libc::POLLIN != 0 }
    }
    #[cfg(windows)] { win::ready(ms as u32) }
}

fn key() -> io::Result<Key> {
    let mut b = [0u8; 1];
    if io::stdin().read_exact(&mut b).is_err() { return Ok(Key::Back); }
    Ok(match b[0] {
        b'\r' | b'\n' => Key::Select,
        b'0' | b'q' | b'Q' | 8 | 127 => Key::Back,
        b'k' | b'K' | b'w' | b'W' => Key::Up,
        b'j' | b'J' | b's' | b'S' => Key::Down,
        b'1'..=b'9' => Key::Digit((b[0] - b'1') as usize),
        27 => {
            if !ready(80) { Key::Back } else {
                io::stdin().read_exact(&mut b)?;
                if b[0] != b'[' && b[0] != b'O' || !ready(80) {
                    Key::Back
                } else {
                    io::stdin().read_exact(&mut b)?;
                    match b[0] { b'A' => Key::Up, b'B' => Key::Down, _ => Key::Other }
                }
            }
        }
        _ => Key::Other,
    })
}

/// Returns a zero-based selected menu entry or None when the user goes back.
pub(crate) fn pick(title: &str, items: &[&str]) -> Result<Option<usize>> {
    ensure!(!items.is_empty() && items.len() <= 9, "menu supports 1..=9 entries");
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        if let Ok(raw) = Raw::new() {
            let response = raw_menu(title, items);
            drop(raw);
            print!("\x1b[?25h\x1b[0m\n");
            io::stdout().flush()?;
            return response;
        }
    }
    number_menu(title, items)
}

fn raw_menu(title: &str, items: &[&str]) -> Result<Option<usize>> {
    let mut current = 0usize;
    loop {
        print!("\x1b[2J\x1b[H\x1b[?25l\x1b[1;36m╭── SSLim ─────────────────────────────╮\x1b[0m\n");
        println!("\x1b[1m{title}\x1b[0m\n");
        for (i, name) in items.iter().enumerate() {
            if i == current {
                println!("\x1b[1;36m  ❯ {}. {name}\x1b[0m", i+1);
            } else {
                println!("    {}. {name}", i+1);
            }
        }
        println!("\n\x1b[2m↑↓ — выбрать · Enter — открыть · 1–9 — быстро · Esc/0 — назад\x1b[0m");
        io::stdout().flush()?;
        match key()? {
            Key::Up => current = if current == 0 { items.len()-1 } else { current-1 },
            Key::Down => current = (current+1)%items.len(),
            Key::Select => return Ok(Some(current)),
            Key::Digit(n) if n < items.len() => return Ok(Some(n)),
            Key::Back => return Ok(None),
            _ => {}
        }
    }
}

fn number_menu(title: &str, items: &[&str]) -> Result<Option<usize>> {
    println!("\nSSLim — {title}");
    for (i, name) in items.iter().enumerate() { println!("  {}. {name}", i+1); }
    loop {
        let value = prompt("Номер пункта (0 — назад)", "0")?;
        if value == "0" { return Ok(None); }
        if let Ok(number) = value.parse::<usize>() {
            if (1..=items.len()).contains(&number) { return Ok(Some(number-1)); }
        }
        println!("Введите допустимый номер.");
    }
}

pub(crate) fn prompt(label: &str, default: &str) -> Result<String> {
    if default.is_empty() { print!("{label}: "); }
    else { print!("{label} [{default}]: "); }
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 { bail!("ввод завершён"); }
    let s = input.trim();
    Ok(if s.is_empty() { default.to_owned() } else { s.to_owned() })
}

pub(crate) fn required(label: &str, default: &str) -> Result<String> {
    loop {
        let value = prompt(label, default)?;
        if !value.is_empty() { return Ok(value); }
        println!("Поле обязательно.");
    }
}

pub(crate) fn confirm(label: &str) -> Result<bool> {
    let value = prompt(&format!("{label} (да/нет)"), "нет")?;
    Ok(matches!(value.to_lowercase().as_str(), "да" | "д" | "yes" | "y"))
}

pub(crate) fn pause() {
    println!("\nEnter — продолжить");
    let _ = io::stdin().read_line(&mut String::new());
}

pub(crate) fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|v| (*v).to_owned()).collect()
}

pub(crate) async fn invoke(args: Vec<String>) -> Result<()> {
    let mut cli = vec!["sslim".to_owned()];
    cli.extend(args);
    let parsed = Cli::try_parse_from(cli).context("неверные параметры")?;
    dispatch_command(parsed.command.context("не выбрана команда")?).await
}

pub(crate) fn choose_profile() -> Result<Option<crate::profile::Profile>> {
    let profiles = crate::profile::list()?;
    if profiles.is_empty() {
        println!("Профилей нет. Сначала импортируйте ссылку sslim://.");
        return Ok(None);
    }
    let mut offset = 0;
    loop {
        let length = (profiles.len()-offset).min(8);
        let mut entries: Vec<String> = profiles[offset..offset+length]
            .iter().map(|p| format!("{} · {}:{}", p.name, p.host, p.port)).collect();
        if profiles.len() > 8 { entries.push("Следующая страница / в начало".to_owned()); }
        let names: Vec<&str> = entries.iter().map(String::as_str).collect();
        match pick("Выберите профиль", &names)? {
            Some(i) if i < length => return Ok(Some(profiles[offset+i].clone())),
            Some(_) => offset = if offset+length == profiles.len() { 0 } else { offset+length },
            None => return Ok(None)
        }
    }
}

pub(crate) fn import_profile() -> Result<()> {
    println!("Ссылка содержит секрет доступа. Не публикуйте её в логах и скриншотах.");
    let uri = required("Вставьте sslim:// ссылку", "")?;
    let profile = crate::profile::Profile::from_uri(&uri)?;
    let path = crate::profile::save(&profile)?;
    crate::profile::set_last(&profile.name)?;
    println!("Профиль {} сохранён: {}", profile.name, path.display());
    Ok(())
}

fn show_profile() -> Result<()> {
    if let Some(profile) = choose_profile()? {
        println!("{}\n{}:{}", profile.name, profile.host, profile.port);
        if confirm("Показать ссылку с секретом и QR")? {
            let uri = profile.to_uri()?;
            println!("\n{uri}\n");
            print!("{}", crate::render_qr(&uri)?);
        }
    }
    Ok(())
}

pub(crate) async fn profiles_menu() -> Result<()> {
    let options = ["Список профилей", "Импорт sslim://", "Показать ссылку / QR", "Удалить профиль"];
    while let Some(n) = pick("Профили", &options)? {
        let answer = match n {
            0 => {
                let entries = crate::profile::list()?;
                if entries.is_empty() { println!("Список пуст."); }
                for p in entries { println!("• {} — {}:{}", p.name, p.host, p.port); }
                Ok(())
            }
            1 => import_profile(),
            2 => show_profile(),
            3 => {
                if let Some(profile) = choose_profile()? {
                    if confirm(&format!("Удалить профиль {}", profile.name))? {
                        crate::profile::remove(&profile.name)?;
                        println!("Удалено.");
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = answer { eprintln!("Ошибка: {e:#}"); }
        pause();
    }
    Ok(())
}

pub(crate) async fn about_menu() -> Result<()> {
    let options = ["Версия", "Ссылка на GitHub Releases"];
    while let Some(n) = pick("О программе", &options)? {
        match n {
            0 => println!("SSLim CLI {}", env!("CARGO_PKG_VERSION")),
            1 => println!("https://github.com/doctorBax/SSLim/releases\nОбновления только вручную."),
            _ => {}
        }
        pause();
    }
    Ok(())
}

pub(crate) async fn run() -> Result<()> {
    if !io::stdin().is_terminal() {
        println!("SSLim {}: интерактивное меню требует TTY. Команды: sslim --help",
            env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    #[cfg(target_os="linux")] { crate::tui_linux::run().await }
    #[cfg(windows)] { crate::tui_windows::run().await }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_args_unchanged_for_automation() {
        let c = <crate::Cli as clap::Parser>::try_parse_from([
            "sslim", "keygen", "--output", "test.bin",
        ]);
        assert!(c.is_ok());
    }
}
