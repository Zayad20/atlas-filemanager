//! Original streaming FLAC encoder written against RFC 9639.
//! Independent channels, constant/verbatim/fixed prediction and Rice2 residuals.
//! No FFmpeg or libFLAC code, dependency or process is used.
use super::{pcm, Command};
use anyhow::{ensure, Context, Result};
use md5::{Digest, Md5};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const BLOCK: usize = 4096;

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    pending: u8,
    used: u32,
}

impl Bits {
    fn put(&mut self, value: u64, mut count: u32) {
        while count > 0 {
            let take = count.min(8 - self.used);
            count -= take;
            self.pending = ((u16::from(self.pending) << take)
                | ((value >> count) & ((1 << take) - 1)) as u16) as u8;
            self.used += take;
            if self.used == 8 {
                self.bytes.push(self.pending);
                self.pending = 0;
                self.used = 0;
            }
        }
    }

    fn unary(&mut self, mut zeros: u64) {
        if self.used != 0 {
            let take = zeros.min(u64::from(8 - self.used));
            self.put(0, take as u32);
            zeros -= take;
        }
        self.bytes
            .resize(self.bytes.len() + (zeros / 8) as usize, 0);
        self.put(0, (zeros % 8) as u32);
        self.put(1, 1);
    }

    fn align(&mut self) {
        if self.used != 0 {
            self.put(0, 8 - self.used);
        }
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |mut crc, byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x80 != 0 { 0x07 } else { 0 };
        }
        crc
    })
}

const CRC16: [u16; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = (index as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x8005 } else { 0 };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |crc, byte| {
        (crc << 8) ^ CRC16[usize::from((crc >> 8) as u8 ^ byte)]
    })
}

// FLAC uses the original UTF-8 integer coding (including 5/6-byte integers).
fn frame_number(number: u32, bytes: &mut Vec<u8>) {
    if number < 128 {
        bytes.push(number as u8);
        return;
    }
    let length = (2..=6)
        .find(|length| u64::from(number) < (1_u64 << (5 * length + 1)))
        .expect("FLAC frame number is bounded by the total sample count");
    bytes.push((0xff_u8 << (8 - length)) | (number >> (6 * (length - 1))) as u8);
    for shift in (0..length - 1).rev() {
        bytes.push(0x80 | ((number >> (6 * shift)) & 0x3f) as u8);
    }
}

fn residual(samples: &[i64], index: usize, order: usize) -> i64 {
    let prediction = match order {
        0 => 0,
        1 => samples[index - 1],
        2 => 2 * samples[index - 1] - samples[index - 2],
        3 => 3 * samples[index - 1] - 3 * samples[index - 2] + samples[index - 3],
        4 => {
            4 * samples[index - 1] - 6 * samples[index - 2] + 4 * samples[index - 3]
                - samples[index - 4]
        }
        _ => unreachable!(),
    };
    samples[index] - prediction
}

