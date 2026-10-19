//! Independent PCM container conversion from the RIFF/WAVE, AIFF and AU formats.
//! No compressed codec, DSP, external process or new dependency is involved.
use super::Command;
use anyhow::{bail, ensure, Context, Result};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug)]
struct Pcm {
    channels: u16,
    bits: u16,
    rate: u32,
    offset: u64,
    bytes: u64,
    little: bool,
    unsigned8: bool,
}

impl Pcm {
    fn validate(self) -> Result<Self> {
        ensure!((1..=32).contains(&self.channels), "PCM needs 1–32 channels");
        ensure!(
            [8, 16, 24, 32].contains(&self.bits),
            "Only 8/16/24/32-bit integer PCM is supported natively"
        );
        ensure!(
            (1..=768_000).contains(&self.rate),
            "Invalid PCM sample rate"
        );
        let frame = u64::from(self.channels) * u64::from(self.bits / 8);
        ensure!(
            self.bytes > 0 && self.bytes % frame == 0,
            "PCM data contains an incomplete or empty sample frame"
        );
        Ok(self)
    }
}

fn read<const N: usize>(file: &mut File) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    file.read_exact(&mut bytes)
        .context("Truncated audio container")?;
    Ok(bytes)
}

fn wave(file: &mut File, length: u64) -> Result<Pcm> {
    let end = u64::from(u32::from_le_bytes(read(file)?)) + 8;
    ensure!(
        read::<4>(file)? == *b"WAVE" && end <= length,
        "Invalid RIFF/WAVE length or signature"
    );
    let mut format = None;
    let mut data = None;
    while file.stream_position()? + 8 <= end {
        let id = read::<4>(file)?;
        let size = u32::from_le_bytes(read(file)?);
        let offset = file.stream_position()?;
        let next = offset + u64::from(size) + u64::from(size % 2);
        ensure!(next <= end, "WAVE chunk exceeds the RIFF boundary");
        match &id {
            b"fmt " => {
                ensure!(
                    format.is_none() && size >= 16,
                    "Invalid or duplicate WAVE format chunk"
                );
                let tag = u16::from_le_bytes(read(file)?);
                let channels = u16::from_le_bytes(read(file)?);
                let rate = u32::from_le_bytes(read(file)?);
                let byte_rate = u32::from_le_bytes(read(file)?);
                let align = u16::from_le_bytes(read(file)?);
                let bits = u16::from_le_bytes(read(file)?);
                if tag == 0xfffe {
                    ensure!(
                        size >= 40 && u16::from_le_bytes(read(file)?) >= 22,
                        "Invalid extensible WAVE format"
                    );
                    let valid_bits = u16::from_le_bytes(read(file)?);
                    let _mask = read::<4>(file)?;
                    let subtype = read::<16>(file)?;
                    ensure!(subtype == [1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113], "This WAVE codec requires an optional audio engine; native conversion accepts integer PCM");
                    ensure!(
                        valid_bits == 0 || valid_bits == bits,
                        "Packed valid-bit WAVE needs an optional audio engine"
                    );
                } else {
                    ensure!(tag == 1, "Compressed or floating-point WAVE needs an optional audio engine; native conversion accepts integer PCM");
                }
                let expected = u32::from(channels) * u32::from(bits / 8);
                ensure!(
                    u32::from(align) == expected
                        && u64::from(byte_rate) == u64::from(rate) * u64::from(expected),
                    "Inconsistent WAVE frame size or byte rate"
                );
                format = Some((channels, bits, rate));
            }
            b"data" => {
                ensure!(
                    data.is_none(),
                    "Multiple WAVE data chunks are not supported"
                );
                data = Some((offset, u64::from(size)));
            }
            _ => {}
        }
        file.seek(SeekFrom::Start(next))?;
    }
    let (channels, bits, rate) = format.context("WAVE has no format chunk")?;
    let (offset, bytes) = data.context("WAVE has no audio data")?;
    Pcm {
        channels,
        bits,
        rate,
        offset,
        bytes,
        little: true,
        unsigned8: true,
    }
    .validate()
}

