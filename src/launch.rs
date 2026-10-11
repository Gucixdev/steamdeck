//! Steam launch-option strings as checkboxes: GameMode, MangoHud, FSR and Proton log are tokens in
//! the string (`FLAG=1 wrapper %command% args`); turning one on or off edits only that token and keeps
//! everything else the user typed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    GameMode,
    MangoHud,
    Fsr,
    ProtonLog,
}

pub const FLAGS: [Flag; 4] = [Flag::GameMode, Flag::MangoHud, Flag::Fsr, Flag::ProtonLog];

impl Flag {
    pub fn label(self) -> &'static str {
        match self {
            Flag::GameMode => "GameMode",
            Flag::MangoHud => "MangoHud",
            Flag::Fsr => "FSR",
            Flag::ProtonLog => "Proton log",
        }
    }

    fn token(self) -> &'static str {
        match self {
            Flag::GameMode => "gamemoderun",
            Flag::MangoHud => "mangohud",
            Flag::Fsr => "WINE_FULLSCREEN_FSR=1",
            Flag::ProtonLog => "PROTON_LOG=1",
        }
    }

    /// environment variables go first, wrappers directly in front of %command%
    fn is_env(self) -> bool {
        matches!(self, Flag::Fsr | Flag::ProtonLog)
    }
}

const CMD: &str = "%command%";

/// Words of `s`, split on whitespace; a double-quoted part (with `\"` inside) stays in one word, so
/// `--name "A  B"` is two words and its inner spaces are never touched.
fn words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut quoted = false;
    let mut esc = false;
    for (i, c) in s.char_indices() {
        if quoted {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            if c == '"' {
                quoted = true;
            }
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

pub fn has(s: &str, f: Flag) -> bool {
    words(s).contains(&f.token())
}

/// Turn one flag on or off. A string that needs no change is returned exactly as it was typed.
pub fn toggle(s: &str, f: Flag, on: bool) -> String {
    let mut toks = words(s);
    let present = toks.contains(&f.token());
    if on == present {
        return s.to_string();
    }
    if on {
        if !toks.contains(&CMD) {
            // only arguments were set: the command goes in front of them
            toks.insert(0, CMD);
        }
        let at = toks.iter().position(|t| *t == CMD).unwrap_or(0);
        if f.is_env() {
            toks.insert(0, f.token());
        } else {
            toks.insert(at, f.token());
        }
    } else {
        toks.retain(|t| *t != f.token());
    }
    if toks == [CMD] {
        return String::new();
    }
    toks.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_read_from_the_string() {
        let s = "WINE_FULLSCREEN_FSR=1 gamemoderun %command% -windowed";
        assert!(has(s, Flag::Fsr) && has(s, Flag::GameMode));
        assert!(!has(s, Flag::MangoHud) && !has(s, Flag::ProtonLog));
        assert!(
            !has("gamemoderunner %command%", Flag::GameMode),
            "whole words only"
        );
    }

    #[test]
    fn turning_flags_on_builds_a_valid_command_in_order() {
        let s = toggle("", Flag::GameMode, true);
        assert_eq!(s, "gamemoderun %command%");
        let s = toggle(&s, Flag::Fsr, true);
        assert_eq!(s, "WINE_FULLSCREEN_FSR=1 gamemoderun %command%");
        let s = toggle(&s, Flag::MangoHud, true);
        assert_eq!(s, "WINE_FULLSCREEN_FSR=1 gamemoderun mangohud %command%");
        assert_eq!(
            toggle(&s, Flag::Fsr, true),
            s,
            "turning on twice changes nothing"
        );
    }

    #[test]
    fn everything_the_user_typed_survives() {
        let s = "DXVK_HUD=fps my-wrapper %command% -fullscreen --lang \"en\"";
        let on = toggle(s, Flag::MangoHud, true);
        assert!(
            on.contains("DXVK_HUD=fps") && on.contains("my-wrapper") && on.contains("-fullscreen")
        );
        assert!(has(&on, Flag::MangoHud));
        assert_eq!(toggle(&on, Flag::MangoHud, false), s);
    }

    #[test]
    fn plain_arguments_get_a_command_in_front() {
        assert_eq!(
            toggle("-windowed", Flag::GameMode, true),
            "gamemoderun %command% -windowed"
        );
    }

    #[test]
    fn turning_the_last_flag_off_leaves_an_empty_string() {
        assert_eq!(toggle("gamemoderun %command%", Flag::GameMode, false), "");
        assert_eq!(toggle("", Flag::GameMode, false), "");
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    #[test]
    fn a_string_that_needs_no_change_is_returned_untouched() {
        let s = "  FOO=1   my-wrapper  %command%   -x  ";
        assert_eq!(toggle(s, Flag::MangoHud, false), s);
        assert_eq!(
            toggle("gamemoderun  %command%", Flag::GameMode, true),
            "gamemoderun  %command%"
        );
    }

    #[test]
    fn spaces_inside_quotes_are_never_collapsed() {
        let s = "%command% --name \"A  B\" -x";
        let on = toggle(s, Flag::GameMode, true);
        assert_eq!(on, "gamemoderun %command% --name \"A  B\" -x");
        assert_eq!(toggle(&on, Flag::GameMode, false), s);
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_word() {
        let s = "%command% --msg \"say \\\"hi  there\\\"\"";
        let on = toggle(s, Flag::MangoHud, true);
        assert!(on.ends_with("--msg \"say \\\"hi  there\\\"\""), "{on}");
    }

    #[test]
    fn a_flag_inside_a_quoted_argument_is_not_a_flag() {
        assert!(!has("%command% --title \"mangohud rocks\"", Flag::MangoHud));
        assert!(!has("%command% --title mangohud-x", Flag::MangoHud));
    }

    #[test]
    fn all_four_on_then_all_four_off_gives_the_empty_string() {
        let mut s = String::new();
        for f in FLAGS {
            s = toggle(&s, f, true);
            assert!(has(&s, f), "{s}");
        }
        assert_eq!(s.matches("%command%").count(), 1, "{s}");
        assert!(s.ends_with("%command%"), "{s}");
        for f in FLAGS {
            s = toggle(&s, f, false);
        }
        assert_eq!(s, "");
    }

    #[test]
    fn environment_variables_stay_in_front_of_wrappers() {
        let s = toggle("gamemoderun %command%", Flag::ProtonLog, true);
        assert_eq!(s, "PROTON_LOG=1 gamemoderun %command%");
    }

    #[test]
    fn an_unterminated_quote_does_not_panic_or_lose_text() {
        let s = "%command% --name \"open";
        let on = toggle(s, Flag::Fsr, true);
        assert!(on.ends_with("--name \"open"), "{on}");
    }

    #[test]
    fn turning_off_keeps_a_command_the_user_has_arguments_for() {
        assert_eq!(
            toggle("mangohud %command% -windowed", Flag::MangoHud, false),
            "%command% -windowed"
        );
    }
}