fn fold(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

#[derive(Debug, PartialEq, Eq)]
enum Coding {
    Constant,
    Verbatim,
    Fixed { order: usize, rice: u32 },
}

fn choose(samples: &[i64], depth: u32, scratch: &mut Vec<u64>) -> Coding {
    if samples.iter().all(|sample| *sample == samples[0]) {
        return Coding::Constant;
    }
    let mut best = Coding::Verbatim;
    let mut cost = 8 + samples.len() as u64 * u64::from(depth);
    for order in 0..=4.min(samples.len() - 1) {
        scratch.clear();
        let mut sum = 0_u64;
        for index in order..samples.len() {
            let value = residual(samples, index, order);
            // RFC 9639 §9.2.7 excludes i32::MIN as well as larger residuals.
            if !(-i64::from(i32::MAX)..=i64::from(i32::MAX)).contains(&value) {
                break;
            }
            let folded = fold(value);
            sum += folded;
            scratch.push(folded);
        }
        if scratch.len() != samples.len() - order {
            continue;
        }
        let mean = sum / scratch.len() as u64;
        let center = (64 - mean.leading_zeros()).saturating_sub(1).min(30);
        for rice in center.saturating_sub(2)..=(center + 2).min(30) {
            let bits = 8
                + order as u64 * u64::from(depth)
                + 11
                + scratch
                    .iter()
                    .map(|value| (value >> rice) + 1 + u64::from(rice))
                    .sum::<u64>();
            if bits < cost {
                cost = bits;
                best = Coding::Fixed { order, rice };
            }
        }
    }
    best
}

fn subframe(bits: &mut Bits, samples: &[i64], depth: u32, scratch: &mut Vec<u64>) {
    match choose(samples, depth, scratch) {
        Coding::Constant => {
            bits.put(0, 8);
            bits.put(samples[0] as u64, depth);
        }
        Coding::Verbatim => {
            bits.put(2, 8);
            for sample in samples {
                bits.put(*sample as u64, depth);
            }
        }
        Coding::Fixed { order, rice } => {
            bits.put(((8 + order) << 1) as u64, 8);
            for sample in &samples[..order] {
                bits.put(*sample as u64, depth);
            }
            bits.put(1, 2); // Rice2: five-bit parameters.
            bits.put(0, 4); // One partition.
            bits.put(u64::from(rice), 5);
            for index in order..samples.len() {
                let value = fold(residual(samples, index, order));
                bits.unary(value >> rice);
                bits.put(value, rice);
            }
        }
    }
}

pub(super) fn convert(command: &Command, output: &Path, cancel: &AtomicBool) -> Result<()> {
    let (mut input, pcm) = pcm::open(&command.source)?;
    ensure!(pcm.channels <= 8, "FLAC supports at most eight channels");
    let width = usize::from(pcm.bits / 8);
    let channels = usize::from(pcm.channels);
    let frame_bytes = width * channels;
    let total = pcm.bytes / frame_bytes as u64;
    ensure!(
        total < (1_u64 << 36),
        "FLAC exceeds its 36-bit sample count limit"
    );
    ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
    let mut writer = BufWriter::new(File::create(output)?);
    writer.write_all(b"fLaC\x80\0\0\x22")?;
    writer.write_all(&(BLOCK as u16).to_be_bytes())?;
    writer.write_all(&(BLOCK as u16).to_be_bytes())?;
    writer.write_all(&[0; 6])?; // Frame sizes are patched after encoding.
    let info = (u64::from(pcm.rate) << 44)
        | (u64::from(pcm.channels - 1) << 41)
        | (u64::from(pcm.bits - 1) << 36)
        | total;
    writer.write_all(&info.to_be_bytes())?;
    writer.write_all(&[0; 16])?; // MD5 is patched after the last frame.
    let mut digest = Md5::new();
    let mut buffer = vec![0_u8; BLOCK * frame_bytes];
    let mut samples = Vec::with_capacity(BLOCK);
    let mut scratch = Vec::with_capacity(BLOCK);
    let mut bits = Bits::default();
    let mut remaining = total;
    let mut number = 0;
    let mut min_frame = u32::MAX;
    let mut max_frame = 0;
    while remaining > 0 {
        ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
        let count = remaining.min(BLOCK as u64) as usize;
        let data = &mut buffer[..count * frame_bytes];
        input.read_exact(data).context("Truncated PCM samples")?;
        // The FLAC MD5 uses signed little-endian, interleaved sample bytes.
        for sample in data.chunks_exact_mut(width) {
            if width == 1 && pcm.unsigned8 {
                sample[0] ^= 0x80;
            } else if width > 1 && !pcm.little {
                sample.reverse();
            }
        }
        digest.update(&*data);
        bits.bytes.clear();
        bits.bytes
            .extend_from_slice(&[0xff, 0xf8, 0x70, ((channels - 1) << 4) as u8]);
        frame_number(number, &mut bits.bytes);
        bits.bytes
            .extend_from_slice(&((count - 1) as u16).to_be_bytes());
        bits.bytes.push(crc8(&bits.bytes));
        for channel in 0..channels {
            samples.clear();
            for frame in data.chunks_exact(frame_bytes) {
                let mut bytes = [0; 4];
                bytes[..width].copy_from_slice(&frame[channel * width..(channel + 1) * width]);
                let value = (i32::from_le_bytes(bytes) << (32 - pcm.bits)) >> (32 - pcm.bits);
                samples.push(i64::from(value));
            }
            subframe(&mut bits, &samples, u32::from(pcm.bits), &mut scratch);
        }
        bits.align();
        let crc = crc16(&bits.bytes);
        bits.bytes.extend_from_slice(&crc.to_be_bytes());
        min_frame = min_frame.min(bits.bytes.len() as u32);
        max_frame = max_frame.max(bits.bytes.len() as u32);
        writer.write_all(&bits.bytes)?;
        number += 1;
        remaining -= count as u64;
    }
    ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
    writer.seek(SeekFrom::Start(12))?;
    writer.write_all(&min_frame.to_be_bytes()[1..])?;
    writer.write_all(&max_frame.to_be_bytes()[1..])?;
    writer.seek(SeekFrom::Start(26))?;
    writer.write_all(&digest.finalize())?;
    writer.flush()?;
    Ok(())
}
