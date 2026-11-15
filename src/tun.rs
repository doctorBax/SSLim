use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{Result, ensure};
use tokio::io::unix::AsyncFd;

use crate::protocol::{append_packet, valid_ip};

pub struct Tun(AsyncFd<File>);

impl Tun {
    pub fn open(name: &str) -> Result<Self> {
        ensure!(
            !name.is_empty()
                && name.len() < libc::IFNAMSIZ
                && name.is_ascii()
                && !name.contains('\0'),
            "invalid TUN interface name"
        );
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/net/tun")?;
        // SAFETY: ifreq has an all-zero valid representation.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (target, source) in request.ifr_name.iter_mut().zip(name.bytes()) {
            *target = source as libc::c_char;
        }
        request.ifr_ifru.ifru_flags = (libc::IFF_TUN | libc::IFF_NO_PI) as libc::c_short;
        // SAFETY: the file owns an open TUN fd and request matches the kernel's ifreq layout.
        if unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF as _, &mut request) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: File owns the nonblocking fd exclusively for its registration lifetime.
        Ok(Self(unsafe { AsyncFd::register(file) }?))
    }

    pub async fn batch(&self, packet: &mut [u8], frames: &mut Vec<u8>) -> io::Result<()> {
        loop {
            let mut ready = self.0.readable().await?;
            for _ in 0..32 {
                match ready.try_io(|fd| fd.get_ref().read(packet)) {
                    Ok(Ok(0)) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                    Ok(Ok(len)) => {
                        if valid_ip(&packet[..len]) {
                            append_packet(frames, &packet[..len]);
                        }
                    }
                    Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Ok(Err(error)) => return Err(error),
                    Err(_) => break,
                }
                if frames.len() >= 32 * 1024 {
                    break;
                }
            }
            if !frames.is_empty() {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    }

    pub async fn write(&self, packet: &[u8]) -> io::Result<()> {
        loop {
            let mut ready = self.0.writable().await?;
            match ready.try_io(|fd| fd.get_ref().write(packet)) {
                Ok(Ok(len)) if len == packet.len() => return Ok(()),
                Ok(Ok(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "partial TUN write",
                    ));
                }
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => continue,
            }
        }
    }
}
