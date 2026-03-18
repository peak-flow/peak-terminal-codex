use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use crate::config;

#[derive(Debug, Clone)]
pub struct FontCandidate {
    pub display_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct FontCatalog {
    fonts: Vec<FontCandidate>,
}

impl FontCatalog {
    pub fn discover() -> Self {
        let mut candidates = Vec::new();

        for dir in font_roots() {
            collect_fonts(&dir, 2, &mut candidates);
        }

        let mut by_name = BTreeMap::<String, PathBuf>::new();
        for path in candidates {
            let Some(name) = path.file_stem().and_then(OsStr::to_str) else {
                continue;
            };

            by_name
                .entry(clean_font_name(name))
                .or_insert(path.to_path_buf());
        }

        let fonts = by_name
            .into_iter()
            .map(|(display_name, path)| FontCandidate { display_name, path })
            .collect();

        Self { fonts }
    }

    pub fn fonts(&self) -> &[FontCandidate] {
        &self.fonts
    }

    pub fn apply(&self, ctx: &egui::Context, preferred_name: Option<&str>) {
        let mut definitions = FontDefinitions::default();

        if let Some(candidate) = preferred_name
            .and_then(|name| self.find_by_name(name))
            .or_else(|| self.fonts.first())
        {
            if let Ok(bytes) = fs::read(&candidate.path) {
                definitions.font_data.insert(
                    "peak-terminal-custom".to_owned(),
                    FontData::from_owned(bytes).into(),
                );

                definitions
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .insert(0, "peak-terminal-custom".to_owned());
            }
        }

        ctx.set_fonts(definitions);
    }

    pub fn has_fonts(&self) -> bool {
        !self.fonts.is_empty()
    }

    pub fn find_by_name(&self, name: &str) -> Option<&FontCandidate> {
        self.fonts.iter().find(|font| font.display_name == name)
    }
}

fn font_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts"),
    ];

    if let Some(home) = config::home_dir() {
        roots.push(home.join("Library/Fonts"));
    }

    roots
}

fn collect_fonts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || !dir.exists() {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, depth - 1, out);
            continue;
        }

        if is_font_file(&path) && looks_like_nerd_font(&path) {
            out.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str).map(|ext| ext.to_ascii_lowercase()),
        Some(ref ext) if ext == "ttf" || ext == "otf" || ext == "ttc"
    )
}

fn looks_like_nerd_font(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    let lower = name.to_ascii_lowercase();
    lower.contains("nerd")
        || lower.contains("meslo")
        || lower.contains("caskaydia")
        || lower.contains("fira code")
        || lower.contains("jetbrainsmono")
}

fn clean_font_name(name: &str) -> String {
    name.replace(['_', '-'], " ")
}
