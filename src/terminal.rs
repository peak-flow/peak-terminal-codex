use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui::{self, Align2, FontFamily, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use vt100::Parser;

use crate::config::{self, AppConfig};
use crate::theme::{Theme, brighten, dim, vt_color};

const SCROLLBACK_LEN: usize = 10_000;
const SPLITTER_THICKNESS: f32 = 6.0;
const MIN_PANE_SIZE: f32 = 120.0;

pub struct RuntimeSupport {
    starship_path: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
}

pub struct TerminalWorkspace {
    tabs: Vec<WorkspaceTab>,
    sessions: BTreeMap<usize, TerminalSession>,
    active_tab: usize,
    focused_session: Option<usize>,
    next_tab_id: usize,
    next_session_id: usize,
    next_split_id: usize,
    empty_message: Option<String>,
}

struct WorkspaceTab {
    id: usize,
    title: String,
    root: PaneNode,
}

enum PaneNode {
    Leaf(usize),
    Split {
        id: usize,
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum SplitAxis {
    Columns,
    Rows,
}

enum SessionEvent {
    Output(Vec<u8>),
    ReadError(String),
}

pub struct TerminalSession {
    id: usize,
    parser: Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send>,
    receiver: Receiver<SessionEvent>,
    title: String,
    rows: u16,
    cols: u16,
    exited: bool,
    last_exit: Option<String>,
}

impl RuntimeSupport {
    pub fn detect() -> Self {
        let runtime_dir = config::runtime_dir().ok();
        if let Some(dir) = &runtime_dir {
            let _ = config::ensure_dir(dir);
        }

        Self {
            starship_path: find_in_path("starship"),
            runtime_dir,
        }
    }

    pub fn starship_available(&self) -> bool {
        self.starship_path.is_some()
    }

    fn prepare_command(&self, config: &AppConfig) -> Result<CommandBuilder> {
        let shell = PathBuf::from(&config.shell);
        let shell_name = shell
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let mut command = CommandBuilder::new(OsString::from(&config.shell));
        command.cwd(default_working_directory());
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "PeakTerminal");

        if config.enable_starship && self.starship_available() {
            match shell_name.as_str() {
                "zsh" => {
                    let zsh_dir = self.prepare_zsh_wrapper()?;
                    command.arg("-i");
                    command.env("ZDOTDIR", zsh_dir.as_os_str());
                }
                "bash" => {
                    let bash_rc = self.prepare_bash_wrapper()?;
                    command.arg("--rcfile");
                    command.arg(bash_rc.as_os_str());
                    command.arg("-i");
                }
                _ => {
                    command.arg("-i");
                }
            }
        } else {
            command.arg("-i");
        }

        Ok(command)
    }

    fn prepare_zsh_wrapper(&self) -> Result<PathBuf> {
        let dir = self
            .runtime_dir
            .as_ref()
            .context("Missing runtime directory for zsh wrapper")?
            .join("zsh");
        config::ensure_dir(&dir)?;

        let starship = self
            .starship_path
            .as_ref()
            .context("Starship was not detected")?;

        let home = config::home_dir()
            .ok_or_else(|| anyhow!("Unable to determine HOME for zsh wrapper"))?;
        let source_path = home.join(".zshrc");
        let wrapper = format!(
            r#"
export PEAK_TERMINAL_STARSHIP=1
if [ -f "{source}" ]; then
  source "{source}"
fi
if command -v "{starship}" >/dev/null 2>&1; then
  eval "$("{starship}" init zsh)"
fi
"#,
            source = source_path.display(),
            starship = starship.display(),
        );

        fs::write(dir.join(".zshrc"), wrapper).context("Failed to write zsh wrapper")?;
        Ok(dir)
    }

    fn prepare_bash_wrapper(&self) -> Result<PathBuf> {
        let dir = self
            .runtime_dir
            .as_ref()
            .context("Missing runtime directory for bash wrapper")?;
        config::ensure_dir(dir)?;

        let starship = self
            .starship_path
            .as_ref()
            .context("Starship was not detected")?;

        let home = config::home_dir()
            .ok_or_else(|| anyhow!("Unable to determine HOME for bash wrapper"))?;
        let bashrc = home.join(".bashrc");
        let bash_profile = home.join(".bash_profile");
        let wrapper_path = dir.join("peak_bashrc");

        let wrapper = format!(
            r#"
export PEAK_TERMINAL_STARSHIP=1
if [ -f "{bashrc}" ]; then
  source "{bashrc}"
elif [ -f "{bash_profile}" ]; then
  source "{bash_profile}"
fi
if command -v "{starship}" >/dev/null 2>&1; then
  eval "$("{starship}" init bash)"
fi
"#,
            bashrc = bashrc.display(),
            bash_profile = bash_profile.display(),
            starship = starship.display(),
        );

        fs::write(&wrapper_path, wrapper).context("Failed to write bash wrapper")?;
        Ok(wrapper_path)
    }
}

impl TerminalWorkspace {
    pub fn new(config: &AppConfig, runtime: &RuntimeSupport) -> Result<Self> {
        let mut workspace = Self {
            tabs: Vec::new(),
            sessions: BTreeMap::new(),
            active_tab: 0,
            focused_session: None,
            next_tab_id: 1,
            next_session_id: 1,
            next_split_id: 1,
            empty_message: None,
        };

        workspace.new_tab(config, runtime)?;
        Ok(workspace)
    }

    pub fn empty(message: String) -> Self {
        Self {
            tabs: Vec::new(),
            sessions: BTreeMap::new(),
            active_tab: 0,
            focused_session: None,
            next_tab_id: 1,
            next_session_id: 1,
            next_split_id: 1,
            empty_message: Some(message),
        }
    }

    pub fn poll(&mut self) {
        for session in self.sessions.values_mut() {
            session.poll();
        }

        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.title = format!("Tab {}", index + 1);
            if let Some(session_id) = tab.root.primary_leaf() {
                if let Some(session) = self.sessions.get(&session_id) {
                    if !session.title.is_empty() {
                        tab.title = session.title.clone();
                    }
                }
            }
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        config: &AppConfig,
        theme: &Theme,
    ) {
        if self.tabs.is_empty() {
            let rect = ui.available_rect_before_wrap();
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(rect, 12.0, theme.background);

            let message = self.empty_message.clone().unwrap_or_else(|| {
                "No active sessions. Create a new tab to start a shell.".to_owned()
            });
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                message,
                FontId::new(18.0, FontFamily::Proportional),
                theme.foreground,
            );
            return;
        }

        let active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        let rect = ui.available_rect_before_wrap();
        let tab = &mut self.tabs[active_tab];
        Self::render_node(
            ui,
            ctx,
            rect,
            &mut tab.root,
            &mut self.sessions,
            &mut self.focused_session,
            config,
            theme,
        );
    }

    pub fn handle_events(&mut self, events: &[egui::Event]) {
        let Some(session_id) = self.focused_session else {
            return;
        };
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };

        for event in events {
            match event {
                egui::Event::Text(text) => {
                    if !text.is_empty() {
                        session.scroll_to_bottom();
                        session.send_bytes(text.as_bytes());
                    }
                }
                egui::Event::Paste(text) => {
                    session.scroll_to_bottom();
                    if session.bracketed_paste() {
                        let mut bytes = Vec::with_capacity(text.len() + 16);
                        bytes.extend_from_slice(b"\x1b[200~");
                        bytes.extend_from_slice(text.as_bytes());
                        bytes.extend_from_slice(b"\x1b[201~");
                        session.send_bytes(&bytes);
                    } else {
                        session.send_bytes(text.as_bytes());
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.command {
                        continue;
                    }

                    if let Some(bytes) =
                        encode_key(*key, *modifiers, session.application_cursor_mode())
                    {
                        session.scroll_to_bottom();
                        session.send_bytes(&bytes);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn new_tab(&mut self, config: &AppConfig, runtime: &RuntimeSupport) -> Result<()> {
        let session_id = self.spawn_session(config, runtime)?;
        self.tabs.push(WorkspaceTab {
            id: self.next_tab_id,
            title: format!("Tab {}", self.next_tab_id),
            root: PaneNode::Leaf(session_id),
        });
        self.active_tab = self.tabs.len().saturating_sub(1);
        self.focused_session = Some(session_id);
        self.next_tab_id += 1;
        self.empty_message = None;
        Ok(())
    }

    pub fn split_focused(
        &mut self,
        axis: SplitAxis,
        config: &AppConfig,
        runtime: &RuntimeSupport,
    ) -> Result<()> {
        let Some(focus) = self.focused_session.or_else(|| {
            self.tabs
                .get(self.active_tab)
                .and_then(|tab| tab.root.primary_leaf())
        }) else {
            return self.new_tab(config, runtime);
        };

        let new_session_id = self.spawn_session(config, runtime)?;
        let split_id = self.next_split_id;
        self.next_split_id += 1;

        let tab = self
            .tabs
            .get_mut(self.active_tab)
            .ok_or_else(|| anyhow!("Unable to access the active tab"))?;

        if tab
            .root
            .replace_with_split(focus, new_session_id, split_id, axis)
        {
            self.focused_session = Some(new_session_id);
            Ok(())
        } else {
            Err(anyhow!("Unable to split the focused pane"))
        }
    }

    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }

        let tab = self.tabs.remove(self.active_tab);
        for session_id in tab.root.collect_leaves() {
            self.sessions.remove(&session_id);
        }

        if self.tabs.is_empty() {
            self.active_tab = 0;
            self.focused_session = None;
        } else {
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
            self.focused_session = self.tabs[self.active_tab].root.primary_leaf();
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    pub fn tabs(&self) -> impl Iterator<Item = (usize, String, usize)> + '_ {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| (index, tab.title.clone(), tab.id))
    }

    pub fn set_active_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
            self.focused_session = self.tabs[index].root.primary_leaf();
        }
    }

    fn spawn_session(&mut self, config: &AppConfig, runtime: &RuntimeSupport) -> Result<usize> {
        let id = self.next_session_id;
        self.next_session_id += 1;

        let session = TerminalSession::spawn(id, config, runtime)?;
        self.sessions.insert(id, session);
        Ok(id)
    }

    fn render_node(
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: Rect,
        node: &mut PaneNode,
        sessions: &mut BTreeMap<usize, TerminalSession>,
        focused_session: &mut Option<usize>,
        config: &AppConfig,
        theme: &Theme,
    ) {
        match node {
            PaneNode::Leaf(session_id) => {
                if let Some(session) = sessions.get_mut(session_id) {
                    Self::render_session(ui, ctx, rect, session, focused_session, config, theme);
                }
            }
            PaneNode::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let painter = ui.painter().with_clip_rect(rect);
                let (first_rect, splitter_rect, second_rect) =
                    split_rects(rect, *axis, *ratio, SPLITTER_THICKNESS);

                let response = ui.interact(
                    splitter_rect,
                    Id::new(("splitter", *id)),
                    Sense::click_and_drag(),
                );

                if response.dragged() {
                    if let Some(pointer) = ctx.pointer_latest_pos() {
                        *ratio = match axis {
                            SplitAxis::Columns => ((pointer.x - rect.left())
                                / (rect.width() - SPLITTER_THICKNESS))
                                .clamp(0.15, 0.85),
                            SplitAxis::Rows => ((pointer.y - rect.top())
                                / (rect.height() - SPLITTER_THICKNESS))
                                .clamp(0.15, 0.85),
                        };
                    }
                }

                let splitter_color = if response.dragged() || response.hovered() {
                    theme.accent
                } else {
                    brighten(theme.panel, 1.15)
                };
                painter.rect_filled(splitter_rect, 2.0, splitter_color);

                Self::render_node(
                    ui,
                    ctx,
                    first_rect,
                    first,
                    sessions,
                    focused_session,
                    config,
                    theme,
                );
                Self::render_node(
                    ui,
                    ctx,
                    second_rect,
                    second,
                    sessions,
                    focused_session,
                    config,
                    theme,
                );
            }
        }
    }

    fn render_session(
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: Rect,
        session: &mut TerminalSession,
        focused_session: &mut Option<usize>,
        config: &AppConfig,
        theme: &Theme,
    ) {
        let response = ui.interact(rect, Id::new(("pane", session.id)), Sense::click());
        if response.clicked() {
            *focused_session = Some(session.id);
        }

        let is_focused = *focused_session == Some(session.id);
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 8.0, theme.background);
        painter.rect_stroke(
            rect,
            8.0,
            Stroke::new(
                if is_focused { 2.0 } else { 1.0 },
                if is_focused {
                    theme.accent
                } else {
                    theme.panel
                },
            ),
            egui::StrokeKind::Outside,
        );

        if response.hovered() {
            let scroll_y = ctx.input(|input| input.raw_scroll_delta.y);
            let delta = (-scroll_y / 32.0).round() as i32;
            if delta != 0 {
                session.scroll(delta);
            }
        }

        let inner = rect.shrink2(Vec2::new(8.0, 8.0));
        let font_id = FontId::new(config.font_size, FontFamily::Monospace);
        let glyph = painter.layout_no_wrap("W".to_owned(), font_id.clone(), theme.foreground);
        let cell_width = glyph.size().x.max(7.0);
        let cell_height = (glyph.size().y * 1.15).max(config.font_size + 2.0);
        let rows = (inner.height() / cell_height).floor().max(2.0) as u16;
        let cols = (inner.width() / cell_width).floor().max(2.0) as u16;

        session.resize(rows, cols, cell_width, cell_height);

        let screen = session.parser.screen();
        let blink_on = ((ctx.input(|input| input.time) * 2.0) as i64) % 2 == 0;

        for row in 0..rows {
            let y = inner.top() + row as f32 * cell_height;
            let mut col = 0u16;
            while col < cols {
                let Some(cell) = screen.cell(row, col) else {
                    col += 1;
                    continue;
                };

                if cell.is_wide_continuation() {
                    col += 1;
                    continue;
                }

                let width_cells = if cell.is_wide() { 2 } else { 1 };
                let x = inner.left() + col as f32 * cell_width;
                let cell_rect = Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new(cell_width * width_cells as f32, cell_height),
                );

                let mut fg = vt_color(theme, cell.fgcolor(), false);
                let mut bg = vt_color(theme, cell.bgcolor(), true);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.bold() {
                    fg = brighten(fg, 1.2);
                }
                if cell.dim() {
                    fg = dim(fg, 0.8);
                }

                if bg != theme.background {
                    painter.rect_filled(cell_rect, 0.0, bg);
                }

                if cell.has_contents() {
                    painter.text(
                        cell_rect.left_top(),
                        Align2::LEFT_TOP,
                        cell.contents(),
                        font_id.clone(),
                        fg,
                    );
                }

                if cell.underline() {
                    let underline_y = cell_rect.bottom() - 2.0;
                    painter.line_segment(
                        [
                            Pos2::new(cell_rect.left(), underline_y),
                            Pos2::new(cell_rect.right(), underline_y),
                        ],
                        Stroke::new(1.0, fg),
                    );
                }

                col += width_cells;
            }
        }

        if is_focused && blink_on && !screen.hide_cursor() {
            let (cursor_row, cursor_col) = screen.cursor_position();
            if cursor_row < rows && cursor_col < cols {
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(
                        inner.left() + cursor_col as f32 * cell_width,
                        inner.top() + cursor_row as f32 * cell_height,
                    ),
                    Vec2::new(cell_width, cell_height),
                );
                painter.rect_filled(cursor_rect, 1.0, theme.cursor);

                if let Some(cell) = screen.cell(cursor_row, cursor_col) {
                    if cell.has_contents() {
                        painter.text(
                            cursor_rect.left_top(),
                            Align2::LEFT_TOP,
                            cell.contents(),
                            font_id.clone(),
                            theme.background,
                        );
                    }
                }
            }
        }

        if let Some(message) = &session.last_exit {
            let banner_rect = Rect::from_min_max(
                Pos2::new(inner.left(), inner.bottom() - 28.0),
                Pos2::new(inner.right(), inner.bottom()),
            );
            painter.rect_filled(banner_rect, 4.0, brighten(theme.panel, 1.12));
            painter.text(
                banner_rect.left_center() + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                message,
                FontId::new(12.0, FontFamily::Proportional),
                theme.foreground,
            );
        }
    }
}

