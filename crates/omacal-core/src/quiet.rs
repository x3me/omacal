//! Quiet titles (#216): events kept on the calendar, where they do their job,
//! but left out of the menu bar and the reminders.
//!
//! The case that asked for it: a block called "Unavailable" put on a work
//! calendar so nobody books the hour. It has to stay busy (setting it to Free
//! would undo it) and it has to stay on its calendar (the real meetings are
//! there too), so neither existing switch helps; and while it ran, the bar
//! named it instead of the next real meeting, and every one of them fired a
//! reminder.
//!
//! **Matched on the title, trimmed and folded the way search folds** —
//! `text::fold`, case and accents — so "Unavailable", " unavailable " and
//! "UNAVAILABLE" are one title, and a list typed on one keyboard matches
//! titles written on another. A second fold here would drift from search's.

use crate::text::fold;

/// How a listed title is compared with an event's title. Lowercase on the
/// wire and in the settings row: `"exact"`, `"contains"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuietMatch {
    /// The whole title, after trimming and folding.
    #[default]
    Exact,
    /// Anywhere in the title: "focus time" covers "Focus time (AM)" and
    /// "Focus time (PM)".
    Contains,
}

impl QuietMatch {
    /// The stored spelling, which is also the wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            QuietMatch::Exact => "exact",
            QuietMatch::Contains => "contains",
        }
    }
}

/// The list, folded once, ready to be asked about one title at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuietTitles {
    folded: Vec<String>,
    matching: QuietMatch,
}

impl QuietTitles {
    /// Folds every entry once. A blank entry is dropped rather than kept: as
    /// an exact title it would match only untitled events, and under
    /// Contains it would match every event there is.
    pub fn new<S: AsRef<str>>(titles: &[S], matching: QuietMatch) -> Self {
        let folded = titles
            .iter()
            .map(|t| fold(t.as_ref().trim()))
            .filter(|t| !t.is_empty())
            .collect();
        QuietTitles { folded, matching }
    }

    /// Whether the list holds nothing, so a caller can skip the walk.
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }

    /// Whether an event with this title is quiet. An untitled event never is:
    /// nobody can have listed it.
    pub fn matches(&self, title: Option<&str>) -> bool {
        let Some(title) = title else { return false };
        if self.folded.is_empty() {
            return false;
        }
        let title = fold(title.trim());
        match self.matching {
            QuietMatch::Exact => self.folded.contains(&title),
            QuietMatch::Contains => self.folded.iter().any(|t| title.contains(t.as_str())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{QuietMatch, QuietTitles};

    fn exact(titles: &[&str]) -> QuietTitles {
        QuietTitles::new(titles, QuietMatch::Exact)
    }
    fn contains(titles: &[&str]) -> QuietTitles {
        QuietTitles::new(titles, QuietMatch::Contains)
    }

    #[test]
    fn exact_matches_the_whole_title_whatever_its_case_and_surrounding_spaces() {
        let quiet = exact(&["Unavailable"]);
        assert!(quiet.matches(Some("Unavailable")));
        assert!(quiet.matches(Some("UNAVAILABLE")));
        assert!(quiet.matches(Some("  unavailable ")));
        assert!(!quiet.matches(Some("Unavailable: dentist")), "exact is the whole title");
        assert!(!quiet.matches(Some("Standup")));
    }

    #[test]
    fn the_listed_title_is_trimmed_and_folded_too() {
        let quiet = exact(&["  UNAVAILABLE\t"]);
        assert!(quiet.matches(Some("Unavailable")));
    }

    #[test]
    fn accents_fold_as_they_do_in_search() {
        assert!(exact(&["Réunion bloquée"]).matches(Some("reunion bloquee")));
    }

    #[test]
    fn contains_matches_anywhere_in_the_title() {
        let quiet = contains(&["focus time"]);
        assert!(quiet.matches(Some("Focus time (AM)")));
        assert!(quiet.matches(Some("Deep FOCUS TIME")));
        assert!(!quiet.matches(Some("Focus")), "the listed text must all be there");
    }

    #[test]
    fn any_listed_title_is_enough() {
        let quiet = exact(&["Unavailable", "Lunch"]);
        assert!(quiet.matches(Some("lunch")));
        assert!(quiet.matches(Some("Unavailable")));
        assert!(!quiet.matches(Some("Standup")));
    }

    #[test]
    fn a_blank_entry_matches_nothing_rather_than_everything() {
        let quiet = contains(&["", "   "]);
        assert!(quiet.is_empty());
        assert!(!quiet.matches(Some("Standup")));
        assert!(!quiet.matches(Some("")));
        assert!(!exact(&[" "]).matches(Some("")), "nor the untitled, under Exact");
    }

    #[test]
    fn an_untitled_event_is_never_quiet() {
        assert!(!exact(&["Unavailable"]).matches(None));
        assert!(!contains(&["Unavailable"]).matches(None));
    }

    #[test]
    fn an_empty_list_matches_nothing() {
        assert!(QuietTitles::default().is_empty());
        assert!(!QuietTitles::default().matches(Some("Unavailable")));
    }
}
