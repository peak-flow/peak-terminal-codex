use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use eframe::egui::{self, Color32, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use vt100::Color;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub surface: String,
    pub panel: String,
    pub accent: String,
    pub ansi: [String; 8],
    pub brights: [String; 8],
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub surface: Color32,
    pub panel: Color32,
    pub accent: Color32,
    pub ansi: [Color32; 8],
    pub brights: [Color32; 8],
}

impl ThemeDefinition {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read theme file {}", path.display()))?;

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        match extension.as_str() {
            "json" => serde_json::from_str(&contents)
                .with_context(|| format!("Failed to parse JSON theme from {}", path.display())),
            "toml" => toml::from_str(&contents)
                .with_context(|| format!("Failed to parse TOML theme from {}", path.display())),
            _ => bail!("Unsupported theme file format. Use .json or .toml"),
        }
    }

    pub fn compile(&self) -> Result<Theme> {
        Ok(Theme {
            background: parse_hex_color(&self.background)?,
            foreground: parse_hex_color(&self.foreground)?,
            cursor: parse_hex_color(&self.cursor)?,
            selection: parse_hex_color(&self.selection)?,
            surface: parse_hex_color(&self.surface)?,
            panel: parse_hex_color(&self.panel)?,
            accent: parse_hex_color(&self.accent)?,
            ansi: compile_palette(&self.ansi)?,
            brights: compile_palette(&self.brights)?,
        })
    }
}

impl Theme {
    pub fn apply_to_egui(&self, ctx: &egui::Context) {
        let mut visuals = if is_dark(self.background) {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        visuals.override_text_color = Some(self.foreground);
        visuals.panel_fill = self.surface;
        visuals.window_fill = self.panel;
        visuals.window_stroke.color = self.accent;
        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.foreground);
        visuals.widgets.inactive.bg_fill = mix(self.panel, self.surface, 0.35);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.foreground);
        visuals.widgets.hovered.bg_fill = mix(self.panel, self.accent, 0.25);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.foreground);
        visuals.widgets.active.bg_fill = mix(self.accent, self.surface, 0.35);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.foreground);
        visuals.widgets.open.bg_fill = mix(self.panel, self.surface, 0.5);
        visuals.selection.bg_fill = mix(self.selection, self.accent, 0.35);
        visuals.selection.stroke = Stroke::new(1.0, self.foreground);
        visuals.extreme_bg_color = self.background;
        visuals.code_bg_color = mix(self.background, self.surface, 0.45);

        ctx.set_visuals(visuals);
    }
}

pub fn builtin_themes() -> Vec<ThemeDefinition> {
    vec![
        catppuccin_latte(),
        catppuccin_frappe(),
        catppuccin_macchiato(),
        catppuccin_mocha(),
        peak_dawn(),
        peak_night(),
    ]
}

pub fn available_themes(custom: &[ThemeDefinition]) -> Vec<ThemeDefinition> {
    let mut themes = builtin_themes();
    themes.extend(custom.iter().cloned());
    themes
}

pub fn resolve_theme(name: &str, custom: &[ThemeDefinition]) -> Theme {
    let themes = available_themes(custom);

    themes
        .iter()
        .find(|theme| theme.name == name)
        .or_else(|| themes.first())
        .and_then(|theme| theme.compile().ok())
        .unwrap_or_else(|| {
            catppuccin_mocha()
                .compile()
                .expect("builtin theme is valid")
        })
}

pub fn theme_names(custom: &[ThemeDefinition]) -> Vec<String> {
    available_themes(custom)
        .into_iter()
        .map(|theme| theme.name)
        .collect()
}

pub fn vt_color(theme: &Theme, color: Color, is_background: bool) -> Color32 {
    match color {
        Color::Default => {
            if is_background {
                theme.background
            } else {
                theme.foreground
            }
        }
        Color::Idx(index) => indexed_color(theme, index),
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

pub fn brighten(color: Color32, factor: f32) -> Color32 {
    let [r, g, b, a] = color.to_array();
    Color32::from_rgba_unmultiplied(
        scale_channel(r, factor),
        scale_channel(g, factor),
        scale_channel(b, factor),
        a,
    )
}

pub fn dim(color: Color32, factor: f32) -> Color32 {
    brighten(color, factor)
}

fn compile_palette(values: &[String; 8]) -> Result<[Color32; 8]> {
    let mut palette = [Color32::BLACK; 8];
    for (index, value) in values.iter().enumerate() {
        palette[index] = parse_hex_color(value)?;
    }
    Ok(palette)
}

fn parse_hex_color(value: &str) -> Result<Color32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        bail!("Invalid color {value}. Expected 6 hex digits")
    }

    let r = u8::from_str_radix(&value[0..2], 16).context("Invalid red channel")?;
    let g = u8::from_str_radix(&value[2..4], 16).context("Invalid green channel")?;
    let b = u8::from_str_radix(&value[4..6], 16).context("Invalid blue channel")?;

    Ok(Color32::from_rgb(r, g, b))
}

fn indexed_color(theme: &Theme, index: u8) -> Color32 {
    match index {
        0..=7 => theme.ansi[index as usize],
        8..=15 => theme.brights[(index - 8) as usize],
        16..=231 => {
            let index = index - 16;
            let r = (index / 36) % 6;
            let g = (index / 6) % 6;
            let b = index % 6;
            Color32::from_rgb(cube_channel(r), cube_channel(g), cube_channel(b))
        }
        232..=255 => {
            let shade = 8 + (index - 232) * 10;
            Color32::from_rgb(shade, shade, shade)
        }
    }
}