impl PaneNode {
    fn primary_leaf(&self) -> Option<usize> {
        match self {
            PaneNode::Leaf(session_id) => Some(*session_id),
            PaneNode::Split { first, .. } => first.primary_leaf(),
        }
    }

    fn replace_with_split(
        &mut self,
        target: usize,
        new_session_id: usize,
        split_id: usize,
        axis: SplitAxis,
    ) -> bool {
        match self {
            PaneNode::Leaf(session_id) if *session_id == target => {
                let original = *session_id;
                *self = PaneNode::Split {
                    id: split_id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(PaneNode::Leaf(original)),
                    second: Box::new(PaneNode::Leaf(new_session_id)),
                };
                true
            }
            PaneNode::Leaf(_) => false,
            PaneNode::Split { first, second, .. } => {
                first.replace_with_split(target, new_session_id, split_id, axis)
                    || second.replace_with_split(target, new_session_id, split_id, axis)
            }
        }
    }

    fn collect_leaves(&self) -> Vec<usize> {
        match self {
            PaneNode::Leaf(session_id) => vec![*session_id],
            PaneNode::Split { first, second, .. } => {
                let mut leaves = first.collect_leaves();
                leaves.extend(second.collect_leaves());
                leaves
            }
        }
    }
}

impl TerminalSession {
    fn spawn(id: usize, config: &AppConfig, runtime: &RuntimeSupport) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to allocate PTY")?;

