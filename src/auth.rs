use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};

pub fn read_key(path: &Path) -> Result<hmac::Key> {
    let secret = std::fs::read(path)?;
    ensure!(secret.len() >= 32, "secret must be at least 32 bytes");
    Ok(key_from_bytes(&secret))
}

pub fn key_from_bytes(secret: &[u8]) -> hmac::Key {
    hmac::Key::new(hmac::HMAC_SHA256, secret)
}

pub fn generate_secret() -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("randomness unavailable"))?;
    Ok(bytes)
}

/// Server-side set of accepted secrets. Each client holds one labelled secret;
/// `--secret-file` produces a ring with a single "default" entry, `--secrets-file`
/// produces one entry per `label:base64` line.
pub struct KeyRing {
    keys: Vec<(String, hmac::Key)>,
}

impl KeyRing {
    pub fn single(path: &Path) -> Result<Self> {
        Ok(Self {
            keys: vec![("default".to_owned(), read_key(path)?)],
        })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut keys = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (label, secret) = line
                .split_once(':')
                .with_context(|| format!("{}:{}: expected label:base64", path.display(), number + 1))?;
            let label = label.trim();
            ensure!(
                !label.is_empty()
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
                "{}:{}: label may contain only letters, digits, '.', '_' and '-'",
                path.display(),
                number + 1
            );
            let secret = BASE64
                .decode(secret.trim())
                .with_context(|| format!("{}:{}: secret is not valid base64", path.display(), number + 1))?;
            ensure!(
                secret.len() >= 32,
                "{}:{}: secret must be at least 32 bytes",
                path.display(),
                number + 1
            );
            ensure!(
                keys.iter().all(|(existing, _)| existing != label),
                "{}:{}: duplicate label {label:?}",
                path.display(),
                number + 1
            );
            keys.push((label.to_owned(), key_from_bytes(&secret)));
        }
        Ok(Self { keys })
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Authenticate a transport-bound proof from QUIC v3. The exporter is
    /// already unique to the verified TLS connection; no shared plaintext
    /// challenge or replayable application token is used.
    pub fn verify_quic_exporter(&self, exporter: &[u8], proof: &[u8]) -> bool {
        proof.len() == 32
            && self.keys.iter().any(|(_, key)| ring::hmac::verify(key, exporter, proof).is_ok())
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Validate the presented credentials and identify the matching secret.
    /// Returns the label and the parsed time/nonce for replay bookkeeping.
    pub fn verify<'a>(
        &'a self,
        time: &str,
        nonce: &str,
        tag: &str,
    ) -> Option<Verified<'a>> {
        let (parsed_time, nonce_bytes, tag_bytes) = parse_credentials(time, nonce, tag)?;
        for (label, key) in &self.keys {
            if hmac::verify(key, &message(parsed_time, nonce), &tag_bytes).is_ok() {
                return Some(Verified {
                    label: label.as_str(),
                    time: parsed_time,
                    nonce: nonce_bytes,
                });
            }
        }
        None
    }
}

pub struct Verified<'a> {
    pub label: &'a str,
    pub time: u64,
    pub nonce: [u8; 16],
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn message(time: u64, nonce: &str) -> Vec<u8> {
    format!("vpn-bot-lab-transport-v1\0{time}\0{nonce}").into_bytes()
}

pub fn credentials(key: &hmac::Key) -> Result<(u64, String, String)> {
    let mut bytes = [0; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("randomness unavailable"))?;
    let nonce = hex::encode(bytes);
    let now = timestamp();
    let tag = hex::encode(hmac::sign(key, &message(now, &nonce)));
    Ok((now, nonce, tag))
}

