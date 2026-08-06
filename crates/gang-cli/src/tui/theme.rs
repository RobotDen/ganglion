//! Visual theme for the `gang tui` dashboard.
//!
//! Two modes: a teal-accented colour theme and a monochrome/ASCII degrade for
//! `NO_COLOR` (and plain recorders). The theme is a plain data struct with no
//! terminal state, so both variants render headlessly under `TestBackend` and
//! the `NO_COLOR` path is testable without a TTY.

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

/// Whether colour output is suppressed, per the `NO_COLOR` convention
/// (<https://no-color.org>): any non-empty value disables colour.
pub fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// A resolved theme: colours, border glyphs, and status markers. In monochrome
/// mode every colour collapses to the terminal default and borders/markers use
/// ASCII so the dashboard stays legible in a plain recorder.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Whether this is the monochrome/ASCII degrade.
    pub mono: bool,
}

impl Theme {
    /// Build a theme, honouring `NO_COLOR` unless `force_color` overrides it.
    pub fn resolve(force_mono: bool) -> Self {
        Self {
            mono: force_mono || no_color_env(),
        }
    }

    // --- Accent + structural colours ---

    /// Primary teal accent (`#0D9488`) — titles, selection, active chrome.
    pub fn accent(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Rgb(0x0d, 0x94, 0x88))
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Bright teal (`#2dd4bf`) — the live heartbeat pulse + focused title.
    pub fn accent_bright(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(0x2d, 0xd4, 0xbf))
        }
    }

    /// A dim style for idle/secondary text and offline rows.
    pub fn dim(&self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }

    /// Plain body text.
    pub fn text(&self) -> Style {
        Style::default()
    }

    /// ALLOW verdict / live status (green).
    pub fn ok(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Rgb(0x22, 0xc5, 0x5e))
                .add_modifier(Modifier::BOLD)
        }
    }

    /// DENY verdict / error (red).
    pub fn deny(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
                .fg(Color::Rgb(0xef, 0x44, 0x44))
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Amber — transitional peer state / warnings.
    pub fn warn(&self) -> Style {
        if self.mono {
            Style::default()
        } else {
            Style::default().fg(Color::Rgb(0xf5, 0x9e, 0x0b))
        }
    }

    /// The style for the currently selected peer row.
    pub fn selection(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .bg(Color::Rgb(0x0d, 0x94, 0x88))
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// The border glyph set — Unicode rounded in colour mode, ASCII in mono.
    pub fn border_set(&self) -> border::Set<'_> {
        if self.mono {
            border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            }
        } else {
            border::ROUNDED
        }
    }

    // --- Status markers (peer liveness dot) ---

    /// Marker + style for a live peer (`●` / `*`).
    pub fn live_marker(&self) -> (&'static str, Style) {
        if self.mono {
            ("*", self.text())
        } else {
            ("\u{25cf}", self.ok()) // ●
        }
    }

    /// Marker + style for a transitional peer (`◐` / `~`).
    pub fn transitional_marker(&self) -> (&'static str, Style) {
        if self.mono {
            ("~", self.text())
        } else {
            ("\u{25d0}", self.warn()) // ◐
        }
    }

    /// Marker + style for an offline peer (`○` / `.`).
    pub fn offline_marker(&self) -> (&'static str, Style) {
        if self.mono {
            (".", self.dim())
        } else {
            ("\u{25cb}", self.dim()) // ○
        }
    }

    /// A dash/placeholder glyph — em dash in colour mode, ASCII hyphen in mono.
    pub fn dash(&self) -> &'static str {
        if self.mono { "-" } else { "\u{2014}" }
    }

    /// Column header for the "bytes up" tunnel counter.
    pub fn up_label(&self) -> &'static str {
        if self.mono { "up" } else { "\u{2191} up" }
    }

    /// Column header for the "bytes down" tunnel counter.
    pub fn down_label(&self) -> &'static str {
        if self.mono { "down" } else { "\u{2193} down" }
    }

    /// The animated live-heartbeat glyph for the title bar, cycled by `phase`.
    /// Mono mode uses ASCII spinner frames.
    pub fn pulse(&self, phase: usize) -> &'static str {
        if self.mono {
            ["-", "\\", "|", "/"][phase % 4]
        } else {
            // A soft heartbeat pulse.
            ["\u{2661}", "\u{2665}"][phase % 2] // ♡ ♥
        }
    }
}
