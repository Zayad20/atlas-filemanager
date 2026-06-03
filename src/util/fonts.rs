use egui::{FontData, FontDefinitions, FontFamily};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SF_REGULAR_FAMILY: &str = "Atlas.sf-regular";
const SF_MEDIUM_FAMILY: &str = "Atlas.sf-medium";
const SF_SEMIBOLD_FAMILY: &str = "Atlas.sf-semibold";
const SF_MONO_FAMILY: &str = "Atlas.sf-mono";

#[derive(Debug)]
pub struct InstalledFonts {
    pub sf_regular: bool,
    pub sf_medium: bool,
    pub sf_semibold: bool,
    pub sf_mono: bool,
}

pub fn install(ctx: &egui::Context) -> InstalledFonts {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let system_fonts = index_system_fonts();

    let regular = install_face(
        &mut fonts,
        &system_fonts,
        "Atlas.sf-regular.face",
        "ATLAS_SF_REGULAR",
        &[
            "SF-Pro-Text-Regular.otf",
            "SFProText-Regular.otf",
            "SF-Pro.ttf",
            "SFNS.ttf",
            ".SFNS.ttf",
        ],
    );
    let medium = install_face(
        &mut fonts,
        &system_fonts,
        "Atlas.sf-medium.face",
        "ATLAS_SF_MEDIUM",
        &["SF-Pro-Text-Medium.otf", "SFProText-Medium.otf"],
    );
    let semibold = install_face(
        &mut fonts,
        &system_fonts,
        "Atlas.sf-semibold.face",
        "ATLAS_SF_SEMIBOLD",
        &["SF-Pro-Text-Semibold.otf", "SFProText-Semibold.otf"],
    );
    let mono = install_face(
        &mut fonts,
        &system_fonts,
        "Atlas.sf-mono.face",
        "ATLAS_SF_MONO",
        &[
            "SFMono-Regular.otf",
            "SF-Mono-Regular.otf",
            "SFNSMono.ttf",
            ".SFNSMono.ttf",
        ],
    );

    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let monospace = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    add_named_family(
        &mut fonts,
        SF_REGULAR_FAMILY,
        regular.as_deref(),
        &proportional,
    );
    add_named_family(
        &mut fonts,
        SF_MEDIUM_FAMILY,
        medium.as_deref().or(regular.as_deref()),
        &proportional,
    );
    add_named_family(
        &mut fonts,
        SF_SEMIBOLD_FAMILY,
        semibold
            .as_deref()
            .or(medium.as_deref())
            .or(regular.as_deref()),
        &proportional,
    );
    add_named_family(&mut fonts, SF_MONO_FAMILY, mono.as_deref(), &monospace);

    if let Some(face) = regular.as_deref() {
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, face.to_owned());
    }
    if let Some(face) = mono.as_deref() {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, face.to_owned());
    }

    ctx.set_fonts(fonts);

    InstalledFonts {
        sf_regular: regular.is_some(),
        sf_medium: medium.is_some(),
        sf_semibold: semibold.is_some(),
        sf_mono: mono.is_some(),
    }
}

pub fn regular() -> FontFamily {
    named_family(SF_REGULAR_FAMILY)
}

pub fn medium() -> FontFamily {
    named_family(SF_MEDIUM_FAMILY)
}

pub fn semibold() -> FontFamily {
    named_family(SF_SEMIBOLD_FAMILY)
}

pub fn mono() -> FontFamily {
    named_family(SF_MONO_FAMILY)
}

fn named_family(name: &'static str) -> FontFamily {
    FontFamily::Name(Arc::from(name))
}

fn add_named_family(
    fonts: &mut FontDefinitions,
    family_name: &'static str,
    primary: Option<&str>,
    fallbacks: &[String],
) {
    let mut family = fallbacks.to_vec();
    if let Some(primary) = primary {
        family.retain(|font| font != primary);
        family.insert(0, primary.to_owned());
    }
    fonts.families.insert(named_family(family_name), family);
}

fn install_face(
    fonts: &mut FontDefinitions,
    system_fonts: &HashMap<String, PathBuf>,
    font_key: &'static str,
    env_var: &'static str,
    filenames: &[&str],
) -> Option<String> {
    let path = std::env::var_os(env_var)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| find_system_font(system_fonts, filenames))?;
    let bytes = std::fs::read(path).ok()?;
    fonts
        .font_data
        .insert(font_key.to_owned(), Arc::new(FontData::from_owned(bytes)));
    Some(font_key.to_owned())
}

fn find_system_font(
    system_fonts: &HashMap<String, PathBuf>,
    filenames: &[&str],
) -> Option<PathBuf> {
    filenames
        .iter()
        .find_map(|filename| system_fonts.get(&filename.to_lowercase()).cloned())
}

fn index_system_fonts() -> HashMap<String, PathBuf> {
    let mut fonts = HashMap::new();
    for root in system_font_dirs() {
        let mut pending = vec![(root, 0_usize)];
        while let Some((dir, depth)) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && depth < 3 {
                    pending.push((path, depth + 1));
                    continue;
                }
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                fonts.entry(name.to_lowercase()).or_insert(path);
            }
        }
    }
    fonts
}

fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/usr/share/fonts"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Library/Fonts"));
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    if let Some(windir) = std::env::var_os("WINDIR") {
        dirs.push(Path::new(&windir).join("Fonts"));
    }
    dirs
}
