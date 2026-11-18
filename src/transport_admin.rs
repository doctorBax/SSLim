//! Safe, reversible systemd configuration of dual TCP v1 + QUIC v3 mode.
//! Does not touch certificates, client secrets, firewall or existing TCP flags.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

const UNIT: &str = "/etc/systemd/system/sslim.service";
const BACKUPS: &str = "/etc/sslim";

fn port_from_args(args: &[&str], flag: &str) -> Result<Option<u16>> {
    let mut result = None;
    for (index, item) in args.iter().enumerate() {
        let value = if *item == flag {
            Some(*args.get(index + 1).context("У параметра сервера отсутствует значение")?)
        } else {
            item.strip_prefix(&format!("{flag}="))
        };
        if let Some(value) = value {
            ensure!(result.is_none(), "Повторяющийся параметр {flag} в ExecStart");
            let port: u16 = value.parse().context(format!("Некорректный {flag}: {value}"))?;
            ensure!(port != 0, "{flag} не может быть 0");
            result = Some(port);
        }
    }
    Ok(result)
}

/// Preserve the exact original TCP server ExecStart and all other unit lines.
/// Only append/remove the optional QUIC port; never replace the whole unit.
fn rewrite_unit(input: &str, enable: bool) -> Result<(String, u16, bool)> {
    let mut matches = 0usize;
    let mut result = String::new();
    let mut chosen_port = None;
    for line in input.split_inclusive('\n') {
        let content = line.trim_end_matches('\n').trim_end_matches('\r');
        let eol = &line[content.len()..];
        if !content.starts_with("ExecStart=") {
            result.push_str(line);
            continue;
        }
        matches += 1;
        ensure!(matches == 1, "Несколько ExecStart: автоматическое изменение небезопасно");
        let invocation = &content["ExecStart=".len()..];
        ensure!(!invocation.ends_with('\\'), "Многострочный ExecStart не поддерживается");
        let args: Vec<&str> = invocation.split_whitespace().collect();
        ensure!(args.len() >= 2 && Path::new(args[0]).file_name().is_some_and(|n| n == "sslim")
            && args[1] == "server", "ExecStart запускает не SSLim server");
        ensure!(args.windows(2).any(|w| w == ["--device", "tun"])
            || args.iter().any(|x| *x == "--device=tun"),
            "QUIC v3 требует --device tun");
        ensure!(args.contains(&"--multi-tun"),
            "QUIC v3 требует --multi-tun для уникальных адресов клиентов");
        let tcp_port = port_from_args(&args, "--port")?.unwrap_or(9443);
        let existing_udp = port_from_args(&args, "--udp-port")?;
        chosen_port = Some(tcp_port);
        let mut filtered = Vec::with_capacity(args.len());
        let mut index = 0;
        while index < args.len() {
            if args[index] == "--udp-port" {
                index += 2;
            } else if args[index].starts_with("--udp-port=") {
                index += 1;
            } else {
                filtered.push(args[index]);
                index += 1;
            }
        }
        if enable {
            // Reject unsafe whitespace-bearing options rather than unexpectedly
            // changing the shell-escaped original command by tokenizing it.
            ensure!(!invocation.contains('"') && !invocation.contains('\'')
                && !invocation.contains('\\'),
                "ExecStart содержит кавычки/экранирование: нужен ручной разбор");
            if existing_udp.is_none() {
                result.push_str(content);
                result.push_str(&format!(" --udp-port {tcp_port}"));
            } else if existing_udp == Some(tcp_port) {
                result.push_str(content);
            } else {
                result.push_str(&format!("ExecStart={} --udp-port {tcp_port}", filtered.join(" ")));
            }
        } else if existing_udp.is_some() {
            ensure!(!invocation.contains('"') && !invocation.contains('\'')
                && !invocation.contains('\\'),
                "ExecStart содержит кавычки/экранирование: нужен ручной разбор");
            result.push_str(&format!("ExecStart={}", filtered.join(" ")));
        } else {
            result.push_str(content);
        }
        result.push_str(eol);
    }
    ensure!(matches == 1, "Не найден ExecStart в конфигурации SSLim");
    let port = chosen_port.context("Порт сервера не обнаружен")?;
    let changed = input != result;
    Ok((result, port, changed))
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl").args(args).status()
        .context("systemctl недоступен")?;
    ensure!(status.success(), "systemctl {} завершился с ошибкой", args.join(" "));
    Ok(())
}

fn service_info(field: &str) -> Result<String> {
    let output = Command::new("systemctl")
        .args(["show", "-p", field, "--value", "sslim.service"])
        .output()
        .context("не удалось проверить настройки systemd")?;
    ensure!(output.status.success(), "systemctl show {field} неуспешен");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn atomic_write(unit: &Path, bytes: &[u8], permissions: fs::Permissions) -> Result<()> {
    let temp = unit.with_extension(format!("service.new.{}", std::process::id()));
    let mut file = OpenOptions::new().write(true).create_new(true)
        .mode(0o600).open(&temp)
        .with_context(|| format!("не удалось создать {}", temp.display()))?;
    let action = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        file.set_permissions(permissions)?;
        fs::rename(&temp, unit)?;
        Ok(())
    })();
    if action.is_err() { let _ = fs::remove_file(&temp); }
    action
}

