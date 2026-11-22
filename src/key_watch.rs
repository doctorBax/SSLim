//! One reader per server; all sessions observe the same revocation snapshot.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::auth::KeyRing;

pub type Identity = (String, [u8; 32]);

#[derive(Clone)]
pub struct Snapshot {
    pub ring: Option<Arc<KeyRing>>,
    pub epochs: HashMap<Identity, u64>,
}

impl Snapshot {
    fn advance(&mut self, ring: Option<Arc<KeyRing>>) {
        let next: HashSet<_> = ring
            .as_ref()
            .map(|r| r.identities().into_iter().collect())
            .unwrap_or_default();
        if let Some(previous) = &self.ring {
            for identity in previous.identities() {
                if !next.contains(&identity) {
                    let epoch = self.epochs.entry(identity).or_default();
                    // Overflow is practically unreachable; fail closed if it occurs.
                    *epoch = epoch
                        .checked_add(1)
                        .expect("key revocation epoch exhausted");
                }
            }
        }
        self.ring = ring;
    }
}

pub type Keys = watch::Receiver<Snapshot>;

pub fn start(ring: KeyRing, path: Option<PathBuf>) -> Keys {
    let mut snapshot = Snapshot {
        ring: Some(Arc::new(ring)),
        epochs: HashMap::new(),
    };
    let (sender, receiver) = watch::channel(snapshot.clone());
    tokio::spawn(async move {
        let Some(path) = path else {
            // Keep a static key source alive until the server exits.
            sender.closed().await;
            return;
        };
        let mut failed = false;
        loop {
            tokio::select! {
                _ = sender.closed() => return,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
            let file = path.clone();
            let loaded = tokio::task::spawn_blocking(move || KeyRing::from_file(&file)).await;
            let keys = match loaded {
                Ok(Ok(ring)) => {
                    failed = false;
                    Some(Arc::new(ring))
                }
                _ => {
                    if !failed {
                        eprintln!(
                            "Client key file unavailable or invalid; closing sessions until repaired"
                        );
                    }
                    failed = true;
                    None
                }
            };
            snapshot.advance(keys);
            if sender.send(snapshot.clone()).is_err() {
                return;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{credentials, key_from_bytes};
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[tokio::test]
    async fn rotation_and_invalid_file_change_the_published_keys() {
        let dir = std::env::temp_dir().join(format!(
            "sslim-watch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clients");
        let initial = [1u8; 32];
        let rotated = [2u8; 32];
        std::fs::write(&path, format!("client:{}\n", STANDARD.encode(initial))).unwrap();
        let ring = KeyRing::from_file(&path).unwrap();
        let (time, nonce, tag) = credentials(&key_from_bytes(&initial)).unwrap();
        let identity = ring
            .verify(&time.to_string(), &nonce, &tag)
            .unwrap()
            .identity;
        let mut rx = start(ring, Some(path.clone()));
        std::fs::write(&path, format!("client:{}\n", STANDARD.encode(rotated))).unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !rx.borrow_and_update()
                .ring
                .as_ref()
                .unwrap()
                .contains_key("client", &identity)
        );
        std::fs::write(&path, "invalid\n").unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(rx.borrow_and_update().ring.is_none());
        std::fs::write(&path, format!("client:{}\n", STANDARD.encode(initial))).unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            rx.borrow_and_update()
                .ring
                .as_ref()
                .unwrap()
                .contains_key("client", &identity)
        );
        drop(rx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn epoch_survives_removed_and_restored_key_between_observations() {
        let key = [7u8; 32];
        let dir = std::env::temp_dir().join(format!(
            "sslim-epoch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clients");
        std::fs::write(&path, format!("client:{}\n", STANDARD.encode(key))).unwrap();
        let ring = Arc::new(KeyRing::from_file(&path).unwrap());
        let identity = ring.identities().pop().unwrap();
        let initial = Snapshot {
            ring: Some(ring.clone()),
            epochs: HashMap::new(),
        };
        let (sender, mut receiver) = watch::channel(initial.clone());
        let mut current = initial;
        current.advance(None);
        sender.send(current.clone()).unwrap();
        current.advance(Some(ring));
        sender.send(current).unwrap();
        let observed = receiver.borrow_and_update();
        assert!(
            observed
                .ring
                .as_ref()
                .unwrap()
                .contains_key(&identity.0, &identity.1)
        );
        assert_eq!(observed.epochs.get(&identity), Some(&1));
        drop(observed);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
