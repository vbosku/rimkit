//! What differs between the saved and the current active list.

use std::collections::HashSet;

#[derive(Default, Debug, PartialEq)]
pub struct Changes {
    pub activated: Vec<String>,
    pub deactivated: Vec<String>,
    /// Mods present in both lists are in a different relative order.
    pub reordered: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.activated.is_empty() && self.deactivated.is_empty() && !self.reordered
    }

    /// "2 activated, 1 deactivated, order changed"
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.activated.is_empty() {
            parts.push(format!("{} activated", self.activated.len()));
        }
        if !self.deactivated.is_empty() {
            parts.push(format!("{} deactivated", self.deactivated.len()));
        }
        if self.reordered {
            parts.push("order changed".into());
        }
        parts.join(", ")
    }

    pub fn contains(&self, id: &str) -> bool {
        self.activated.iter().chain(&self.deactivated).any(|x| x == id)
    }
}

pub fn diff(saved: &[String], active: &[String]) -> Changes {
    let in_saved: HashSet<&str> = saved.iter().map(String::as_str).collect();
    let in_active: HashSet<&str> = active.iter().map(String::as_str).collect();
    let activated = active.iter().filter(|x| !in_saved.contains(x.as_str())).cloned().collect();
    let deactivated = saved.iter().filter(|x| !in_active.contains(x.as_str())).cloned().collect();
    let common_saved = saved.iter().filter(|x| in_active.contains(x.as_str()));
    let common_active = active.iter().filter(|x| in_saved.contains(x.as_str()));
    let reordered = !common_saved.eq(common_active);
    Changes { activated, deactivated, reordered }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_changes() {
        let c = diff(&v(&["a", "b"]), &v(&["a", "b"]));
        assert!(c.is_empty());
        assert_eq!(c.summary(), "");
    }

    #[test]
    fn activated_and_deactivated() {
        let c = diff(&v(&["a", "b"]), &v(&["a", "c", "d"]));
        assert_eq!(c.activated, v(&["c", "d"]));
        assert_eq!(c.deactivated, v(&["b"]));
        assert!(!c.reordered);
        assert_eq!(c.summary(), "2 activated, 1 deactivated");
        assert!(c.contains("b") && c.contains("c") && !c.contains("a"));
    }

    #[test]
    fn reorder_is_detected_but_insertions_are_not_reorders() {
        assert!(diff(&v(&["a", "b"]), &v(&["b", "a"])).reordered);
        assert!(!diff(&v(&["a", "b"]), &v(&["a", "x", "b"])).reordered);
        assert_eq!(diff(&v(&["a", "b"]), &v(&["b", "a"])).summary(), "order changed");
    }
}
