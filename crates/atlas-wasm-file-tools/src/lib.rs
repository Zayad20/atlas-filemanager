use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct CommandExecute {
    command_id: String,
    #[serde(default)]
    payload: String,
}

#[derive(Deserialize)]
struct WasmPayload {
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Serialize)]
struct RunRequest<'a> {
    cmd: &'a str,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct RunResponse {
    exit_code: i32,
    #[serde(default)]
    stderr: String,
}

#[extism_pdk::host_fn]
extern "ExtismHost" {
    fn host_show_quick_pick(options_json: String) -> String;
    fn host_run_command(request_json: String) -> String;
}

fn run_cmd(cmd: &str, args: Vec<String>) -> FnResult<RunResponse> {
    let req = RunRequest { cmd, args };
    let req_json = serde_json::to_string(&req)?;
    let resp_json = unsafe { host_run_command(req_json)? };
    let resp: RunResponse = serde_json::from_str(&resp_json)?;
    Ok(resp)
}

fn pick(options: &[&str]) -> FnResult<String> {
    let json = serde_json::to_string(options)?;
    let choice = unsafe { host_show_quick_pick(json)? };
    Ok(choice)
}

#[plugin_fn]
pub fn execute_command(input: String) -> FnResult<String> {
    let query: CommandExecute = serde_json::from_str(&input)?;
    let payload: WasmPayload =
        serde_json::from_str(&query.payload).unwrap_or(WasmPayload { paths: vec![] });

    match query.command_id.as_str() {
        "resize-image" => cmd_resize_image(&payload),
        "image-to-png" => cmd_image_to_png(&payload),
        "compress-pdf" => cmd_compress_pdf(&payload),
        "video-to-mp4" => cmd_video_to_mp4(&payload),
        _ => Ok(format!("Unknown command: {}", query.command_id)),
    }
}

fn path_parts(path: &str) -> (String, String, String) {
    let p = std::path::Path::new(path);
    let dir = p
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    (dir, stem, ext)
}

fn cmd_resize_image(payload: &WasmPayload) -> FnResult<String> {
    let width = pick(&["640", "1280", "1920", "3840"])?;
    if width.is_empty() {
        return Ok("Cancelado".to_string());
    }

    let mut errors = Vec::new();
    for path in &payload.paths {
        let (dir, stem, ext) = path_parts(path);
        let output = format!("{}/{}_resized.{}", dir, stem, ext);
        let resp = run_cmd(
            "magick",
            vec![
                path.clone(),
                "-resize".to_string(),
                format!("{}x", width),
                output,
            ],
        )?;
        if resp.exit_code != 0 {
            errors.push(format!("{}: {}", path, resp.stderr.trim()));
        }
    }

    if errors.is_empty() {
        Ok(format!(
            "Redimensionados {} archivo(s) a {}px de ancho",
            payload.paths.len(),
            width
        ))
    } else {
        Ok(errors.join("\n"))
    }
}

fn cmd_image_to_png(payload: &WasmPayload) -> FnResult<String> {
    let mut errors = Vec::new();
    for path in &payload.paths {
        let (dir, stem, _) = path_parts(path);
        let output = format!("{}/{}.png", dir, stem);
        let resp = run_cmd("magick", vec![path.clone(), output])?;
        if resp.exit_code != 0 {
            errors.push(format!("{}: {}", path, resp.stderr.trim()));
        }
    }

    if errors.is_empty() {
        Ok(format!(
            "Convertidos {} archivo(s) a PNG",
            payload.paths.len()
        ))
    } else {
        Ok(errors.join("\n"))
    }
}

fn cmd_compress_pdf(payload: &WasmPayload) -> FnResult<String> {
    let mut errors = Vec::new();
    for path in &payload.paths {
        let (dir, stem, _) = path_parts(path);
        let output = format!("{}/{}_compressed.pdf", dir, stem);
        let resp = run_cmd(
            "gs",
            vec![
                "-sDEVICE=pdfwrite".to_string(),
                "-dCompatibilityLevel=1.4".to_string(),
                "-dPDFSETTINGS=/ebook".to_string(),
                "-dNOPAUSE".to_string(),
                "-dQUIET".to_string(),
                "-dBATCH".to_string(),
                format!("-sOutputFile={}", output),
                path.clone(),
            ],
        )?;
        if resp.exit_code != 0 {
            errors.push(format!("{}: {}", path, resp.stderr.trim()));
        }
    }

    if errors.is_empty() {
        Ok(format!("Comprimidos {} PDF(s)", payload.paths.len()))
    } else {
        Ok(errors.join("\n"))
    }
}

fn cmd_video_to_mp4(payload: &WasmPayload) -> FnResult<String> {
    let quality = pick(&["Alta (CRF 18)", "Media (CRF 23)", "Baja (CRF 28)"])?;
    if quality.is_empty() {
        return Ok("Cancelado".to_string());
    }

    let crf = if quality.contains("18") {
        "18"
    } else if quality.contains("28") {
        "28"
    } else {
        "23"
    };

    let mut errors = Vec::new();
    for path in &payload.paths {
        let (dir, stem, _) = path_parts(path);
        let output = format!("{}/{}.mp4", dir, stem);
        let resp = run_cmd(
            "ffmpeg",
            vec![
                "-i".to_string(),
                path.clone(),
                "-c:v".to_string(),
                "libx264".to_string(),
                "-crf".to_string(),
                crf.to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-y".to_string(),
                output,
            ],
        )?;
        if resp.exit_code != 0 {
            errors.push(format!("{}: {}", path, resp.stderr.trim()));
        }
    }

    if errors.is_empty() {
        Ok(format!(
            "Convertidos {} video(s) a MP4 (CRF {})",
            payload.paths.len(),
            crf
        ))
    } else {
        Ok(errors.join("\n"))
    }
}
