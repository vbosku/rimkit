//! Multi-selection in a list, like a file explorer: click, Ctrl+click to toggle, Shift+click for a range.
//! A mod is in exactly one list, so one selection serves both lists; `in_list` splits it.

use std::collections::HashSet;

#[derive(Default)]
pub struct Selection {
    picked: HashSet<String>,
    /// Where a Shift+click range starts: the last plain or Ctrl click.
    anchor: Option<String>,
}

impl Selection {
    /// `visible` = the rows of the clicked list, in display order (after filtering).
    pub fn click(&mut self, id: &str, visible: &[&str], shift: bool, toggle: bool) {
        let pos = |x: &str| visible.iter().position(|v| *v == x);
        let range = self.anchor.as_deref().and_then(pos).zip(pos(id));
        match range {
            Some((a, b)) if shift => self.picked = visible[a.min(b)..=a.max(b)].iter().map(|s| s.to_string()).collect(),
            _ if toggle => {
                if !self.picked.remove(id) {
                    self.picked.insert(id.to_string());
                }
                self.anchor = Some(id.to_string());
            }
            _ => {
                self.picked = HashSet::from([id.to_string()]);
                self.anchor = Some(id.to_string());
            }
        }
    }

    /// Selects every visible row of one list (and nothing else).
    pub fn select_all(&mut self, visible: &[&str]) {
        self.picked = visible.iter().map(|s| s.to_string()).collect();
        self.anchor = visible.first().map(|s| s.to_string());
    }

    pub fn clear(&mut self) {
        self.picked.clear();
        self.anchor = None;
    }

    pub fn contains(&self, id: &str) -> bool {
        self.picked.contains(id)
    }

    /// The selected ids among `list`, in `list` order.
    pub fn in_list(&self, list: &[&str]) -> Vec<String> {
        list.iter().filter(|id| self.picked.contains(**id)).map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [&str; 5] = ["a", "b", "c", "d", "e"];

    #[test]
    fn plain_click_selects_only_that_row() {
        let mut s = Selection::default();
        s.click("b", &ROWS, false, false);
        s.click("d", &ROWS, false, false);
        assert_eq!(s.in_list(&ROWS), ["d"]);
    }

    #[test]
    fn ctrl_click_toggles() {
        let mut s = Selection::default();
        s.click("b", &ROWS, false, false);
        s.click("d", &ROWS, false, true);
        assert_eq!(s.in_list(&ROWS), ["b", "d"]);
        s.click("b", &ROWS, false, true);
        assert_eq!(s.in_list(&ROWS), ["d"]);
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor_either_way() {
        let mut s = Selection::default();
        s.click("b", &ROWS, false, false);
        s.click("d", &ROWS, true, false);
        assert_eq!(s.in_list(&ROWS), ["b", "c", "d"]);
        s.click("a", &ROWS, true, false);
        assert_eq!(s.in_list(&ROWS), ["a", "b"], "range re-drawn from the same anchor");
    }

    #[test]
    fn shift_without_anchor_in_this_list_acts_like_a_click() {
        let mut s = Selection::default();
        s.click("x", &["x", "y"], false, false);
        s.click("c", &ROWS, true, false);
        assert_eq!(s.in_list(&ROWS), ["c"]);
        assert!(!s.contains("x"));
    }

    #[test]
    fn select_all_and_clear() {
        let mut s = Selection::default();
        s.click("x", &["x"], false, false);
        s.select_all(&ROWS);
        assert_eq!(s.in_list(&ROWS), ROWS);
        assert!(!s.contains("x"), "select all replaces the selection");
        s.clear();
        assert!(s.in_list(&ROWS).is_empty());
    }
}
