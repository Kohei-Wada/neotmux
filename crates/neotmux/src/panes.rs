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

    // The wording is a placeholder M2 deletes; what these pin is that a note names the op's
    // own subject, which is the part a plugin author reads to tell the calls apart.
    #[test]
    fn a_split_note_names_the_direction() {
        let note = split_note(SplitDir::Vertical);
        assert!(note.contains("vertical"), "{note}");
    }

    #[test]
    fn a_focus_note_names_the_pane() {
        let note = focus_note(PaneId::from(1));
        assert!(note.contains("%1"), "{note}");
    }
}
