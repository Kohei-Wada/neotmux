use neotmux_core::proto::{PaneId, SplitDir};

// Applying a pane op needs the tree M2 introduces (§6). Until then applying one is a no-op
// that says so, so a plugin author can tell from the log that the call arrived.
pub fn split_note(dir: SplitDir) -> String {
    format!("pane.split('{dir}') arrived; there is no pane tree yet, so nothing changed")
}

pub fn focus_note(pane: PaneId) -> String {
    format!("pane.focus({pane}) arrived; there is no pane tree yet, so nothing changed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_note_names_the_direction_and_says_nothing_happened() {
        let note = split_note(SplitDir::Vertical);
        assert!(note.contains("vertical"), "{note}");
        assert!(note.contains("nothing changed"), "{note}");
    }

    #[test]
    fn a_focus_note_names_the_pane_and_says_nothing_happened() {
        let note = focus_note(PaneId::from(1));
        assert!(note.contains("%1"), "{note}");
        assert!(note.contains("nothing changed"), "{note}");
    }
}
