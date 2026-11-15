use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Args, Subcommand};

use crate::auth::{KeyRing, generate_secret};
use crate::profile::{self, Profile};

#[derive(Args)]
pub struct ClientAdminArgs {
    #[command(subcommand)]
    action: ClientAction,
}

#[derive(Subcommand)]
enum ClientAction {
    /// Create a client key and print its import link
    Add {
        #[arg(long)]
        label: String,
        #[arg(long)]
        host: String,
        #[arg(long)]
        secrets_file: PathBuf,
        #[arg(long, default_value_t = profile::DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        server_name: Option<String>,
        #[arg(long)]
        ca_file: Option<PathBuf>,
    },
    /// List client labels without revealing keys
    List {
        #[arg(long)]
        secrets_file: PathBuf,
    },
    /// Revoke one client key
    Remove {
        label: String,
        #[arg(long)]
        secrets_file: PathBuf,
    },
}

struct FileLock(PathBuf);

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn lock_file(path: &Path) -> Result<FileLock> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let lock = path.with_extension("lock");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&lock)
        .with_context(|| format!("another client change may be running ({})", lock.display()))?;
    Ok(FileLock(lock))
}

fn entries(path: &Path) -> Result<Vec<(String, String)>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut entries = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (label, secret) = line.split_once(':').with_context(|| {
            format!("{}:{} is not label:base64", path.display(), line_number + 1)
        })?;
        ensure!(
            !entries.iter().any(|(existing, _)| existing == label),
            "duplicate client label {label:?}"
        );
        entries.push((label.to_owned(), secret.to_owned()));
    }
    Ok(entries)
}

fn write_entries(path: &Path, entries: &[(String, String)]) -> Result<()> {
    ensure!(!entries.is_empty(), "at least one client must remain");
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = path.with_extension(format!("tmp-{}-{stamp}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    for (label, secret) in entries {
        writeln!(file, "{label}:{secret}")?;
    }
    file.sync_all()?;
    fs::rename(&temporary, path).with_context(|| format!("updating {}", path.display()))?;
    Ok(())
}

pub fn run(args: ClientAdminArgs) -> Result<Option<String>> {
    match args.action {
        ClientAction::Add {
            label,
            host,
            secrets_file,
            port,
            server_name,
            ca_file,
        } => {
            let _lock = lock_file(&secrets_file)?;
            ensure!(
                !label.is_empty()
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)),
                "client label may contain only letters, digits, '.', '_' and '-'"
            );
            let mut entries = entries(&secrets_file)?;
            if !entries.is_empty() {
                KeyRing::from_file(&secrets_file)?;
            }
            ensure!(
                !entries.iter().any(|(existing, _)| existing == &label),
                "client {label:?} already exists"
            );
            let ca_base64 = ca_file
                .map(|path| {
                    fs::read_to_string(&path)
                        .with_context(|| format!("reading certificate {}", path.display()))
                })
                .transpose()?;
            let secret = generate_secret()?;
            let profile = Profile {
                name: label,
                host: host.clone(),
                port,
                server_name: server_name.unwrap_or(host),
                marker: profile::DEFAULT_MARKER.to_owned(),
                secret_base64: STANDARD.encode(secret),
                secret_file: None,
                ca_file: None,
                ca_base64,
                stats: false,
            };
            let uri = profile.to_uri()?;
            entries.push((profile.name, STANDARD.encode(secret)));
            write_entries(&secrets_file, &entries)?;
            Ok(Some(uri))
        }
        ClientAction::List { secrets_file } => {
            let entries = entries(&secrets_file)?;
            if entries.is_empty() {
                println!("No clients found.");
            } else {
                for (label, _) in entries {
                    println!("{label}");
                }
            }
            Ok(None)
        }
        ClientAction::Remove {
            label,
            secrets_file,
        } => {
            let _lock = lock_file(&secrets_file)?;
            KeyRing::from_file(&secrets_file)?;
            let mut entries = entries(&secrets_file)?;
            let original_len = entries.len();
            entries.retain(|(existing, _)| existing != &label);
            ensure!(
                entries.len() != original_len,
                "client {label:?} was not found"
            );
            write_entries(&secrets_file, &entries)?;
            println!("Client {label:?} revoked. New connections will reject this key.");
            Ok(None)
        }
    }
}
