use super::*;
use std::process::Command as Process;

fn decode(reader: &Path, source: &Path, args: &[&str]) -> Vec<u8> {
    let result = Process::new(reader)
        .args(["-v", "error", "-xerror", "-i"])
        .arg(source)
        .args(["-fps_mode", "passthrough"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}

#[test]
fn windows_video_encodes_h264_with_optional_audio_and_resize_without_external_engines() {
    let mut tools = Tools::discover();
    let Some(reader) = tools.get(Tool::Ffmpeg).map(Path::to_path_buf) else {
        return;
    };
    tools.0.clear();
    assert!(
        tools.1.h264 && tools.1.aac,
        "Validation machine needs Windows H.264 and AAC encoders"
    );
    let dir = tempfile::tempdir().unwrap();
    for audio in [false, true] {
        let source = dir.path().join(format!("clip-{audio}.avi"));
        let mut fixture = Process::new(&reader);
        fixture.args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x96:rate=12",
        ]);
        if audio {
            fixture.args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100"]);
        }
        let fixture = fixture
            .args([
                "-t",
                "0.75",
                "-c:v",
                "rawvideo",
                "-pix_fmt",
                "bgr24",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            fixture.status.success(),
            "{}",
            String::from_utf8_lossy(&fixture.stderr)
        );
        let original = std::fs::read(&source).unwrap();
        for edge in [None, Some(80)] {
            let plan = Plan::build(
                &[source.clone()],
                "mp4",
                Settings {
                    max_edge: edge,
                    ..Settings::default()
                },
                tools.clone(),
            )
            .unwrap();
            assert_eq!(plan.commands[0].backend, Backend::SystemVideo);
            assert!(plan.tools.0.is_empty());
            let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
            assert!(
                report.errors().is_empty(),
                "audio={audio}, edge={edge:?}: {:?}",
                report.errors()
            );
            let output = report.outputs()[0].clone();
            let probe = reader.with_file_name(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
            let metadata = if probe.is_file() {
                let result = Process::new(probe).args(["-v", "error", "-show_entries", "stream=codec_name,codec_type,start_time,duration,nb_frames,width,height:format=duration", "-of", "json"]).arg(&output).output().unwrap();
                assert!(result.status.success());
                let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
                let streams = data["streams"].as_array().unwrap();
                let video = streams
                    .iter()
                    .find(|stream| stream["codec_type"] == "video")
                    .unwrap();
                assert_eq!(video["codec_name"], "h264");
                let start: f64 = video["start_time"].as_str().unwrap().parse().unwrap();
                assert!(
                    start.abs() < 0.01,
                    "Video encoder shifted its time origin: {start}"
                );
                if audio {
                    let track = streams
                        .iter()
                        .find(|stream| stream["codec_type"] == "audio")
                        .unwrap();
                    assert_eq!(track["codec_name"], "aac");
                    let audio_start: f64 = track["start_time"].as_str().unwrap().parse().unwrap();
                    assert!(
                        (audio_start - start).abs() < 0.025,
                        "A/V synchronization changed: video={start}, audio={audio_start}"
                    );
                } else {
                    assert_eq!(streams.len(), 1);
                }
                let duration: f64 = data["format"]["duration"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(
                    (duration - 0.75).abs() < 0.04,
                    "Video timeline changed: {duration}"
                );
                Some(data)
            } else {
                None
            };
            let pixels = decode(
                &reader,
                &output,
                &[
                    "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
                ],
            );
            let frame_bytes = if edge.is_some() {
                80 * 48 * 3
            } else {
                160 * 96 * 3
            };
            assert_eq!(
                pixels.len(),
                frame_bytes * 9,
                "Video duration/frame count changed"
            );
            if edge.is_none() {
                let expected = decode(
                    &reader,
                    &source,
                    &[
                        "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
                    ],
                );
                assert_eq!(expected.len(), pixels.len());
                let error = pixels
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| f64::from(a.abs_diff(*b)))
                    .sum::<f64>()
                    / pixels.len() as f64;
                assert!(error < 16.0, "Unexpected color/orientation error: {error}");
            }
            if audio {
                let audio_bytes = decode(
                    &reader,
                    &output,
                    &[
                        "-map",
                        "0:a:0",
                        "-f",
                        "f32le",
                        "-acodec",
                        "pcm_f32le",
                        "pipe:1",
                    ],
                );
                let samples: Vec<_> = audio_bytes
                    .chunks_exact(4)
                    .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                    .collect();
                assert!(
                    samples.len() >= 32000 && samples.len() <= 37000,
                    "Audio duration changed: {}",
                    samples.len()
                );
                let energy = samples
                    .iter()
                    .map(|sample| f64::from(*sample).powi(2))
                    .sum::<f64>()
                    / samples.len() as f64;
                assert!(
                    energy > 0.002 && energy < 0.02,
                    "Missing or corrupt video audio: {energy}"
                );
            }
            if std::env::var_os("ATLAS_REAL_VIDEO_TEST").is_some() {
                let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../artifacts/handmade-conversion-validation/video");
                std::fs::create_dir_all(&artifacts).unwrap();
                let name = format!("audio-{audio}-edge-{}", edge.unwrap_or(160));
                std::fs::copy(&output, artifacts.join(format!("{name}.mp4"))).unwrap();
                if let Some(metadata) = &metadata {
                    std::fs::write(
                        artifacts.join(format!("{name}.json")),
                        serde_json::to_vec_pretty(metadata).unwrap(),
                    )
                    .unwrap();
                }
                image::RgbImage::from_raw(
                    if edge.is_some() { 80 } else { 160 },
                    if edge.is_some() { 48 } else { 96 },
                    pixels[..frame_bytes].to_vec(),
                )
                .unwrap()
                .save(artifacts.join(format!("{name}.png")))
                .unwrap();
            }
        }
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
}

#[test]
fn windows_video_rejects_invalid_sources_and_cancelled_jobs_without_leaking_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invalid.avi");
    std::fs::write(&source, b"not an AVI").unwrap();
    let mut tools = Tools::default();
    tools.1.h264 = true;
    let plan = Plan::build(&[source.clone()], "mp4", Settings::default(), tools).unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert_eq!(report.errors().len(), 1);
    assert!(report.outputs().is_empty());
    let report = execute(&plan, &Arc::new(AtomicBool::new(true)), |_, _, _| {});
    assert_eq!(report.cancelled(), 1);
    assert!(report.outputs().is_empty());
    assert_eq!(std::fs::read(source).unwrap(), b"not an AVI");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
