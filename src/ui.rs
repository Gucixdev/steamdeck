//! Look shared by the Steam Deck tools: one dark theme, touch-sized controls (the Deck's screen is
//! 1280x800 and most of the time it is used with fingers or the trackpads).
use iced::theme::Palette;
use iced::{color, Theme};

pub const TEXT_SIZE: f32 = 20.0;
pub const SMALL_SIZE: f32 = 16.0;
pub const TITLE_SIZE: f32 = 28.0;
pub const SPACING: f32 = 12.0;
pub const PAD: f32 = 14.0;

pub const GREEN: iced::Color = color!(0x75b022);
/// Steam light blue (headings, selection)
pub const ACCENT: iced::Color = color!(0x66c0f4);
pub const RED: iced::Color = color!(0xc94545);
pub const AMBER: iced::Color = color!(0xe5a823);
pub const DIM: iced::Color = color!(0x8f98a0);

pub fn theme() -> Theme {
    Theme::custom(
        "Deck".to_string(),
        Palette {
            background: color!(0x1b2838),
            text: color!(0xc7d5e0),
            primary: ACCENT,
            success: GREEN,
            warning: AMBER,
            danger: RED,
        },
    )
}

/// "45 s", "3 min 20 s", "1 h 05 min": a time left or elapsed, for progress lines.
pub fn human_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min {:02} s", secs / 60, secs % 60)
    } else {
        format!("{} h {:02} min", secs / 3600, (secs % 3600) / 60)
    }
}

/// A one-character spinner frame for a counter that goes up.
pub fn spinner(tick: usize) -> &'static str {
    ["|", "/", "-", "\\"][tick % 4]
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
    solid(color!(0x417a9b))
}
/// The Steam green of "Install" and "Play".
pub fn primary_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0x5c7e10))
}
pub fn quiet_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0x2a475e))
}
pub fn danger_button() -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    solid(color!(0xa33e29))
}

use iced::widget::{button as btn, column, container, row, text};
use iced::{Element, Fill, Length};

pub const SIDEBAR_W: f32 = 230.0;

/// Sidebar plus main area, padded the same way in every window.
pub fn shell<'a, M: 'a>(side: Element<'a, M>, main: Element<'a, M>) -> Element<'a, M> {
    container(
        row![
            container(side).width(Length::Fixed(SIDEBAR_W)).height(Fill),
            container(main).width(Fill).height(Fill)
        ]
        .spacing(SPACING * 1.5),
    )
    .padding(PAD)
    .width(Fill)
    .height(Fill)
    .into()
}

/// One full-width entry of the sidebar; the selected one is highlighted.
pub fn nav_item<'a, M: Clone + 'a>(
    label: impl Into<String>,
    selected: bool,
    msg: M,
) -> Element<'a, M> {
    let b = btn(text(label.into()).size(SMALL_SIZE))
        .width(Fill)
        .padding(10)
        .on_press(msg);
    if selected {
        b.style(accent_button()).into()
    } else {
        b.style(quiet_button()).into()
    }
}

/// The top of a page: what this is, in one plain sentence.
pub fn page_title<'a, M: 'a>(title: &str, about: &str) -> Element<'a, M> {
    column![
        text(title.to_string()).size(TITLE_SIZE).color(ACCENT),
        text(about.to_string()).size(SMALL_SIZE).color(DIM)
    ]
    .spacing(4)
    .into()
}

/// A big tile for the home page: a title, a sentence, and one action.
pub fn card<'a, M: Clone + 'a>(
    title: &str,
    about: &str,
    action: &'static str,
    msg: M,
) -> Element<'a, M> {
    container(
        row![
            column![
                text(title.to_string()).size(TEXT_SIZE),
                text(about.to_string()).size(SMALL_SIZE).color(DIM)
            ]
            .spacing(4)
            .width(Fill),
            btn(text(action).size(TEXT_SIZE))
                .padding([10, 20])
                .on_press(msg)
                .style(primary_button()),
        ]
        .spacing(SPACING)
        .align_y(iced::Alignment::Center),
    )
    .padding(PAD)
    .width(Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(color!(0x2a475e))),
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// Header of a table: dim column names; `w` is the relative width of each column.
pub fn table_header<'a, M: 'a>(cols: &[(&str, u16)]) -> Element<'a, M> {
    let mut r = row![].spacing(SPACING).padding([0, 12]);
    for (name, w) in cols {
        r = r.push(
            text(name.to_string())
                .size(SMALL_SIZE)
                .color(DIM)
                .width(Length::FillPortion(*w)),
        );
    }
    r.into()
}

/// One table row: cells with relative widths, alternating background so rows read as rows.
pub fn table_row<'a, M: 'a>(cells: Vec<(Element<'a, M>, u16)>, shaded: bool) -> Element<'a, M> {
    let mut r = row![].spacing(SPACING).align_y(iced::Alignment::Center);
    for (c, w) in cells {
        r = r.push(container(c).width(Length::FillPortion(w)));
    }
    let bg = if shaded {
        color!(0x203245)
    } else {
        color!(0x1b2838)
    };
    container(r)
        .padding([8, 12])
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// A warning tile (yellow) for something the user should not miss.
pub fn warning_card<'a, M: Clone + 'a>(
    title: &str,
    about: &str,
    action: &'static str,
    msg: M,
) -> Element<'a, M> {
    container(
        row![
            column![
                text(title.to_string()).size(TEXT_SIZE).color(AMBER),
                text(about.to_string()).size(SMALL_SIZE)
            ]
            .spacing(4)
            .width(Fill),
            btn(text(action).size(TEXT_SIZE))
                .padding([10, 20])
                .on_press(msg)
                .style(solid(AMBER_DARK)),
        ]
        .spacing(SPACING)
        .align_y(iced::Alignment::Center),
    )
    .padding(PAD)
    .width(Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(color!(0x4a3c0f))),
        border: iced::Border {
            radius: 4.0.into(),
            width: 2.0,
            color: AMBER,
        },
        ..Default::default()
    })
    .into()
}

const AMBER_DARK: iced::Color = color!(0xb8860b);

/// A section title in the main area.
pub fn heading<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into()).size(TEXT_SIZE).color(ACCENT).into()
}

/// One of a few choices shown side by side (default / on / off, default / yes / no).
pub fn segment<'a, M: Clone + 'a>(
    label: &'static str,
    selected: bool,
    danger: bool,
    msg: M,
) -> Element<'a, M> {
    let b = btn(text(label).size(SMALL_SIZE))
        .padding([6, 10])
        .on_press(msg);
    if !selected {
        b.style(quiet_button()).into()
    } else if danger {
        b.style(danger_button()).into()
    } else {
        b.style(accent_button()).into()
    }
}

/// A line of a list: a name with a dim detail line under it, and controls on the right.
pub fn item<'a, M: 'a>(
    name: impl Into<String>,
    detail: impl Into<String>,
    controls: Element<'a, M>,
) -> Element<'a, M> {
    row![
        column![
            text(name.into()).size(TEXT_SIZE),
            text(detail.into()).size(SMALL_SIZE).color(DIM)
        ]
        .spacing(2)
        .width(Fill),
        controls,
    ]
    .spacing(SPACING)
    .align_y(iced::Alignment::Center)
    .into()
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
    #[test]
    fn durations_read_naturally() {
        assert_eq!(human_duration(5), "5 s");
        assert_eq!(human_duration(200), "3 min 20 s");
        assert_eq!(human_duration(3900), "1 h 05 min");
    }
}

// --- the layout both programs share: a sidebar on the left, the working area on the right ---------
