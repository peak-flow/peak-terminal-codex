use std::fs;
use std::time::Duration;

use eframe::egui;

use crate::config::{self, AppConfig};
use crate::fonts::FontCatalog;
use crate::terminal::{RuntimeSupport, SplitAxis, TerminalWorkspace};
use crate::theme::{Theme, ThemeDefinition, resolve_theme, theme_names};

pub struct PeakTerminalApp {
    config: AppConfig,
    theme: Theme,
    fonts: FontCatalog,
    runtime: RuntimeSupport,
    workspace: TerminalWorkspace,
    preferences_open: bool,
    status_message: Option<String>,
}

impl PeakTerminalApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = AppConfig::load();
        let fonts = FontCatalog::discover();
        fonts.apply(&cc.egui_ctx, config.font_name.as_deref());

        let theme = resolve_theme(&config.theme, &config.custom_themes);
        theme.apply_to_egui(&cc.egui_ctx);

        let runtime = RuntimeSupport::detect();
        let workspace = TerminalWorkspace::new(&config, &runtime)
            .unwrap_or_else(|error| TerminalWorkspace::empty(error.to_string()));

        Self {
            config,
            theme,
            fonts,
            runtime,
            workspace,
            preferences_open: false,
            status_message: None,
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    fn refresh_theme(&mut self, ctx: &egui::Context) {
        self.theme = resolve_theme(&self.config.theme, &self.config.custom_themes);
        self.theme.apply_to_egui(ctx);
    }

    fn save_config(&mut self) {
        if let Err(error) = self.config.save() {
            self.set_status(format!("Failed to save preferences: {error}"));
        }
    }

    fn apply_font_preferences(&mut self, ctx: &egui::Context) {
        self.fonts.apply(ctx, self.config.font_name.as_deref());
    }

    fn handle_shortcuts(&mut self, events: &[egui::Event], ctx: &egui::Context) {
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };

            if modifiers.command && *key == egui::Key::Comma {
                self.preferences_open = true;
            }

            if modifiers.command && *key == egui::Key::T {
                if let Err(error) = self.workspace.new_tab(&self.config, &self.runtime) {
                    self.set_status(format!("Failed to open new tab: {error}"));
                }
            }

            if modifiers.command && *key == egui::Key::W && self.workspace.tab_count() > 0 {
                self.workspace.close_active_tab();
            }

            if modifiers.command && modifiers.shift && *key == egui::Key::OpenBracket {
                let next = self.workspace.active_tab().saturating_sub(1);
                self.workspace.set_active_tab(next);
            }

            if modifiers.command && modifiers.shift && *key == egui::Key::CloseBracket {
                let next = (self.workspace.active_tab() + 1)
                    .min(self.workspace.tab_count().saturating_sub(1));
                self.workspace.set_active_tab(next);
            }

            if modifiers.command && modifiers.alt && *key == egui::Key::Minus {
                self.config.font_size = (self.config.font_size - 1.0).max(11.0);
                self.apply_font_preferences(ctx);
                self.save_config();
            }

            if modifiers.command && modifiers.alt && *key == egui::Key::Equals {
                self.config.font_size = (self.config.font_size + 1.0).min(28.0);
                self.apply_font_preferences(ctx);
                self.save_config();
            }
        }
    }

    fn show_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("peak_top_bar")
            .frame(egui::Frame::new().fill(self.theme.panel))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("New Tab").clicked() {
                        if let Err(error) = self.workspace.new_tab(&self.config, &self.runtime) {
                            self.set_status(format!("Failed to create tab: {error}"));
                        }
                    }

                    if ui.button("Close Tab").clicked() && self.workspace.tab_count() > 0 {
                        self.workspace.close_active_tab();
                    }

                    if ui.button("Split Right").clicked() {
                        if let Err(error) = self.workspace.split_focused(
                            SplitAxis::Columns,
                            &self.config,
                            &self.runtime,
                        ) {
                            self.set_status(format!("Failed to split pane: {error}"));
                        }
                    }

                    if ui.button("Split Down").clicked() {
                        if let Err(error) = self.workspace.split_focused(
                            SplitAxis::Rows,
                            &self.config,
                            &self.runtime,
                        ) {
                            self.set_status(format!("Failed to split pane: {error}"));
                        }
                    }

                    if ui.button("Preferences").clicked() {
                        self.preferences_open = true;
                    }

                    if let Some(message) = &self.status_message {
                        ui.separator();
                        ui.label(message);
                    }
                });

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    let tabs: Vec<_> = self.workspace.tabs().collect();
                    for (index, title, tab_id) in tabs {
                        let selected = index == self.workspace.active_tab();
                        let label = format!("{}  {}", if selected { ">" } else { " " }, title);
                        if ui
                            .push_id(tab_id, |ui| ui.selectable_label(selected, label))
                            .inner
                            .clicked()
                        {
                            self.workspace.set_active_tab(index);
                        }
                    }
                });
                ui.add_space(4.0);
            });
    }

    fn show_preferences(&mut self, ctx: &egui::Context) {
        let mut open = self.preferences_open;
        egui::Window::new("Preferences")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                let mut config_changed = false;

                ui.heading("Appearance");
                ui.add_space(6.0);

                egui::ComboBox::from_label("Theme")
                    .selected_text(&self.config.theme)
                    .show_ui(ui, |ui| {
                        for name in theme_names(&self.config.custom_themes) {
                            if ui
                                .selectable_label(self.config.theme == name, &name)
                                .clicked()
                            {
                                self.config.theme = name;
                                config_changed = true;
                            }
                        }
                    });

                if ui.button("Import Custom Theme").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Theme files", &["json", "toml"])
                        .pick_file()
                    {
                        match ThemeDefinition::from_file(&path) {
                            Ok(theme) => {
                                let imported_name = theme.name.clone();
                                self.config
                                    .custom_themes
                                    .retain(|existing| existing.name != imported_name);
                                self.config.custom_themes.push(theme.clone());
                                self.config.theme = imported_name.clone();
                                config_changed = true;

                                if let Ok(dir) = config::themes_dir() {
                                    let _ = config::ensure_dir(&dir);
                                    let destination = dir.join(format!("{imported_name}.toml"));
                                    if let Ok(contents) = toml::to_string_pretty(&theme) {
                                        let _ = fs::write(destination, contents);
                                    }
                                }

                                self.set_status(format!("Imported theme: {imported_name}"));
                            }
                            Err(error) => {
                                self.set_status(format!("Theme import failed: {error}"));
                            }
                        }
                    }
                }

                ui.add(
                    egui::Slider::new(&mut self.config.font_size, 11.0..=28.0)
                        .text("Terminal Font Size"),
                );
                if ui.button("Apply Font Size").clicked() {
                    self.apply_font_preferences(ctx);
                    config_changed = true;
                }

                if self.fonts.has_fonts() {
                    let selected_font = self
                        .config
                        .font_name
                        .as_deref()
                        .unwrap_or("Auto-detect Nerd Font");

                    egui::ComboBox::from_label("Terminal Font")
                        .selected_text(selected_font)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.config.font_name.is_none(), "Auto-detect Nerd Font")
                                .clicked()
                            {
                                self.config.font_name = None;
                                config_changed = true;
                            }

                            for font in self.fonts.fonts() {
                                if ui
                                    .selectable_label(
                                        self.config.font_name.as_deref()
                                            == Some(font.display_name.as_str()),
                                        &font.display_name,
                                    )
                                    .clicked()
                                {
                                    self.config.font_name = Some(font.display_name.clone());
                                    config_changed = true;
                                }
                            }
                        });
                } else {
                    ui.label("No Nerd Fonts were detected. The app will use the system monospace font.");
                }

                ui.separator();
                ui.heading("Shell");
                ui.add_space(6.0);

                if ui
                    .text_edit_singleline(&mut self.config.shell)
                    .changed()
                {
                    config_changed = true;
                }

                let starship_label = if self.runtime.starship_available() {
                    "Enable Starship prompt"
                } else {
                    "Enable Starship prompt (starship not detected)"
                };

                if ui
                    .checkbox(&mut self.config.enable_starship, starship_label)
                    .changed()
                {
                    config_changed = true;
                }

                ui.label("Shell and Starship changes apply to new tabs and new splits.");
                ui.label("Terminal theme files support JSON or TOML and are copied into the app config folder.");

                if config_changed {
                    self.apply_font_preferences(ctx);
                    self.refresh_theme(ctx);
                    self.save_config();
                }
            });
        self.preferences_open = open;
    }
}

impl eframe::App for PeakTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(16));

        self.workspace.poll();

        let events = ctx.input(|input| input.events.clone());
        if !self.preferences_open {
            self.handle_shortcuts(&events, ctx);
            self.workspace.handle_events(&events);
        }

        self.show_top_bar(ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.theme.surface))
            .show(ctx, |ui| {
                self.workspace.ui(ui, ctx, &self.config, &self.theme);
            });

        if self.preferences_open {
            self.show_preferences(ctx);
        }
    }
}
