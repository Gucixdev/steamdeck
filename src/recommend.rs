//! A suggested Proton for a game. There is no oracle for "the right Proton"; this is a plain rule over
//! what is known locally and, when the user asked for it, the ProtonDB rating:
//!
//! * rated platinum or gold: Steam's default already works, so do not suggest a change (and if a custom
//!   tool is set, suggest going back to the default);
//! * silver, bronze, borked, pending or unknown: the newest installed Proton-GE (it carries the extra
//!   fixes and media codecs that the usual failures need), else Valve's Proton Experimental.
//!
//! The reason is always shown, so the suggestion can be judged and not just trusted.

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// the tool to use; empty means "Steam default"
    pub tool: String,
    pub why: String,
}

/// `installed` is the tool names of the community Protons, newest first; `valve` are Valve's own.
pub fn suggest(
    tier: Option<&str>,
    current: Option<&str>,
    installed: &[String],
    valve: &[String],
) -> Option<Suggestion> {
    let ge = installed
        .iter()
        .find(|n| n.to_lowercase().starts_with("ge-proton"))
        .cloned();
    let experimental = valve
        .iter()
        .find(|n| n.as_str() == "proton_experimental")
        .cloned();
    let best = ge.clone().or(experimental.clone());
    let s = match tier {
        Some("platinum") | Some("gold") => {
            // it works as it is; only a leftover custom choice is worth undoing
            current?;
            Suggestion {
                tool: String::new(),
                why: format!(
                    "ProtonDB rates it {}: Steam's default works",
                    tier.unwrap_or("")
                ),
            }
        }
        Some(t @ ("silver" | "bronze" | "borked")) => {
            let tool = best?;
            Suggestion {
                why: format!("ProtonDB rates it {t}: {tool} has the most fixes and codecs"),
                tool,
            }
        }
        _ => {
            let tool = best?;
            Suggestion {
                why: format!("no rating yet: {tool} is the safest bet"),
                tool,
            }
        }
    };
    // nothing to suggest when it is set that way already
    let now = current.unwrap_or("");
    if s.tool == now {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> (Vec<String>, Vec<String>) {
        (
            vec![
                "GE-Proton10-20".into(),
                "GE-Proton9-27".into(),
                "proton-cachyos-slr".into(),
            ],
            vec!["proton_experimental".into(), "proton_9".into()],
        )
    }

    #[test]
    fn a_game_that_works_is_left_on_the_default() {
        let (ge, v) = tools();
        assert_eq!(suggest(Some("platinum"), None, &ge, &v), None);
        assert_eq!(suggest(Some("gold"), None, &ge, &v), None);
        let s = suggest(Some("gold"), Some("GE-Proton9-27"), &ge, &v).unwrap();
        assert_eq!(s.tool, "", "go back to the default");
    }

    #[test]
    fn a_troubled_game_gets_the_newest_ge_and_the_reason() {
        let (ge, v) = tools();
        for t in ["silver", "bronze", "borked"] {
            let s = suggest(Some(t), None, &ge, &v).unwrap();
            assert_eq!(s.tool, "GE-Proton10-20");
            assert!(s.why.contains(t));
        }
        let unknown = suggest(None, None, &ge, &v).unwrap();
        assert_eq!(unknown.tool, "GE-Proton10-20");
        assert!(unknown.why.contains("no rating"));
    }

    #[test]
    fn without_a_ge_it_falls_back_to_experimental_and_without_anything_it_says_nothing() {
        let (_, v) = tools();
        let s = suggest(
            Some("silver"),
            None,
            &["proton-cachyos-slr".to_string()],
            &v,
        )
        .unwrap();
        assert_eq!(s.tool, "proton_experimental");
        assert_eq!(suggest(Some("silver"), None, &[], &[]), None);
    }

    #[test]
    fn nothing_is_suggested_when_it_is_already_set() {
        let (ge, v) = tools();
        assert_eq!(
            suggest(Some("borked"), Some("GE-Proton10-20"), &ge, &v),
            None
        );
    }
}