fn extended_rate(bytes: [u8; 10]) -> Result<u32> {
    let exponent = u16::from_be_bytes([bytes[0], bytes[1]]);
    ensure!(
        exponent & 0x8000 == 0 && exponent != 0x7fff,
        "Invalid AIFF sample rate"
    );
    let mantissa = u64::from_be_bytes(bytes[2..].try_into()?);
    let value = (mantissa as f64) * 2.0_f64.powi(i32::from(exponent) - 16383 - 63);
    ensure!(
        value.is_finite()
            && (1.0..=768_000.0).contains(&value)
            && (value - value.round()).abs() < 0.001,
        "AIFF requires an integer sample rate between 1 and 768000 Hz"
    );
    Ok(value.round() as u32)
}

fn aiff(file: &mut File, length: u64) -> Result<Pcm> {
    let end = u64::from(u32::from_be_bytes(read(file)?)) + 8;
    let kind = read::<4>(file)?;
    ensure!(
        [*b"AIFF", *b"AIFC"].contains(&kind) && end <= length,
        "Invalid AIFF length or signature"
    );
    let mut format = None;
    let mut data = None;
    while file.stream_position()? + 8 <= end {
        let id = read::<4>(file)?;
        let size = u32::from_be_bytes(read(file)?);
        let offset = file.stream_position()?;
        let next = offset + u64::from(size) + u64::from(size % 2);
        ensure!(next <= end, "AIFF chunk exceeds the FORM boundary");
        match &id {
            b"COMM" => {
                ensure!(
                    format.is_none() && size >= 18,
                    "Invalid or duplicate AIFF format chunk"
                );
                let channels = u16::from_be_bytes(read(file)?);
                let frames = u32::from_be_bytes(read(file)?);
                let bits = u16::from_be_bytes(read(file)?);
                let rate = extended_rate(read(file)?)?;
                let little = if kind == *b"AIFC" {
                    ensure!(size >= 22, "Missing AIFC compression type");
                    let compression = read::<4>(file)?;
                    ensure!(
                        [*b"NONE", *b"twos", *b"sowt"].contains(&compression),
                        "Compressed or floating-point AIFC needs an optional audio engine"
                    );
                    compression == *b"sowt"
                } else {
                    false
                };
                format = Some((channels, frames, bits, rate, little));
            }
            b"SSND" => {
                ensure!(
                    data.is_none() && size >= 8,
                    "Invalid or duplicate AIFF audio chunk"
                );
                let skip = u32::from_be_bytes(read(file)?);
                let _block_size = read::<4>(file)?;
                ensure!(skip <= size - 8, "AIFF audio offset exceeds its chunk");
                data = Some((offset + 8 + u64::from(skip), u64::from(size - 8 - skip)));
            }
            _ => {}
        }
        file.seek(SeekFrom::Start(next))?;
    }
    let (channels, frames, bits, rate, little) = format.context("AIFF has no COMM chunk")?;
    let (offset, available) = data.context("AIFF has no SSND chunk")?;
    let bytes = u64::from(frames) * u64::from(channels) * u64::from(bits / 8);
    ensure!(
        bytes <= available,
        "AIFF declares more sample frames than its audio data contains"
    );
    Pcm {
        channels,
        bits,
        rate,
        offset,
        bytes,
        little,
        unsigned8: false,
    }
    .validate()
}

fn au(file: &mut File, length: u64) -> Result<Pcm> {
    let offset = u64::from(u32::from_be_bytes(read(file)?));
    let declared = u32::from_be_bytes(read(file)?);
    let encoding = u32::from_be_bytes(read(file)?);
    let rate = u32::from_be_bytes(read(file)?);
    let channels = u32::from_be_bytes(read(file)?);
    ensure!(offset >= 24 && offset <= length, "Invalid AU audio offset");
    ensure!(
        (2..=5).contains(&encoding),
        "This AU codec requires an optional audio engine; native conversion accepts integer PCM"
    );
    let bytes = if declared == u32::MAX {
        length - offset
    } else {
        u64::from(declared)
    };
    ensure!(
        bytes <= length - offset && channels <= 32,
        "Invalid AU length or channel count"
    );
    Pcm {
        channels: channels as u16,
        bits: ((encoding - 1) * 8) as u16,
        rate,
        offset,
        bytes,
        little: false,
        unsigned8: false,
    }
    .validate()
}

