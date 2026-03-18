# Peak Terminal

A native Rust terminal emulator with tabs, split panes, custom themes, and Starship prompt support.

Built with [eframe/egui](https://github.com/emilk/egui) and a real PTY backend via [portable-pty](https://github.com/wez/wezterm/tree/main/pty).

## Requirements

- macOS (arm64 or x86_64)
- [Rust](https://rustup.rs/) 1.93+

## Run

```bash
cargo run
```

For an optimized build:

```bash
cargo run --release
```

## Optional enhancements

**Starship prompt** — install [Starship](https://starship.rs/) and enable it in Preferences. Peak Terminal will bootstrap it automatically for `zsh` and `bash` without touching your shell config files.

```bash
brew install starship
```

**Nerd Fonts** — install any Nerd Font to your macOS font library and it will be auto-detected. Recommended: [MesloLGS NF](https://github.com/romkatv/powerlevel10k#meslo-nerd-font-patched-for-powerlevel10k).

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Cmd+T` | New tab |
| `Cmd+W` | Close tab |
| `Cmd+Shift+[` / `]` | Previous / next tab |
| `Cmd+,` | Open preferences |
| `Cmd+Alt+-` / `=` | Decrease / increase font size |

## Themes

Six built-in themes: **Catppuccin Latte**, **Frappe**, **Macchiato**, **Mocha**, **Peak Dawn**, **Peak Night**.

Import custom themes via Preferences → Import Custom Theme. Supports `.json` and `.toml` files with this schema:

```toml
name = "My Theme"
background = "#1e1e2e"
foreground = "#cdd6f4"
cursor     = "#f5e0dc"
selection  = "#45475a"
surface    = "#181825"
panel      = "#11111b"
accent     = "#89b4fa"
ansi    = ["#45475a","#f38ba8","#a6e3a1","#f9e2af","#89b4fa","#f5c2e7","#94e2d5","#bac2de"]
brights = ["#585b70","#f38ba8","#a6e3a1","#f9e2af","#89b4fa","#f5c2e7","#94e2d5","#cdd6f4"]
```

Config and imported themes are stored in `~/Library/Application Support/dev.Peak.PeakTerminal/`.
