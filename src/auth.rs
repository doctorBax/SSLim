use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, ensure};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};

pub fn read_key(path: &Path) -> Result<hmac::Key> {
    let secret = std::fs::read(path)?;
    ensure!(secret.len() >= 32, "secret must be at least 32 bytes");
    Ok(hmac::Key::new(hmac::HMAC_SHA256, &secret))
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn message(timestamp: u64, nonce: &str) -> Vec<u8> {
    format!("vpn-bot-lab-transport-v1\0{timestamp}\0{nonce}").into_bytes()
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

#[derive(Default)]
pub struct ReplayGuard {
    nonces: HashSet<[u8; 16]>,
    expirations: VecDeque<(Instant, [u8; 16])>,
}

impl ReplayGuard {
    pub fn verify(&mut self, key: &hmac::Key, time: &str, nonce: &str, tag: &str) -> bool {
        let mut nonce_bytes = [0; 16];
        let mut tag_bytes = [0; 32];
        if time.len() > 12
            || !time.bytes().all(|b| b.is_ascii_digit())
            || !lowercase_hex(nonce)
            || !lowercase_hex(tag)
            || hex::decode_to_slice(nonce, &mut nonce_bytes).is_err()
            || hex::decode_to_slice(tag, &mut tag_bytes).is_err()
        {
            return false;
        }
        let Ok(time) = time.parse::<u64>() else {
            return false;
        };
        if timestamp().abs_diff(time) > 60
            || hmac::verify(key, &message(time, nonce), &tag_bytes).is_err()
        {
            return false;
        }
        let now = Instant::now();
        while self
            .expirations
            .front()
            .is_some_and(|(expiry, _)| *expiry <= now)
        {
            let (_, old_nonce) = self.expirations.pop_front().expect("front exists");
            self.nonces.remove(&old_nonce);
        }
        if self.nonces.len() >= 4096 || !self.nonces.insert(nonce_bytes) {
            return false;
        }
        self.expirations
            .push_back((now + Duration::from_secs(121), nonce_bytes));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_key_and_replayed_nonce() {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &[0; 32]);
        let wrong_key = hmac::Key::new(hmac::HMAC_SHA256, &[1; 32]);
        let (time, nonce, tag) = credentials(&key).unwrap();
        let time = time.to_string();
        let mut replay = ReplayGuard::default();
        assert!(!replay.verify(&wrong_key, &time, &nonce, &tag));
        assert!(replay.verify(&key, &time, &nonce, &tag));
        assert!(!replay.verify(&key, &time, &nonce, &tag));
    }
}
