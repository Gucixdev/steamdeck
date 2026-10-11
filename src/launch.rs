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

pub fn has(s: &str, f: Flag) -> bool {
    s.split_whitespace().any(|t| t == f.token())
}

pub fn toggle(s: &str, f: Flag, on: bool) -> String {
    let mut toks: Vec<&str> = s.split_whitespace().collect();
    let present = toks.contains(&f.token());
    if on && !present {
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
    } else if !on && present {
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