        let command = runtime.prepare_command(config)?;
        let child = pair
            .slave
            .spawn_command(command.clone())
            .with_context(|| format!("Failed to spawn shell with {:?}", command.get_argv()))?;

        let writer = pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let (sender, receiver) = unbounded();

        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(SessionEvent::ReadError(
                            "Shell process closed the PTY".to_owned(),
                        ));
                        break;
                    }
                    Ok(read) => {
                        let _ = sender.send(SessionEvent::Output(buffer[..read].to_vec()));
                    }
                    Err(error) => {
                        let _ = sender.send(SessionEvent::ReadError(error.to_string()));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            id,
            parser: Parser::new(30, 120, SCROLLBACK_LEN),
            master: pair.master,
            writer,
            child,
            receiver,
            title: String::new(),
            rows: 30,
            cols: 120,
            exited: false,
            last_exit: None,
        })
    }

    fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                SessionEvent::Output(bytes) => {
                    self.parser.process(&bytes);
                }
                SessionEvent::ReadError(error) => {
                    if self.last_exit.is_none() {
                        self.last_exit = Some(error);
                    }
                }
            }
        }

        if !self.exited {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exited = true;
                    self.last_exit = Some(format!("Process exited with status: {status:?}"));
                }
                Ok(None) => {}
                Err(error) => {
                    self.exited = true;
                    self.last_exit = Some(format!("Failed to query child status: {error}"));
                }
            }
        }
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    fn resize(&mut self, rows: u16, cols: u16, cell_width: f32, cell_height: f32) {
        if self.rows == rows && self.cols == cols {
            return;
        }

        self.rows = rows;
        self.cols = cols;
        self.parser.screen_mut().set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: (cols as f32 * cell_width) as u16,
            pixel_height: (rows as f32 * cell_height) as u16,
        });
    }

    fn application_cursor_mode(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    fn scroll(&mut self, delta: i32) {
        let current = self.parser.screen().scrollback() as i32;
        let next = (current + delta).max(0) as usize;
        self.parser.screen_mut().set_scrollback(next);
    }

    fn scroll_to_bottom(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn split_rects(rect: Rect, axis: SplitAxis, ratio: f32, thickness: f32) -> (Rect, Rect, Rect) {
    match axis {
        SplitAxis::Columns => {
            let span = (rect.width() - thickness).max(MIN_PANE_SIZE * 2.0);
            let first_width = (span * ratio).clamp(MIN_PANE_SIZE, span - MIN_PANE_SIZE);
            let splitter_left = rect.left() + first_width;
            let first = Rect::from_min_max(rect.min, Pos2::new(splitter_left, rect.bottom()));
            let splitter = Rect::from_min_max(
                Pos2::new(splitter_left, rect.top()),
                Pos2::new(splitter_left + thickness, rect.bottom()),
            );
            let second = Rect::from_min_max(Pos2::new(splitter.right(), rect.top()), rect.max);
            (first, splitter, second)
        }
        SplitAxis::Rows => {
            let span = (rect.height() - thickness).max(MIN_PANE_SIZE * 2.0);
            let first_height = (span * ratio).clamp(MIN_PANE_SIZE, span - MIN_PANE_SIZE);
            let splitter_top = rect.top() + first_height;
            let first = Rect::from_min_max(rect.min, Pos2::new(rect.right(), splitter_top));
            let splitter = Rect::from_min_max(
                Pos2::new(rect.left(), splitter_top),
                Pos2::new(rect.right(), splitter_top + thickness),
            );
            let second = Rect::from_min_max(Pos2::new(rect.left(), splitter.bottom()), rect.max);
            (first, splitter, second)
        }
    }
}

fn encode_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    if modifiers.ctrl {
        if let Some(byte) = control_key(key) {
            return Some(vec![byte]);
        }
    }

    let bytes = match key {
        egui::Key::Enter => b"\r".to_vec(),
        egui::Key::Tab => {
            if modifiers.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        egui::Key::Backspace => vec![0x7f],
        egui::Key::Escape => vec![0x1b],
        egui::Key::ArrowUp => {
            if application_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        egui::Key::ArrowDown => {
            if application_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        egui::Key::ArrowRight => {
            if application_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        egui::Key::ArrowLeft => {
            if application_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        egui::Key::Home => b"\x1b[H".to_vec(),
        egui::Key::End => b"\x1b[F".to_vec(),
        egui::Key::Insert => b"\x1b[2~".to_vec(),
        egui::Key::Delete => b"\x1b[3~".to_vec(),
        egui::Key::PageUp => b"\x1b[5~".to_vec(),
        egui::Key::PageDown => b"\x1b[6~".to_vec(),
        egui::Key::F1 => b"\x1bOP".to_vec(),
        egui::Key::F2 => b"\x1bOQ".to_vec(),
        egui::Key::F3 => b"\x1bOR".to_vec(),
        egui::Key::F4 => b"\x1bOS".to_vec(),
        egui::Key::F5 => b"\x1b[15~".to_vec(),
        egui::Key::F6 => b"\x1b[17~".to_vec(),
        egui::Key::F7 => b"\x1b[18~".to_vec(),
        egui::Key::F8 => b"\x1b[19~".to_vec(),
        egui::Key::F9 => b"\x1b[20~".to_vec(),
        egui::Key::F10 => b"\x1b[21~".to_vec(),
        egui::Key::F11 => b"\x1b[23~".to_vec(),
        egui::Key::F12 => b"\x1b[24~".to_vec(),
        _ => return None,
    };

    Some(bytes)
}

fn control_key(key: egui::Key) -> Option<u8> {
    let byte = match key {
        egui::Key::A => 0x01,
        egui::Key::B => 0x02,
        egui::Key::C => 0x03,
        egui::Key::D => 0x04,
        egui::Key::E => 0x05,
        egui::Key::F => 0x06,
        egui::Key::G => 0x07,
        egui::Key::H => 0x08,
        egui::Key::I => 0x09,
        egui::Key::J => 0x0a,
        egui::Key::K => 0x0b,
        egui::Key::L => 0x0c,
        egui::Key::M => 0x0d,
        egui::Key::N => 0x0e,
        egui::Key::O => 0x0f,
        egui::Key::P => 0x10,
        egui::Key::Q => 0x11,
        egui::Key::R => 0x12,
        egui::Key::S => 0x13,
        egui::Key::T => 0x14,
        egui::Key::U => 0x15,
        egui::Key::V => 0x16,
        egui::Key::W => 0x17,
        egui::Key::X => 0x18,
        egui::Key::Y => 0x19,
        egui::Key::Z => 0x1a,
        egui::Key::OpenBracket => 0x1b,
        egui::Key::Backslash => 0x1c,
        egui::Key::CloseBracket => 0x1d,
        egui::Key::Num6 => 0x1e,
        egui::Key::Slash => 0x1f,
        egui::Key::Space => 0x00,
        _ => return None,
    };

    Some(byte)
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for path in std::env::split_paths(&paths) {
        let candidate = path.join(program);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn default_working_directory() -> PathBuf {
    config::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf())
}
