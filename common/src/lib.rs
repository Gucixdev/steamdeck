//! Look shared by the Steam Deck tools: one dark theme, touch-sized controls (the Deck's screen is
//! 1280x800 and most of the time it is used with fingers or the trackpads).
use iced::theme::Palette;
use iced::{color, Theme};

pub const TEXT_SIZE: f32 = 20.0;
pub const SMALL_SIZE: f32 = 16.0;
pub const TITLE_SIZE: f32 = 28.0;
pub const SPACING: f32 = 12.0;
pub const PAD: f32 = 14.0;

pub const GREEN: iced::Color = color!(0x00ff41);
pub const PURPLE: iced::Color = color!(0x9944ff);
pub const RED: iced::Color = color!(0xff3b3b);
pub const AMBER: iced::Color = color!(0xffae00);
pub const DIM: iced::Color = color!(0x8a8a99);

pub fn theme() -> Theme {
    Theme::custom(
        "Deck".to_string(),
        Palette {
            background: color!(0x0b0b10),
            text: color!(0xe6e6ee),
            primary: PURPLE,
            success: GREEN,
            warning: AMBER,
            danger: RED,
        },
    )
}

/// "1.4 GB", "312 MB", "12 KB": binary units, one decimal above a megabyte.
pub fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", n, U[0])
    } else if i == 1 {
        format!("{:.0} {}", v, U[i])
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

use iced::widget::button;
use iced::{Background, Border, Theme as IcedTheme};

/// A solid button: white text on `bg`, a little lighter when hovered, dimmed when it has no action.
pub fn solid(bg: iced::Color) -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    move |_theme, status| {
        let (bg, text) = match status {
            button::Status::Active => (bg, iced::Color::WHITE),
            button::Status::Hovered | button::Status::Pressed => (
                iced::Color {
                    r: (bg.r + 0.12).min(1.0),
                    g: (bg.g + 0.12).min(1.0),
                    b: (bg.b + 0.12).min(1.0),
                    a: 1.0,
                },
                iced::Color::WHITE,
            ),
            button::Status::Disabled => (
                iced::Color { a: 0.35, ..bg },
                iced::Color {
                    a: 0.6,
                    ..iced::Color::WHITE
                },
            ),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: text,
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

pub fn accent_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0x7b3fe4))
}
pub fn quiet_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0x2a2a38))
}
pub fn danger_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0xc0392b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bytes_are_human() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(1536 * 1024 * 1024), "1.5 GB");
    }
}
