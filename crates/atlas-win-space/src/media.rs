//! Atlas's own local transcoder over the Windows Media Foundation APIs.
//! There is no FFmpeg code or binary. MF DLLs are loaded from System32 on demand
//! so Windows N / missing media components never prevent Atlas from starting.
mod video;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
pub use video::convert_video;
use windows::core::{Error, Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioCapabilities {
    pub wave: bool,
    pub mp3: bool,
    pub aac: bool,
    pub h264: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum AudioTarget {
    Wave,
    Mp3,
    Aac,
    M4a,
}

struct Module(HMODULE);
impl Module {
    fn load(name: &str) -> windows::core::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module == 0 {
            Err(Error::from_win32())
        } else {
            Ok(Self(module))
        }
    }
    /// T is the documented ABI of this exact Windows export at each call site.
    unsafe fn symbol<T: Copy>(&self, name: &'static [u8]) -> windows::core::Result<T> {
        let address =
            unsafe { GetProcAddress(self.0, name.as_ptr()) }.ok_or_else(Error::from_win32)?;
        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&address));
        Ok(unsafe { std::mem::transmute_copy(&address) })
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

type CreateType = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateReader = unsafe extern "system" fn(PCWSTR, *mut c_void, *mut *mut c_void) -> HRESULT;
type CreateWriter =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;
type EnumMft = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut *mut c_void,
    *mut u32,
) -> HRESULT;

struct Api {
    start: unsafe extern "system" fn(u32, u32) -> HRESULT,
    stop: unsafe extern "system" fn() -> HRESULT,
    create_type: CreateType,
    create_attributes: CreateAttributes,
    create_reader: CreateReader,
    create_writer: CreateWriter,
    enum_mft: EnumMft,
    _readwrite: Module,
    _platform: Module,
}

impl Api {
    fn load() -> windows::core::Result<Self> {
        let platform = Module::load("mfplat.dll")?;
        let readwrite = Module::load("mfreadwrite.dll")?;
        unsafe {
            Ok(Self {
                start: platform.symbol(b"MFStartup\0")?,
                stop: platform.symbol(b"MFShutdown\0")?,
                create_type: platform.symbol(b"MFCreateMediaType\0")?,
                create_attributes: platform.symbol(b"MFCreateAttributes\0")?,
                enum_mft: platform.symbol(b"MFTEnumEx\0")?,
                create_reader: readwrite.symbol(b"MFCreateSourceReaderFromURL\0")?,
                create_writer: readwrite.symbol(b"MFCreateSinkWriterFromURL\0")?,
                _readwrite: readwrite,
                _platform: platform,
            })
        }
    }
    fn media_type(&self) -> windows::core::Result<IMFMediaType> {
        let mut raw = null_mut();
        unsafe {
            (self.create_type)(&mut raw).ok()?;
            Ok(IMFMediaType::from_raw(raw))
        }
    }
    fn attributes(&self) -> windows::core::Result<IMFAttributes> {
        let mut raw = null_mut();
        unsafe {
            (self.create_attributes)(&mut raw, 4).ok()?;
            Ok(IMFAttributes::from_raw(raw))
        }
    }
    fn encoder(&self, subtype: GUID) -> windows::core::Result<bool> {
        self.encoder_for(MFMediaType_Audio, subtype)
    }

    fn encoder_for(&self, major: GUID, subtype: GUID) -> windows::core::Result<bool> {
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: major,
            guidSubtype: subtype,
        };
        let mut activations: *mut *mut c_void = null_mut();
        let mut count = 0;
        unsafe {
            (self.enum_mft)(
                if major == MFMediaType_Audio {
                    MFT_CATEGORY_AUDIO_ENCODER
                } else {
                    MFT_CATEGORY_VIDEO_ENCODER
                },
                (MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0) as u32,
                std::ptr::null(),
                &output,
                &mut activations,
                &mut count,
            )
            .ok()?;
            if !activations.is_null() {
                for index in 0..count as usize {
                    let activation = *activations.add(index);
                    if !activation.is_null() {
                        drop(windows::core::IUnknown::from_raw(activation));
                    }
                }
                CoTaskMemFree(Some(activations.cast()));
            }
        }
        Ok(count > 0)
    }
}

struct Com;
impl Com {
    fn init() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct Session {
    _com: Com,
    api: Api,
}
impl Session {
    fn start() -> windows::core::Result<Self> {
        let api = Api::load()?;
        let com = Com::init()?;
        unsafe {
            (api.start)(MF_VERSION, MFSTARTUP_FULL).ok()?;
        }
        Ok(Self { _com: com, api })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.api.stop)();
        }
    }
}

/// Called on a worker when building the catalog, never in a render/layout phase.
pub fn audio_capabilities() -> AudioCapabilities {
    let Ok(session) = Session::start() else {
        return AudioCapabilities::default();
    };
    AudioCapabilities {
        wave: true,
        mp3: session.api.encoder(MFAudioFormat_MP3).unwrap_or(false),
        aac: session.api.encoder(MFAudioFormat_AAC).unwrap_or(false),
        h264: session
            .api
            .encoder_for(MFMediaType_Video, MFVideoFormat_H264)
            .unwrap_or(false),
    }
}