fn cube_channel(index: u8) -> u8 {
    match index {
        0 => 0,
        _ => 55 + index * 40,
    }
}

fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let inverse = 1.0 - amount;
    Color32::from_rgba_unmultiplied(
        (ar as f32 * inverse + br as f32 * amount).round() as u8,
        (ag as f32 * inverse + bg as f32 * amount).round() as u8,
        (ab as f32 * inverse + bb as f32 * amount).round() as u8,
        aa,
    )
}

fn scale_channel(channel: u8, factor: f32) -> u8 {
    ((channel as f32 * factor).clamp(0.0, 255.0)).round() as u8
}

fn is_dark(color: Color32) -> bool {
    let [r, g, b, _] = color.to_array();
    let luminance = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    luminance < 145.0
}

fn catppuccin_latte() -> ThemeDefinition {
    ThemeDefinition {
        name: "Catppuccin Latte".to_owned(),
        background: "#eff1f5".to_owned(),
        foreground: "#4c4f69".to_owned(),
        cursor: "#dc8a78".to_owned(),
        selection: "#bcc0cc".to_owned(),
        surface: "#e6e9ef".to_owned(),
        panel: "#dce0e8".to_owned(),
        accent: "#1e66f5".to_owned(),
        ansi: palette([
            "#5c5f77", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#ea76cb", "#179299", "#acb0be",
        ]),
        brights: palette([
            "#6c6f85", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#ea76cb", "#179299", "#bcc0cc",
        ]),
    }
}

fn catppuccin_frappe() -> ThemeDefinition {
    ThemeDefinition {
        name: "Catppuccin Frappe".to_owned(),
        background: "#303446".to_owned(),
        foreground: "#c6d0f5".to_owned(),
        cursor: "#f2d5cf".to_owned(),
        selection: "#51576d".to_owned(),
        surface: "#292c3c".to_owned(),
        panel: "#232634".to_owned(),
        accent: "#8caaee".to_owned(),
        ansi: palette([
            "#51576d", "#e78284", "#a6d189", "#e5c890", "#8caaee", "#f4b8e4", "#81c8be", "#b5bfe2",
        ]),
        brights: palette([
            "#626880", "#e78284", "#a6d189", "#e5c890", "#8caaee", "#f4b8e4", "#81c8be", "#c6d0f5",
        ]),
    }
}

fn catppuccin_macchiato() -> ThemeDefinition {
    ThemeDefinition {
        name: "Catppuccin Macchiato".to_owned(),
        background: "#24273a".to_owned(),
        foreground: "#cad3f5".to_owned(),
        cursor: "#f4dbd6".to_owned(),
        selection: "#494d64".to_owned(),
        surface: "#1f2232".to_owned(),
        panel: "#181926".to_owned(),
        accent: "#8aadf4".to_owned(),
        ansi: palette([
            "#494d64", "#ed8796", "#a6da95", "#eed49f", "#8aadf4", "#f5bde6", "#8bd5ca", "#b8c0e0",
        ]),
        brights: palette([
            "#5b6078", "#ed8796", "#a6da95", "#eed49f", "#8aadf4", "#f5bde6", "#8bd5ca", "#cad3f5",
        ]),
    }
}

fn catppuccin_mocha() -> ThemeDefinition {
    ThemeDefinition {
        name: "Catppuccin Mocha".to_owned(),
        background: "#1e1e2e".to_owned(),
        foreground: "#cdd6f4".to_owned(),
        cursor: "#f5e0dc".to_owned(),
        selection: "#45475a".to_owned(),
        surface: "#181825".to_owned(),
        panel: "#11111b".to_owned(),
        accent: "#89b4fa".to_owned(),
        ansi: palette([
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
        ]),
        brights: palette([
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#cdd6f4",
        ]),
    }
}

fn peak_dawn() -> ThemeDefinition {
    ThemeDefinition {
        name: "Peak Dawn".to_owned(),
        background: "#f6efe4".to_owned(),
        foreground: "#3d342d".to_owned(),
        cursor: "#bf5f3c".to_owned(),
        selection: "#e2d3be".to_owned(),
        surface: "#efe1ca".to_owned(),
        panel: "#e7d4b4".to_owned(),
        accent: "#197278".to_owned(),
        ansi: palette([
            "#5a5046", "#b55239", "#4f772d", "#c07f00", "#197278", "#b65f7f", "#26828e", "#cdc3b5",
        ]),
        brights: palette([
            "#73675b", "#c96246", "#5b8c35", "#d08d0f", "#2c8b92", "#cb7393", "#359faa", "#efe6d8",
        ]),
    }
}

fn peak_night() -> ThemeDefinition {
    ThemeDefinition {
        name: "Peak Night".to_owned(),
        background: "#10212b".to_owned(),
        foreground: "#d8e2dc".to_owned(),
        cursor: "#f4a261".to_owned(),
        selection: "#23404e".to_owned(),
        surface: "#0d1a22".to_owned(),
        panel: "#081319".to_owned(),
        accent: "#2ec4b6".to_owned(),
        ansi: palette([
            "#314b57", "#e76f51", "#6ab04c", "#f4a261", "#4ea8de", "#c77dff", "#2ec4b6", "#a6b8c0",
        ]),
        brights: palette([
            "#466775", "#f08a6a", "#8bcf6a", "#f7b87a", "#72bcea", "#d7a4ff", "#59d7cb", "#d8e2dc",
        ]),
    }
}

fn palette(values: [&str; 8]) -> [String; 8] {
    values.map(str::to_owned)
}
