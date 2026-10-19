//! Local SDR video to MP4 using Windows decoders and H.264/AAC encoders.
//! Streams stay interleaved; one shared time origin preserves audio/video sync.
use super::{audio_type, Session};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{Error, Interface, HRESULT, PCWSTR};
use windows::Win32::Media::MediaFoundation::*;

fn invalid(message: &str) -> Error {
    Error::new(HRESULT(0x80070057_u32 as i32), message)
}

fn encode(
    source: &Path,
    output: &Path,
    quality: u8,
    max_edge: Option<u32>,
    cancel: &AtomicBool,
) -> windows::core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::new(HRESULT(0x800704c7_u32 as i32), "Cancelled"));
    }
    let session = Session::start()?;
    let api = &session.api;
    let attributes = api.attributes()?;
    unsafe {
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
    }
    let input: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let output: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut raw = null_mut();
    unsafe {
        (api.create_reader)(PCWSTR(input.as_ptr()), attributes.as_raw(), &mut raw).ok()?;
    }
    let reader = unsafe { IMFSourceReader::from_raw(raw) };
    unsafe {
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
    }
    let mut video = None;
    let mut audio = None;
    for index in 0..32 {
        let native = match unsafe { reader.GetNativeMediaType(index, 0) } {
            Ok(native) => native,
            Err(error) if error.code() == MF_E_INVALIDSTREAMNUMBER => break,
            Err(error) => return Err(error),
        };
        match unsafe { native.GetGUID(&MF_MT_MAJOR_TYPE)? } {
            major if major == MFMediaType_Video && video.is_none() => video = Some((index, native)),
            major if major == MFMediaType_Audio && audio.is_none() => audio = Some((index, native)),
            _ => {}
        }
    }
    let (video_index, native) = video.ok_or_else(|| invalid("Source contains no video stream"))?;
    let size = unsafe { native.GetUINT64(&MF_MT_FRAME_SIZE)? };
    let (width, height) = ((size >> 32) as u32, size as u32);
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 33_554_432
    {
        return Err(invalid("Invalid or excessive video dimensions"));
    }
    let rotation = unsafe { native.GetUINT32(&MF_MT_VIDEO_ROTATION) }.unwrap_or(0);
    if rotation != 0 {
        return Err(invalid(
            "Rotated video requires an optional engine to preserve orientation",
        ));
    }
    let transfer = unsafe { native.GetUINT32(&MF_MT_TRANSFER_FUNCTION) }.unwrap_or(0);
    if [
        MFVideoTransFunc_2084.0 as u32,
        MFVideoTransFunc_HLG.0 as u32,
    ]
    .contains(&transfer)
    {
        return Err(invalid(
            "HDR video requires an optional engine with explicit tone mapping",
        ));
    }
    let interlace = unsafe { native.GetUINT32(&MF_MT_INTERLACE_MODE) }.unwrap_or(0);
    if interlace != 0 && !(2..=7).contains(&interlace) {
        return Err(invalid("Invalid video interlace mode"));
    }
    let rate = unsafe { native.GetUINT64(&MF_MT_FRAME_RATE)? };
    let (numerator, denominator) = ((rate >> 32) as u32, rate as u32);
    if numerator == 0 || denominator == 0 || u64::from(numerator) > u64::from(denominator) * 120 {
        return Err(invalid("Video requires a valid frame rate up to 120 fps"));
    }
    let edge = max_edge.unwrap_or(width.max(height)).clamp(2, 4096);
    let scale = f64::from(edge.min(width.max(height))) / f64::from(width.max(height));
    let out_width = ((f64::from(width) * scale) as u32 / 2 * 2).max(2);
    let out_height = ((f64::from(height) * scale) as u32 / 2 * 2).max(2);
    if max_edge.is_none() && (out_width != width || out_height != height) {
        return Err(invalid("H.264 needs even dimensions up to 4096; select a maximum size or use an optional engine"));
    }
    let aspect =
        unsafe { native.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO) }.unwrap_or((1_u64 << 32) | 1);
    let pcm_video = api.media_type()?;
    unsafe {
        pcm_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        pcm_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        pcm_video.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(out_width) << 32) | u64::from(out_height),
        )?;
        pcm_video.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        pcm_video.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, aspect)?;
        pcm_video.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        reader.SetStreamSelection(video_index, true)?;
        reader.SetCurrentMediaType(video_index, None, &pcm_video)?;
    }
    let actual_video = unsafe { reader.GetCurrentMediaType(video_index)? };
    let output_video = api.media_type()?;
    let factor = if quality >= 95 {
        0.18
    } else if quality >= 85 {
        0.12
    } else {
        0.08
    };
    let bitrate = (f64::from(out_width) * f64::from(out_height) * f64::from(numerator)
        / f64::from(denominator)
        * factor)
        .clamp(250_000.0, 30_000_000.0) as u32;
    unsafe {
        output_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        output_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        output_video.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(out_width) << 32) | u64::from(out_height),
        )?;
        output_video.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        output_video.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, aspect)?;
        output_video.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        output_video.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        output_video.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
        for key in [
            MF_MT_VIDEO_PRIMARIES,
            MF_MT_TRANSFER_FUNCTION,
            MF_MT_YUV_MATRIX,
            MF_MT_VIDEO_NOMINAL_RANGE,
        ] {
            if let Ok(value) = native.GetUINT32(&key) {
                output_video.SetUINT32(&key, value)?;
            }
        }
    }
    let mut raw = null_mut();
    let writer_attributes = api.attributes()?;
    unsafe {
        // Prevent encoder reordering from adding a frame of video-only delay.
        // The MP4 writer otherwise offsets H.264 while AAC keeps its origin.
        writer_attributes.SetUINT32(&MF_LOW_LATENCY, 1)?;
        (api.create_writer)(
            PCWSTR(output.as_ptr()),
            null_mut(),
            writer_attributes.as_raw(),
            &mut raw,
        )
        .ok()?;
    }
    let writer = unsafe { IMFSinkWriter::from_raw(raw) };
    let video_output = unsafe { writer.AddStream(&output_video)? };
    unsafe {
        writer.SetInputMediaType(video_output, &actual_video, None)?;
    }
    let mut streams = vec![(video_index, video_output, false, false)];
    if let Some((index, native)) = audio {
        let channels = unsafe { native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? };
        let source_rate = unsafe { native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? };
        if channels == 0 || channels > 32 || source_rate == 0 || source_rate > 768_000 {
            return Err(invalid("Invalid video audio track"));
        }
        let channels = channels.min(2);
        let rate = if [44100, 48000].contains(&source_rate) {
            source_rate
        } else {
            44100
        };
        let pcm = audio_type(
            api,
            MFAudioFormat_PCM,
            channels,
            rate,
            Some(16),
            rate * channels * 2,
        )?;
        unsafe {
            reader.SetStreamSelection(index, true)?;
            reader.SetCurrentMediaType(index, None, &pcm)?;
        }
        let actual = unsafe { reader.GetCurrentMediaType(index)? };
        let bitrate = if quality >= 95 {
            256_000
        } else if quality >= 85 {
            192_000
        } else {
            128_000
        };
        let aac = audio_type(
            api,
            MFAudioFormat_AAC,
            channels,
            rate,
            None,
            bitrate / 8 / if channels == 1 { 2 } else { 1 },
        )?;
        let output = unsafe { writer.AddStream(&aac)? };
        unsafe {
            writer.SetInputMediaType(output, &actual, None)?;
        }
        streams.push((index, output, false, false));
    }
    unsafe {
        writer.BeginWriting()?;
    }
    let mut origin = None;
    while streams.iter().any(|stream| !stream.2) {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::new(HRESULT(0x800704c7_u32 as i32), "Cancelled"));
        }
        let (mut index, mut flags, mut timestamp, mut sample) = (0, 0, 0, None);
        unsafe {
            reader.ReadSample(
                MF_SOURCE_READER_ANY_STREAM.0 as u32,
                0,
                Some(&mut index),
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;
        }
        if flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0) as u32
            != 0
        {
            return Err(invalid("Video decoder failed or changed format mid-stream"));
        }
        let stream = streams
            .iter_mut()
            .find(|stream| stream.0 == index)
            .ok_or_else(|| invalid("Decoder returned an unselected stream"))?;
        if let Some(sample) = sample {
            let base = *origin.get_or_insert(timestamp);
            unsafe {
                sample.SetSampleTime(timestamp.saturating_sub(base).max(0))?;
                if stream.0 == video_index && sample.GetSampleDuration().is_err() {
                    sample.SetSampleDuration(
                        10_000_000_i64 * i64::from(denominator) / i64::from(numerator),
                    )?;
                }
                writer.WriteSample(stream.1, &sample)?;
            }
            stream.3 = true;
        } else if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 {
            if let Some(base) = origin {
                unsafe {
                    writer.SendStreamTick(stream.1, timestamp.saturating_sub(base).max(0))?;
                }
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            stream.2 = true;
            unsafe {
                reader.SetStreamSelection(stream.0, false)?;
            }
        }
    }
    if streams.iter().any(|stream| !stream.3) {
        return Err(invalid(
            "Selected video/audio stream has no decodable samples",
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::new(HRESULT(0x800704c7_u32 as i32), "Cancelled"));
    }
    unsafe {
        writer.Finalize()?;
    }
    Ok(())
}

pub fn convert_video(
    source: &Path,
    output: &Path,
    quality: u8,
    max_edge: Option<u32>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    encode(source, output, quality, max_edge, cancel).map_err(|error| {
        format!(
            "Windows video conversion: {error}. This file may require an optional video engine."
        )
    })
}