fn audio_type(
    api: &Api,
    subtype: GUID,
    channels: u32,
    rate: u32,
    bits: Option<u32>,
    byte_rate: u32,
) -> windows::core::Result<IMFMediaType> {
    let media = api.media_type()?;
    unsafe {
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        media.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, byte_rate)?;
        if let Some(bits) = bits {
            media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, bits)?;
            media.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * (bits / 8))?;
            media.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        }
    }
    Ok(media)
}

fn encode_audio(
    source: &Path,
    output: &Path,
    target: AudioTarget,
    quality: u8,
    cancel: &AtomicBool,
) -> windows::core::Result<()> {
    let session = Session::start()?;
    let api = &session.api;
    let attributes = api.attributes()?;
    let input: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let output: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut raw = null_mut();
    unsafe {
        (api.create_reader)(PCWSTR(input.as_ptr()), attributes.as_raw(), &mut raw).ok()?;
    }
    let reader = unsafe { IMFSourceReader::from_raw(raw) };
    let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    unsafe {
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(audio, true)?;
    }
    let native = unsafe { reader.GetNativeMediaType(audio, 0)? };
    let source_rate = unsafe { native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? };
    let source_channels = unsafe { native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? };
    if source_rate == 0 || source_rate > 768_000 || source_channels == 0 || source_channels > 32 {
        return Err(Error::new(
            HRESULT(0x80070057_u32 as i32),
            "Invalid source audio rate or channel count",
        ));
    }
    let compressed = !matches!(target, AudioTarget::Wave);
    let channels = if compressed {
        source_channels.min(2)
    } else {
        source_channels
    };
    let rates: &[u32] = if matches!(target, AudioTarget::Mp3) {
        &[32_000, 44_100, 48_000]
    } else {
        &[44_100, 48_000]
    };
    let rate = if compressed && !rates.contains(&source_rate) {
        44_100
    } else {
        source_rate
    };
    let bits = if compressed {
        16
    } else {
        unsafe { native.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }
            .ok()
            .filter(|b| [8, 16, 24, 32].contains(b))
            .unwrap_or(16)
    };
    let pcm = audio_type(
        api,
        MFAudioFormat_PCM,
        channels,
        rate,
        Some(bits),
        rate * channels * (bits / 8),
    )?;
    unsafe {
        reader.SetCurrentMediaType(audio, None, &pcm)?;
    }
    let actual = unsafe { reader.GetCurrentMediaType(audio)? };
    let output_type = if compressed {
        let bitrate = if quality >= 95 {
            256_000
        } else if quality >= 85 {
            192_000
        } else {
            128_000
        };
        let bitrate = if channels == 1 { bitrate / 2 } else { bitrate };
        let subtype = if matches!(target, AudioTarget::Mp3) {
            MFAudioFormat_MP3
        } else {
            MFAudioFormat_AAC
        };
        audio_type(api, subtype, channels, rate, None, bitrate / 8)?
    } else {
        actual.clone()
    };
    let mut raw = null_mut();
    unsafe {
        (api.create_writer)(PCWSTR(output.as_ptr()), null_mut(), null_mut(), &mut raw).ok()?;
    }
    let writer = unsafe { IMFSinkWriter::from_raw(raw) };
    let stream = unsafe { writer.AddStream(&output_type)? };
    unsafe {
        writer.SetInputMediaType(stream, &actual, None)?;
        writer.BeginWriting()?;
    }
    let mut origin = None;
    let mut written = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::new(HRESULT(0x800704c7_u32 as i32), "Cancelled"));
        }
        let (mut flags, mut timestamp, mut sample) = (0, 0, None);
        unsafe {
            reader.ReadSample(
                audio,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;
        }
        if flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0) as u32
            != 0
        {
            return Err(Error::new(
                HRESULT(0x80004005_u32 as i32),
                "Audio decoder failed or changed format mid-stream",
            ));
        }
        if let Some(sample) = sample {
            let base = *origin.get_or_insert(timestamp);
            unsafe {
                sample.SetSampleTime(timestamp.saturating_sub(base).max(0))?;
                writer.WriteSample(stream, &sample)?;
            }
            written = true;
        } else if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 {
            if let Some(base) = origin {
                unsafe {
                    writer.SendStreamTick(stream, timestamp.saturating_sub(base).max(0))?;
                }
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            break;
        }
    }
    if !written {
        return Err(Error::new(
            HRESULT(0x80004005_u32 as i32),
            "Source has no decodable audio samples",
        ));
    }
    unsafe {
        writer.Finalize()?;
    }
    Ok(())
}

/// The caller owns the temporary output and publishes it only after success.
pub fn convert_audio(
    source: &Path,
    output: &Path,
    target: AudioTarget,
    quality: u8,
    cancel: &AtomicBool,
) -> Result<(), String> {
    encode_audio(source, output, target, quality, cancel).map_err(|error| format!("Windows audio conversion: {error}. The file may require an optional codec or audio engine."))
}