fn lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parse and structurally validate presented credentials; HMAC matching
/// happens separately in `KeyRing::verify`.
fn parse_credentials(time: &str, nonce: &str, tag: &str) -> Option<(u64, [u8; 16], [u8; 32])> {
    if time.len() > 12
        || !time.bytes().all(|b| b.is_ascii_digit())
        || !lowercase_hex(nonce)
        || !lowercase_hex(tag)
    {
        return None;
    }
    let mut nonce_bytes = [0; 16];
    let mut tag_bytes = [0; 32];
    hex::decode_to_slice(nonce, &mut nonce_bytes).ok()?;
    hex::decode_to_slice(tag, &mut tag_bytes).ok()?;
    let parsed_time = time.parse::<u64>().ok()?;
    if timestamp().abs_diff(parsed_time) > 60 {
        return None;
    }
    Some((parsed_time, nonce_bytes, tag_bytes))
}

#[derive(Default)]
pub struct ReplayGuard {
    nonces: HashSet<[u8; 16]>,
    expirations: VecDeque<(Instant, [u8; 16])>,
}

impl ReplayGuard {
    /// Record a nonce once HMAC verification has already succeeded. A client
    /// clock at +60 s can leave a request valid for another 120 s, so nonces
    /// are kept slightly longer than the timestamp window.
    pub fn record(&mut self, nonce: [u8; 16]) -> bool {
        let now = Instant::now();
        while self
            .expirations
            .front()
            .is_some_and(|(expiry, _)| *expiry <= now)
        {
            let (_, old_nonce) = self.expirations.pop_front().expect("front exists");
            self.nonces.remove(&old_nonce);
        }
        if self.nonces.len() >= 4096 || !self.nonces.insert(nonce) {
            return false;
        }
        self.expirations
            .push_back((now + Duration::from_secs(121), nonce));
        true
    }
}

/// Write a freshly generated secret file with restrictive permissions.
pub fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        if path.exists() {
            anyhow::bail!("{} already exists", path.display());
        }
        std::fs::write(path, bytes).with_context(|| format!("creating {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_with(labels: &[&str]) -> KeyRing {
        KeyRing {
            keys: labels
                .iter()
                .map(|label| ((*label).to_owned(), key_from_bytes(label.as_bytes())))
                .collect(),
        }
    }

    #[test]
    fn identifies_the_matching_label_and_rejects_the_rest() {
        let ring = ring_with(&["alice", "bob"]);
        let key = key_from_bytes(b"alice");
        let (time, nonce, tag) = credentials(&key).unwrap();
        let verified = ring.verify(&time.to_string(), &nonce, &tag).unwrap();
        assert_eq!(verified.label, "alice");

        let wrong = key_from_bytes(b"eve");
        let (time, nonce, tag) = credentials(&wrong).unwrap();
        assert!(ring.verify(&time.to_string(), &nonce, &tag).is_none());
    }

    #[test]
    fn replayed_nonce_is_recorded_once() {
        let ring = ring_with(&["alice"]);
        let key = key_from_bytes(b"alice");
        let (time, nonce, tag) = credentials(&key).unwrap();
        let verified = ring.verify(&time.to_string(), &nonce, &tag).unwrap();
        let mut guard = ReplayGuard::default();
        assert!(guard.record(verified.nonce));
        assert!(!guard.record(verified.nonce), "same nonce must not pass twice");
    }

    #[test]
    fn stale_timestamp_is_rejected() {
        let ring = ring_with(&["alice"]);
        let key = key_from_bytes(b"alice");
        let (time, nonce, tag) = credentials(&key).unwrap();
        let stale = time - 3600;
        assert!(ring.verify(&stale.to_string(), &nonce, &tag).is_none());
    }

    #[test]
    fn secrets_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("sslim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.txt");
        std::fs::write(
            &path,
            format!(
                "# clients\nalice:{}\nbob:{}\n",
                BASE64.encode([1u8; 32]),
                BASE64.encode([2u8; 32])
            ),
        )
        .unwrap();
        let ring = KeyRing::from_file(&path).unwrap();
        assert_eq!(ring.len(), 2);
        assert!(ring.verify("1", &hex::encode([0u8; 16]), &hex::encode([0u8; 32])).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