fn header(file: &mut impl Write, pcm: Pcm, extension: &str) -> Result<()> {
    let bytes =
        u32::try_from(pcm.bytes).context("PCM output exceeds the container's 4 GiB limit")?;
    let frame = pcm.channels * (pcm.bits / 8);
    let padded = bytes
        .checked_add(bytes % 2)
        .context("PCM output is too large")?;
    match extension {
        "wav" => {
            file.write_all(b"RIFF")?;
            file.write_all(
                &padded
                    .checked_add(36)
                    .context("WAVE output is too large")?
                    .to_le_bytes(),
            )?;
            file.write_all(b"WAVEfmt \x10\0\0\0\x01\0")?;
            file.write_all(&pcm.channels.to_le_bytes())?;
            file.write_all(&pcm.rate.to_le_bytes())?;
            file.write_all(
                &pcm.rate
                    .checked_mul(u32::from(frame))
                    .context("PCM byte rate is too large")?
                    .to_le_bytes(),
            )?;
            file.write_all(&frame.to_le_bytes())?;
            file.write_all(&pcm.bits.to_le_bytes())?;
            file.write_all(b"data")?;
            file.write_all(&bytes.to_le_bytes())?;
        }
        "aiff" => {
            file.write_all(b"FORM")?;
            file.write_all(
                &padded
                    .checked_add(46)
                    .context("AIFF output is too large")?
                    .to_be_bytes(),
            )?;
            file.write_all(b"AIFFCOMM\0\0\0\x12")?;
            file.write_all(&pcm.channels.to_be_bytes())?;
            file.write_all(&(bytes / u32::from(frame)).to_be_bytes())?;
            file.write_all(&pcm.bits.to_be_bytes())?;
            let highest_bit = 31 - pcm.rate.leading_zeros();
            file.write_all(&((16383 + highest_bit) as u16).to_be_bytes())?;
            file.write_all(&(u64::from(pcm.rate) << (63 - highest_bit)).to_be_bytes())?;
            file.write_all(b"SSND")?;
            file.write_all(
                &bytes
                    .checked_add(8)
                    .context("AIFF audio is too large")?
                    .to_be_bytes(),
            )?;
            file.write_all(&[0; 8])?;
        }
        "au" => {
            file.write_all(b".snd\0\0\0\x18")?;
            ensure!(
                bytes != u32::MAX,
                "AU size collides with its unknown-length marker"
            );
            file.write_all(&bytes.to_be_bytes())?;
            file.write_all(&(u32::from(pcm.bits / 8) + 1).to_be_bytes())?;
            file.write_all(&pcm.rate.to_be_bytes())?;
            file.write_all(&u32::from(pcm.channels).to_be_bytes())?;
        }
        _ => bail!("Unsupported PCM container"),
    }
    Ok(())
}

pub(super) fn convert(command: &Command, output: &Path, cancel: &AtomicBool) -> Result<()> {
    let mut input = File::open(&command.source)?;
    let length = input.metadata()?.len();
    let pcm = match &read::<4>(&mut input)? {
        b"RIFF" => wave(&mut input, length)?,
        b"FORM" => aiff(&mut input, length)?,
        b".snd" => au(&mut input, length)?,
        _ => bail!("Expected a RIFF/WAVE, AIFF or AU container with integer PCM samples"),
    };
    let mut writer = BufWriter::new(File::create(output)?);
    let dst = command.target.extension;
    header(&mut writer, pcm, dst)?;
    input.seek(SeekFrom::Start(pcm.offset))?;
    let mut remaining = pcm.bytes;
    let sample_bytes = usize::from(pcm.bits / 8);
    let mut buffer = [0; 64 * 1024];
    let block = buffer.len() / sample_bytes * sample_bytes;
    while remaining > 0 {
        ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
        let count = remaining.min(block as u64) as usize;
        input
            .read_exact(&mut buffer[..count])
            .context("Truncated PCM samples")?;
        if sample_bytes == 1 && pcm.unsigned8 != (dst == "wav") {
            for sample in &mut buffer[..count] {
                *sample ^= 0x80;
            }
        } else if sample_bytes > 1 && pcm.little != (dst == "wav") {
            for sample in buffer[..count].chunks_exact_mut(sample_bytes) {
                sample.reverse();
            }
        }
        writer.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    if dst != "au" && pcm.bytes % 2 != 0 {
        writer.write_all(&[0])?;
    }
    writer.flush()?;
    Ok(())
}
