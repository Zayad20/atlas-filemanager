use extism_pdk::*;
use serde::Deserialize;
use std::fs::File;
use std::io::{Read, Write};
use zip::ZipArchive;

#[derive(Deserialize)]
struct CommandExecute {
    command_id: String,
    #[serde(default)]
    payload: String,
    #[serde(default)]
    host_dir: String,
}

// Vinculamos la función del Host definida en el manager de Rust
#[extism_pdk::host_fn]
extern "ExtismHost" {
    fn host_show_quick_pick(options_json: String) -> String;
}

#[plugin_fn]
pub fn execute_command(input: String) -> FnResult<String> {
    let query: CommandExecute = serde_json::from_str(&input)?;

    match query.command_id.as_str() {
        "icon-manager.select-theme" => {
            // Llamamos al Host para que dibuje la interfaz gráfica (la Command Palette)
            let themes = serde_json::to_string(&vec!["Material Dark", "Material Light", "Ocean"])?;
            let user_choice = unsafe { host_show_quick_pick(themes)? };
            return Ok(format!("Tema actualizado a: {}", user_choice));
        }
        "icon-manager.download-theme" => {
            let ext_id = if query.payload.is_empty() {
                let options = vec![
                    "PKief/material-icon-theme",
                    "BeardedBear/beardedicons",
                    "catppuccin/catppuccin-vsc-icons",
                    "yusifaliyevpro/vscicons",
                    "thang-nm/flow-icons",
                ];
                let options_json = serde_json::to_string(&options)?;
                let choice = unsafe { host_show_quick_pick(options_json)? };
                if choice.is_empty() {
                    return Ok("Cancelado por el usuario".to_string());
                }
                choice
            } else {
                query.payload.clone()
            };

            // 1. Llamar a la API de Open VSX
            let api_url = format!("https://open-vsx.org/api/{}/latest", ext_id);
            let req = extism_pdk::HttpRequest::new(&api_url);
            let res = extism_pdk::http::request::<()>(&req, None)?;

            let json_body: serde_json::Value = serde_json::from_slice(&res.body())?;

            // 2. Extraer el link al archivo .vsix
            let download_url = json_body
                .get("files")
                .and_then(|f| f.get("download"))
                .and_then(|url| url.as_str())
                .ok_or_else(|| extism_pdk::Error::msg(format!("No download URL found. URL: {}, API response: {}", api_url, json_body)))?;

            // 3. Descargar el archivo .vsix
            let vsix_req = extism_pdk::HttpRequest::new(download_url);
            let vsix_res = extism_pdk::http::request::<()>(&vsix_req, None)?;

            // 4. Guardarlo en el disco duro aislado (proveido por el Host vía WASI)
            let vsix_path = format!("/ext/{}.vsix", ext_id.replace('/', "_"));
            let mut file = File::create(&vsix_path)?;
            file.write_all(&vsix_res.body())?;

            // 5. Descomprimir el .vsix (que es un zip) en un directorio
            let out_dir = format!("/ext/{}", ext_id.replace('/', "_"));
            std::fs::create_dir_all(&out_dir).ok();

            let file = File::open(&vsix_path)?;
            let mut archive = ZipArchive::new(file)?;

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                let outpath = match file.enclosed_name() {
                    Some(path) => format!("{}/{}", out_dir, path.to_string_lossy()),
                    None => continue,
                };

                if (*file.name()).ends_with('/') {
                    std::fs::create_dir_all(&outpath).ok();
                } else {
                    if let Some(p) = std::path::Path::new(&outpath).parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut outfile = File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;
                }
            }

            // 6. Analizar el package.json para descubrir los sub-temas
            let pkg_json_path = format!("{}/extension/package.json", out_dir);
            let mut pkg_file = File::open(&pkg_json_path)?;
            let mut pkg_contents = String::new();
            pkg_file.read_to_string(&mut pkg_contents)?;

            let pkg: serde_json::Value = serde_json::from_str(&pkg_contents)?;
            let mut available_themes = vec![];
            let mut theme_paths = std::collections::HashMap::new();

            if let Some(themes) = pkg
                .get("contributes")
                .and_then(|c| c.get("iconThemes"))
                .and_then(|t| t.as_array())
            {
                for theme in themes {
                    let id = theme.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let label = theme.get("label").and_then(|l| l.as_str()).unwrap_or(id);
                    if let Some(path) = theme.get("path").and_then(|p| p.as_str()) {
                        let display_label = if label.is_empty() { "Default" } else { label };
                        available_themes.push(display_label.to_string());
                        theme_paths.insert(display_label.to_string(), path.to_string());
                    }
                }
            }

            if available_themes.is_empty() {
                return Ok("No se encontraron iconThemes en package.json".to_string());
            }

            // 7. Preguntar al usuario qué sub-tema quiere si hay varios
            let selected_label = if available_themes.len() > 1 {
                let options_json = serde_json::to_string(&available_themes)?;
                let choice = unsafe { host_show_quick_pick(options_json)? };
                if choice.is_empty() {
                    return Ok("Cancelado por el usuario".to_string());
                }
                choice
            } else {
                available_themes[0].clone()
            };

            let rel_path = theme_paths
                .get(&selected_label)
                .unwrap()
                .trim_start_matches("./");
            let theme_json_path = format!("{}/extension/{}", out_dir, rel_path);
            
            // Calculamos el directorio relativo dentro del zip para poder construir el path absoluto del host
            let host_theme_dir = format!("{}/{}/extension/{}", 
                query.host_dir, 
                ext_id.replace('/', "_"),
                std::path::Path::new(rel_path).parent().unwrap_or(std::path::Path::new("")).to_string_lossy()
            );

            // 8. Leer el JSON del tema de iconos de VSCode
            let mut tfile = File::open(&theme_json_path)?;
            let mut tcontents = String::new();
            tfile.read_to_string(&mut tcontents)?;

            // Ignorar comentarios en JSON (común en VSCode themes) antes de parsear si es necesario,
            // pero serde_json puede fallar. Reemplazamos los bloque y de linea temporalmente de forma rustica.
            // Para simplificar, asumimos JSON limpio o le quitamos los comentarios básicos.
            let clean_json = tcontents
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");

            let theme_data: serde_json::Value =
                serde_json::from_str(&clean_json).unwrap_or_else(|_| serde_json::json!({}));

            let mut host_mappings = serde_json::Map::new();

            if let (Some(exts), Some(defs)) = (
                theme_data.get("fileExtensions"),
                theme_data.get("iconDefinitions"),
            ) {
                if let (Some(ext_map), Some(def_map)) = (exts.as_object(), defs.as_object()) {
                    for (ext, icon_id) in ext_map {
                        if let Some(icon_id_str) = icon_id.as_str() {
                            if let Some(def) = def_map.get(icon_id_str) {
                                if let Some(icon_path) =
                                    def.get("iconPath").and_then(|p| p.as_str())
                                {
                                    // Limpiar icon_path y resolverlo a ruta absoluta del Host
                                    let clean_ipath = icon_path.trim_start_matches("./");
                                    // Limpiamos los "../" para rutas simples asumiendo que los iconos están al lado o en subcarpetas
                                    // No es un path resolve perfecto pero sirve para la mayoría de temas.
                                    let absolute_path = format!("{}/{}", host_theme_dir, clean_ipath);
                                    host_mappings.insert(
                                        ext.clone(),
                                        serde_json::Value::String(absolute_path),
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // 9. Guardar theme_mappings.json para el host
            let mut mappings_file = File::create("/ext/theme_mappings.json")?;
            mappings_file.write_all(
                serde_json::Value::Object(host_mappings)
                    .to_string()
                    .as_bytes(),
            )?;

            return Ok(format!("Tema '{}' activado exitosamente.", selected_label));
        }
        "icon-manager.get-theme-mappings" => {
            // Un comando directo para pedir los mapeos JSON desde el UI si fuera necesario
            let mappings = serde_json::json!({
                "rs": "rust.svg",
                "js": "javascript.svg"
            });
            return Ok(mappings.to_string());
        }
        _ => return Ok("Comando desconocido".to_string()),
    }
}
