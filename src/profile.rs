use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 9443;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Profile {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub server_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
    #[serde(default)]
    pub stats: bool,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn valid_token(value: &str, extra: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c) || extra.contains(c))
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        ensure!(valid_token(&self.name, ""), "profile name may contain only letters, digits, '.', '_' and '-'");
        ensure!(valid_token(&self.host, ":"), "host may contain only letters, digits, '.', '_', '-', ':'");
        ensure!(valid_token(&self.server_name, ""), "server name may contain only letters, digits, '.', '_' and '-'");
        ensure!(self.port > 0, "port must be positive");
        Ok(())
    }

    /// Raw secret bytes, at least 32 of them.
    pub fn secret(&self) -> Result<Vec<u8>> {
        let bytes = if !self.secret_base64.is_empty() {
            BASE64
                .decode(self.secret_base64.as_bytes())
                .context("profile secret is not valid base64")?
        } else if let Some(path) = &self.secret_file {
            fs::read(path).with_context(|| format!("reading {}", path.display()))?
        } else {
            bail!("profile has no secret");
        };
        ensure!(bytes.len() >= 32, "secret must be at least 32 bytes");
        Ok(bytes)
    }

    pub fn to_uri(&self) -> Result<String> {
        self.validate()?;
        let secret = if !self.secret_base64.is_empty() {
            self.secret_base64.clone()
        } else if let Some(path) = &self.secret_file {
            BASE64.encode(fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        } else {
            bail!("profile has no secret to export");
        };
        Ok(format!(
            "sslim://{}:{}?sni={}&k={}&n={}",
            self.host, self.port, self.server_name, secret, self.name
        ))
    }

    pub fn from_uri(uri: &str) -> Result<Self> {
        let rest = uri
            .strip_prefix("sslim://")
            .with_context(|| format!("unsupported URI scheme in {uri:?}"))?;
        let (authority, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse::<u16>().context("invalid port in URI")?,
            ),
            None => (authority, DEFAULT_PORT),
        };
        let mut sni = String::new();
        let mut key = String::new();
        let mut label = String::new();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .with_context(|| format!("invalid URI parameter {pair:?}"))?;
            match name {
                "sni" => sni = value.to_owned(),
                "k" => key = value.to_owned(),
                "n" => label = value.to_owned(),
                _ => {}
            }
        }
        let profile = Self {
            name: if label.is_empty() { host.to_owned() } else { label },
            host: host.to_owned(),
            port,
            server_name: if sni.is_empty() { host.to_owned() } else { sni },
            secret_base64: key,
            secret_file: None,
            ca_file: None,
            stats: false,
        };
        ensure!(!profile.secret_base64.is_empty(), "URI is missing the secret parameter (k=)");
        profile.validate()?;
        Ok(profile)
    }
}

fn base_dir() -> Result<PathBuf> {
    let root = if cfg!(windows) {
        PathBuf::from(
            std::env::var("APPDATA").context("APPDATA is not set")?,
        )
    } else {
        PathBuf::from(std::env::var("HOME").context("HOME is not set")?).join(".config")
    };
    Ok(root.join("sslim"))
}

pub fn profiles_dir() -> Result<PathBuf> {
    let dir = base_dir()?.join("profiles");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn profile_path(name: &str) -> Result<PathBuf> {
    ensure!(
        valid_token(name, ""),
        "profile name may contain only letters, digits, '.', '_' and '-'"
    );
    Ok(profiles_dir()?.join(format!("{name}.toml")))
}

pub fn save(profile: &Profile) -> Result<PathBuf> {
    profile.validate()?;
    let path = profile_path(&profile.name)?;
    fs::write(&path, toml::to_string_pretty(profile)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn load(name: &str) -> Result<Profile> {
    let path = profile_path(name)?;
    let text = fs::read_to_string(&path)
        .with_context(|| format!("profile {name:?} not found ({})", path.display()))?;
    toml::from_str(&text).with_context(|| format!("profile {name:?} is not valid TOML"))
}

pub fn list() -> Result<Vec<Profile>> {
    let dir = profiles_dir()?;
    let mut profiles = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(profile) = toml::from_str::<Profile>(&text) {
                    profiles.push(profile);
                }
            }
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

pub fn remove(name: &str) -> Result<bool> {
    let path = profile_path(name)?;
    Ok(fs::remove_file(&path).is_ok())
}

/// The profile to connect with when no explicit one is named: the stored
/// last-used choice, or the only profile present.
pub fn default_profile() -> Result<Option<Profile>> {
    let last = base_dir()?.join("blade.last");
    if let Ok(name) = fs::read_to_string(&last) {
        if let Ok(profile) = load(name.trim()) {
            return Ok(Some(profile));
        }
    }
    let profiles = list()?;
    match profiles.as_slice() {
        [profile] => Ok(Some(profile.clone())),
        [] => Ok(None),
        _ => {
            bail!("several profiles exist; choose one with --profile <name> (see `sslim blade list`)")
        }
    }
}

pub fn set_last(name: &str) -> Result<()> {
    ensure!(profile_path(name).is_ok(), "invalid profile name");
    let path = base_dir()?.join("blade.last");
    fs::write(&path, format!("{name}\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        Profile {
            name: "home".into(),
            host: "vpn.example.com".into(),
            port: 9443,
            server_name: "vpn.example.com".into(),
            secret_base64: BASE64.encode([0x5au8; 32]),
            secret_file: None,
            ca_file: None,
            stats: false,
        }
    }

    #[test]
    fn uri_round_trip() {
        let profile = sample();
        let uri = profile.to_uri().unwrap();
        assert!(uri.starts_with("sslim://vpn.example.com:9443?sni=vpn.example.com&k="));
        let parsed = Profile::from_uri(&uri).unwrap();
        assert_eq!(parsed.name, "home");
        assert_eq!(parsed.host, profile.host);
        assert_eq!(parsed.port, profile.port);
        assert_eq!(parsed.server_name, profile.server_name);
        assert_eq!(parsed.secret().unwrap(), vec![0x5au8; 32]);
    }

    #[test]
    fn uri_requires_secret_and_scheme() {
        assert!(Profile::from_uri("https://vpn.example.com").is_err());
        assert!(
            Profile::from_uri("sslim://vpn.example.com:9443?sni=vpn.example.com").is_err(),
            "URI without k= must be rejected"
        );
    }

    #[test]
    fn uri_defaults() {
        let uri = format!("sslim://vpn.example.com?k={}", BASE64.encode([7u8; 32]));
        let parsed = Profile::from_uri(&uri).unwrap();
        assert_eq!(parsed.port, DEFAULT_PORT);
        assert_eq!(parsed.server_name, "vpn.example.com");
        assert_eq!(parsed.name, "vpn.example.com");
        assert_eq!(parsed.secret().unwrap(), vec![7u8; 32]);
    }

    #[test]
    fn names_reject_path_traversal() {
        assert!(profile_path("../evil").is_err());
        assert!(profile_path("a/b").is_err());
        assert!(profile_path("ok.name-1_x").is_ok());
    }

    #[test]
    fn secret_file_is_read_when_base64_is_absent() {
        let mut profile = sample();
        profile.secret_base64.clear();
        profile.secret_base64.push_str("not-base64!!");
        assert!(profile.secret().is_err());
    }
}
