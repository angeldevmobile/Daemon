use anstyle::{Color, RgbColor, Style};

use crate::style::{self, paint};

const GLYPHS: [(char, [&str; 7]); 5] = [
    (
        '>',
        [
            "##....", ".##...", "..##..", "...##.", "..##..", ".##...", "##....",
        ],
    ),
    (
        'H',
        [
            "##..##", "##..##", "##..##", "######", "##..##", "##..##", "##..##",
        ],
    ),
    (
        'P',
        [
            "#####.", "##..##", "##..##", "#####.", "##....", "##....", "##....",
        ],
    ),
    (
        'C',
        [
            ".####.", "##..##", "##....", "##....", "##....", "##..##", ".####.",
        ],
    ),
    (
        'D',
        [
            "#####.", "##..##", "##..##", "##..##", "##..##", "##..##", "#####.",
        ],
    ),
];
const WORD: [(char, usize); 6] = [('>', 6), (' ', 1), ('H', 7), ('P', 7), ('C', 7), ('D', 6)];

const FROM: [u8; 3] = [0xB3, 0x92, 0xF0];
const TO: [u8; 3] = [0x56, 0xD4, 0xDD];

const TIPS: [(&str, &str); 3] = [
    (
        "hpcctl audit --unit <name>.service",
        "find what makes it jittery",
    ),
    ("hpcctl explain <rule>", "why it matters and how to fix it"),
    ("hpcctl --help", "all commands"),
];

#[derive(Clone, Copy, PartialEq)]
enum Px {
    Empty,
    Lit(RgbColor),
}

fn pixels() -> Vec<Vec<Px>> {
    let width: usize = WORD.iter().map(|(_, w)| w).sum();
    let mut lit = vec![vec![false; width]; 7];
    let mut col = 0;
    for (ch, advance) in WORD {
        if let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) {
            for (r, row) in rows.iter().enumerate() {
                for (c, b) in row.bytes().enumerate() {
                    lit[r][col + c] |= b == b'#';
                }
            }
        }
        col += advance;
    }

    let color = |c: usize| {
        let t = c as f32 / (width - 1) as f32;
        let mix = |i: usize| (FROM[i] as f32 + (TO[i] as f32 - FROM[i] as f32) * t).round() as u8;
        RgbColor(mix(0), mix(1), mix(2))
    };

    (0..8)
        .map(|r| {
            (0..width)
                .map(|c| {
                    if r < 7 && lit[r][c] {
                        Px::Lit(color(c))
                    } else {
                        Px::Empty
                    }
                })
                .collect()
        })
        .collect()
}

fn art() -> Vec<String> {
    let px = pixels();
    px.chunks(2)
        .map(|pair| {
            let line: String = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(top, bottom)| match (*top, *bottom) {
                    (Px::Empty, Px::Empty) => " ".to_string(),
                    (Px::Lit(t), Px::Empty) => paint(fg(t), '▀'),
                    (Px::Empty, Px::Lit(b)) => paint(fg(b), '▄'),
                    (Px::Lit(t), Px::Lit(b)) if t == b => paint(fg(t), '█'),
                    (Px::Lit(t), Px::Lit(b)) => paint(fg(t).bg_color(Some(Color::Rgb(b))), '▀'),
                })
                .collect();
            line.trim_end().to_string()
        })
        .collect()
}

fn fg(c: RgbColor) -> Style {
    Style::new().fg_color(Some(Color::Rgb(c)))
}

pub fn render() -> String {
    let mut out = vec![String::new()];
    out.extend(art());
    out.push(String::new());
    out.push(format!(
        "{}{}",
        paint(Style::new().bold(), "High-Performance Compute Daemon"),
        paint(style::DIM, format!(" · v{}", env!("CARGO_PKG_VERSION")))
    ));
    out.push(paint(
        style::DIM,
        "Find out why your workload is jittery. Fix it. Prove it.",
    ));
    out.push(String::new());
    out.push("Tips for getting started:".into());
    let width = TIPS.iter().map(|(c, _)| c.len()).max().unwrap_or(0) + 3;
    for (i, (cmd, what)) in TIPS.iter().enumerate() {
        out.push(format!(
            "{}. {}{}",
            i + 1,
            paint(style::ID, format!("{cmd:width$}")),
            paint(style::DIM, what)
        ));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_spells_the_name_in_four_lines() {
        let lines: Vec<String> = art()
            .iter()
            .map(|l| anstream::adapter::strip_str(l).to_string())
            .collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("▀█"));
        assert!(lines.iter().all(|l| l.chars().count() <= 34));
    }

    #[test]
    fn tips_line_up() {
        let text = anstream::adapter::strip_str(&render()).to_string();
        assert!(
            text.contains("1. hpcctl audit --unit <name>.service   find what makes it jittery")
        );
        assert!(text.contains("3. hpcctl --help                        all commands"));
    }
}