/// Configure the existing persistent service; caller must explicitly confirm
/// the disruption. Roll back the original unit and try to restore TCP on error.
pub(crate) fn configure(enable: bool) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0,
        "Для настройки службы требуется sudo sslim");
    ensure!(service_info("FragmentPath")? == UNIT,
        "Служба SSLim установлена не в {UNIT}; автоматическая правка отменена");
    ensure!(service_info("DropInPaths")?.is_empty(),
        "У SSLim уже есть systemd drop-in overrides; нужна ручная проверка");
    if enable {
        let output = Command::new("/usr/local/bin/sslim")
            .args(["server", "--help"]).output()
            .context("не удалось проверить версию SSLim")?;
        ensure!(output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("--udp-port"),
            "Сначала обнови серверный бинарник до SSLim 0.6.0+");
    }
    let unit = Path::new(UNIT);
    let original = fs::read_to_string(unit)
        .with_context(|| format!("не удалось прочитать {UNIT}"))?;
    let (updated, port, changed) = rewrite_unit(&original, enable)?;
    println!("TCP v1: порт {port}, без изменений.");
    println!("QUIC v3: {}", if enable { format!("UDP {port}") } else { "выключен".to_owned() });
    if !changed {
        println!("Конфигурация уже соответствует выбранному режиму. Перезапуск не нужен.");
        return Ok(());
    }
    let stamps = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let backup = PathBuf::from(BACKUPS).join(format!("sslim.service.before-transport-{stamps}-{}", std::process::id()));
    fs::copy(unit, &backup)
        .with_context(|| format!("не удалось сделать резервную копию {}", backup.display()))?;
    let original_permissions = fs::metadata(unit)?.permissions();
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))?;

    atomic_write(unit, updated.as_bytes(), original_permissions.clone())?;
    let attempt = (|| -> Result<()> {
        systemctl(&["daemon-reload"])?;
        systemctl(&["restart", "sslim.service"])?;
        systemctl(&["is-active", "--quiet", "sslim.service"])?;
        Ok(())
    })();
    if let Err(error) = attempt {
        // Do not destroy keys, certificates, network rules or backups.
        let restore = (|| -> Result<()> {
            atomic_write(unit, original.as_bytes(), original_permissions)?;
            systemctl(&["daemon-reload"])?;
            systemctl(&["restart", "sslim.service"])?;
            Ok(())
        })();
        if let Err(rollback_error) = restore {
            bail!("Новый сервер не запустился: {error:#}. Откат тоже не завершился: {rollback_error:#}. Копия: {}", backup.display());
        }
        bail!("Новая конфигурация не запустилась: {error:#}. TCP-служба восстановлена из резервной копии {}", backup.display());
    }
    println!("Готово. SSLim service active; старый TCP и все ключи сохранены.");
    println!("Резервная копия: {}", backup.display());
    if enable {
        println!("Открой UDP порт {port} в файрволе VPS. Android Blade: Транспорт → Авто.");
        println!("Сервер TCP {port} / UDP {port}. QUIC v2 отдельно не нужен.");
    }
    Ok(())
}

pub(crate) fn status() -> Result<()> {
    let unit = fs::read_to_string(UNIT)
        .with_context(|| format!("не найден {UNIT}"))?;
    let (disabled, port, _) = rewrite_unit(&unit, false)?;
    let is_enabled = disabled != unit;
    println!("TCP {port}; QUIC v3 {}", if is_enabled { "включён в конфигурации службы" } else { "отключён" });
    if is_enabled {
        println!("Android Blade: Авто → QUIC v3, если UDP доступен; иначе TCP v1.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SERVICE: &str = "[Unit]\nDescription=SSLim\n[Service]\nExecStartPre=/etc/sslim/network up\nExecStart=/usr/local/bin/sslim server --host 0.0.0.0 --device tun --multi-tun --cert /etc/sslim/server.pem --key /etc/sslim/server.key --secrets-file /etc/sslim/clients.txt\nExecStopPost=/etc/sslim/network down\nRestart=on-failure\n";
    #[test]
    fn enable_is_idempotent_and_preserves_every_old_flag() {
        let (new, port, changed) = rewrite_unit(SERVICE, true).unwrap();
        assert_eq!(port, 9443);
        assert!(changed);
        assert!(new.contains(" --secrets-file /etc/sslim/clients.txt --udp-port 9443"));
        let (again, _, did_change) = rewrite_unit(&new, true).unwrap();
        assert_eq!(new, again);
        assert!(!did_change);
        assert_eq!(rewrite_unit(&new, false).unwrap().0, SERVICE);
    }
    #[test]
    fn existing_custom_port_and_other_lines_are_untouched() {
        let unit = SERVICE.replace("--device tun",
            "--port 9444 --device tun").replace("--cert", "--congestion-control bbr --cert");
        let (new, port, _) = rewrite_unit(&unit, true).unwrap();
        assert_eq!(port, 9444);
        assert!(new.contains("--udp-port 9444"));
        assert_eq!(rewrite_unit(&new, false).unwrap().0, unit);
    }
    #[test]
    fn wrong_server_configuration_is_rejected() {
        assert!(rewrite_unit(&SERVICE.replace("--multi-tun", ""), true).is_err());
        assert!(rewrite_unit(&SERVICE.replace("--device tun", "--device echo"), true).is_err());
        assert!(rewrite_unit("[Service]\nExecStart=/usr/bin/python old.py\n", true).is_err());
    }
    #[test]
    fn replacement_of_different_udp_port_is_reversible() {
        let old = SERVICE.replace("--secrets-file /etc/sslim/clients.txt",
            "--secrets-file /etc/sslim/clients.txt --udp-port 9543");
        let (new, port, _) = rewrite_unit(&old, true).unwrap();
        assert_eq!(port, 9443);
        assert!(new.contains("--udp-port 9443"));
        assert!(!new.contains("9543"));
    }
}
