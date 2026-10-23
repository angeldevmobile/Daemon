use anstyle::{Color, RgbColor, Style};
use hpcd_core::{Risk, Severity};

const fn rgb(hex: u32) -> Option<Color> {
    Some(Color::Rgb(RgbColor(
        (hex >> 16) as u8,
        (hex >> 8) as u8,
        hex as u8,
    )))
}

const BADGE_TEXT: Option<Color> = rgb(0x0D1117);

pub const DIM: Style = Style::new().fg_color(rgb(0x7D8590));
pub const HEADING: Style = DIM.bold();
pub const ACCENT: Style = Style::new().fg_color(rgb(0xB392F0)).bold();
pub const ID: Style = Style::new().fg_color(rgb(0x56D4DD));
pub const OK: Style = Style::new().fg_color(rgb(0x3FB950));
pub const OK_BOLD: Style = OK.bold();
pub const BAD: Style = Style::new().fg_color(rgb(0xF85149));
pub const CAUTION: Style = Style::new().fg_color(rgb(0xE3B341));
pub const WARN: Style = Style::new().fg_color(rgb(0xF0883E)).bold();
pub const LINK: Style = Style::new().fg_color(rgb(0x58A6FF));

pub fn badge(severity: Severity) -> Style {
    let (bg, fg) = match severity {
        Severity::High => (0xF85149, BADGE_TEXT),
        Severity::Medium => (0xE3B341, BADGE_TEXT),
        Severity::Low => (0x58A6FF, BADGE_TEXT),
        Severity::Warn => (0xF0883E, BADGE_TEXT),
        Severity::Info => (0x6E7681, rgb(0xD6DCE4)),
    };
    Style::new().bg_color(rgb(bg)).fg_color(fg).bold()
}

pub fn risk(risk: Risk) -> Style {
    match risk {
        Risk::Low => OK,
        Risk::Medium => CAUTION,
        Risk::High => BAD,
    }
}

pub fn paint(style: Style, text: impl std::fmt::Display) -> String {
    format!("{style}{text}{style:#}")
}

pub fn severity_badge(severity: Severity) -> String {
    paint(badge(severity), format!(" {:<4} ", severity.to_string()))
}
