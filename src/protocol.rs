use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

pub const MAX_PACKET: usize = u16::MAX as usize;
pub const PACKET: u8 = 1;
pub const PING: u8 = 2;
pub const PONG: u8 = 3;
pub const CLOSE: u8 = 4;
pub const PONG_FRAME: &[u8] = &[PONG, 0, 0];

pub fn valid_ip(packet: &[u8]) -> bool {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => {
            let header = usize::from(packet[0] & 15) * 4;
            (20..=packet.len()).contains(&header)
                && usize::from(u16::from_be_bytes([packet[2], packet[3]])) == packet.len()
        }
        Some(6) if packet.len() >= 40 => {
            usize::from(u16::from_be_bytes([packet[4], packet[5]])) + 40 == packet.len()
        }
        _ => false,
    }
}

pub fn append_packet(output: &mut Vec<u8>, packet: &[u8]) {
    output.push(PACKET);
    output.extend_from_slice(&(packet.len() as u16).to_be_bytes());
    output.extend_from_slice(packet);
}

pub struct FrameReader<R> {
    inner: R,
    packet: Box<[u8]>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            packet: vec![0; MAX_PACKET].into_boxed_slice(),
        }
    }

    pub async fn next(&mut self) -> io::Result<(u8, &mut [u8])> {
        let mut header = [0; 3];
        self.inner.read_exact(&mut header).await?;
        let len = usize::from(u16::from_be_bytes([header[1], header[2]]));
        let payload = &mut self.packet[..len];
        match header[0] {
            PACKET => {
                self.inner.read_exact(payload).await?;
                if !valid_ip(payload) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid IP packet",
                    ));
                }
            }
            PING | PONG | CLOSE if len == 0 => {}
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid frame")),
        }
        Ok((header[0], payload))
    }
}

fn checksum(data: &[u8]) -> [u8; 2] {
    let (chunks, remainder) = data.as_chunks::<2>();
    let mut sum: u32 = chunks
        .iter()
        .map(|chunk| u32::from(u16::from_be_bytes(*chunk)))
        .sum();
    if let [last] = remainder {
        sum += u32::from(*last) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    (!(sum as u16)).to_be_bytes()
}

pub fn icmp_reply(packet: &mut [u8]) -> bool {
    if packet[0] >> 4 != 4 {
        return false;
    }
    let header = usize::from(packet[0] & 15) * 4;
    if packet.len() < header + 8
        || packet[9] != 1
        || u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff != 0
        || packet[16..20] != [10, 253, 0, 1]
        || packet[header..header + 2] != [8, 0]
    {
        return false;
    }
    packet[12..20].rotate_left(4);
    packet[8] = 64;
    packet[header] = 0;
    packet[header + 2..header + 4].fill(0);
    let icmp = checksum(&packet[header..]);
    packet[header + 2..header + 4].copy_from_slice(&icmp);
    packet[10..12].fill(0);
    let ip = checksum(&packet[..header]);
    packet[10..12].copy_from_slice(&ip);
    true
}
