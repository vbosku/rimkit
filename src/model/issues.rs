//! Problems in the active list: missing mods, dependencies, incompatibilities, load order, game version.

use super::ModInfo;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Issue {
    NotInstalled,
    Duplicate,
    MissingDependency(String),
    Incompatible(String),
    /// Should load after this mod, but is above it.
    LoadAfter(String),
    /// Should load before this mod, but is below it.
    LoadBefore(String),
    /// Doesn't list the running game version; holds the newest version it lists.
    WrongVersion(String),
}

impl Issue {
    pub fn severity(&self) -> Severity {
        match self {
            Issue::NotInstalled | Issue::Duplicate | Issue::MissingDependency(_) | Issue::Incompatible(_) => Severity::Error,
            Issue::LoadAfter(_) | Issue::LoadBefore(_) => Severity::Warning,
            Issue::WrongVersion(_) => Severity::Info,
        }
    }

    /// `name` turns a package id into a display name.
    pub fn message(&self, name: impl Fn(&str) -> String) -> String {
        match self {
            Issue::NotInstalled => "Not installed".into(),
            Issue::Duplicate => "Listed more than once".into(),
            Issue::MissingDependency(id) => format!("Needs {}", name(id)),
            Issue::Incompatible(id) => format!("Incompatible with {}", name(id)),
            Issue::LoadAfter(id) => format!("Should load after {}", name(id)),
            Issue::LoadBefore(id) => format!("Should load before {}", name(id)),
            Issue::WrongVersion(v) => format!("Made for {v}, not this game version"),
        }
    }
}

/// Issues of every active mod that has at least one, keyed by package id.
pub fn check<'a>(active: &[String], info: impl Fn(&str) -> Option<&'a ModInfo>, game_version: &str) -> HashMap<String, Vec<Issue>> {
    let mut pos: HashMap<&str, usize> = HashMap::new();
    for (i, id) in active.iter().enumerate() {
        pos.entry(id.as_str()).or_insert(i);
    }
    let mut out: HashMap<String, Vec<Issue>> = HashMap::new();
    for (i, id) in active.iter().enumerate() {
        let mut found = Vec::new();
        if pos[id.as_str()] != i {
            found.push(Issue::Duplicate);
        } else if let Some(m) = info(id) {
            for dep in &m.dependencies {
                match pos.get(dep.as_str()) {
                    None => found.push(Issue::MissingDependency(dep.clone())),
                    Some(&p) if p > i => found.push(Issue::LoadAfter(dep.clone())),
                    _ => {}
                }
            }
            for x in m.load_after.iter().filter(|x| !m.dependencies.contains(x)) {
                if pos.get(x.as_str()).is_some_and(|&p| p > i) {
                    found.push(Issue::LoadAfter(x.clone()));
                }
            }
            for x in &m.load_before {
                if pos.get(x.as_str()).is_some_and(|&p| p < i) {
                    found.push(Issue::LoadBefore(x.clone()));
                }
            }
            for x in m.incompatible.iter().filter(|x| pos.contains_key(x.as_str())) {
                found.push(Issue::Incompatible(x.clone()));
            }
            if !m.supports(game_version) {
                found.push(Issue::WrongVersion(m.versions.last().cloned().unwrap_or_else(|| "?".into())));
            }
        } else {
            found.push(Issue::NotInstalled);
        }
        if !found.is_empty() {
            out.entry(id.clone()).or_default().extend(found);
        }
    }
    out
}

/// How many mods have at least one issue of exactly this severity.
pub fn count(issues: &HashMap<String, Vec<Issue>>, severity: Severity) -> usize {
    issues.values().filter(|list| list.iter().any(|i| i.severity() == severity)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str) -> ModInfo {
        ModInfo { id: id.into(), name: id.to_uppercase(), versions: vec!["1.7".into()], ..Default::default() }
    }

    fn run(mods: &[ModInfo], active: &[&str]) -> HashMap<String, Vec<Issue>> {
        let active: Vec<String> = active.iter().map(|s| s.to_string()).collect();
        check(&active, |id| mods.iter().find(|m| m.id == id), "1.7")
    }

    #[test]
    fn clean_list_has_no_issues() {
        let mods = [m("a"), m("b")];
        assert!(run(&mods, &["a", "b"]).is_empty());
    }

    #[test]
    fn not_installed_and_duplicate() {
        let mods = [m("a")];
        let r = run(&mods, &["a", "ghost", "a"]);
        assert_eq!(r["ghost"], [Issue::NotInstalled]);
        assert_eq!(r["a"], [Issue::Duplicate]);
    }

    #[test]
    fn missing_and_misordered_dependency() {
        let mut a = m("a");
        a.dependencies = vec!["b".into(), "c".into()];
        let mods = [a, m("b")];
        let r = run(&mods, &["a", "b"]);
        assert_eq!(r["a"], [Issue::LoadAfter("b".into()), Issue::MissingDependency("c".into())]);
    }

    #[test]
    fn load_after_and_before_rules() {
        let mut a = m("a");
        a.load_after = vec!["b".into(), "absent".into()];
        let mut c = m("c");
        c.load_before = vec!["a".into()];
        let mods = [a, m("b"), c];
        let r = run(&mods, &["a", "b", "c"]);
        assert_eq!(r["a"], [Issue::LoadAfter("b".into())]);
        assert_eq!(r["c"], [Issue::LoadBefore("a".into())]);
    }

    #[test]
    fn incompatible_only_when_both_active() {
        let mut a = m("a");
        a.incompatible = vec!["b".into(), "z".into()];
        let mods = [a, m("b")];
        assert_eq!(run(&mods, &["a", "b"])["a"], [Issue::Incompatible("b".into())]);
        assert!(run(&mods, &["a"]).is_empty());
    }

    #[test]
    fn wrong_version_is_info_and_official_is_exempt() {
        let mut old = m("old");
        old.versions = vec!["1.5".into(), "1.6".into()];
        let mut core = m("ludeon.rimworld");
        core.versions.clear();
        core.source = crate::model::Source::Official;
        let mods = [old, core];
        let r = run(&mods, &["ludeon.rimworld", "old"]);
        assert_eq!(r["old"], [Issue::WrongVersion("1.6".into())]);
        assert!(!r.contains_key("ludeon.rimworld"));
        assert_eq!(Issue::WrongVersion("1.6".into()).severity(), Severity::Info);
    }

    #[test]
    fn severities_and_counts() {
        assert_eq!(Issue::NotInstalled.severity(), Severity::Error);
        assert_eq!(Issue::MissingDependency("x".into()).severity(), Severity::Error);
        assert_eq!(Issue::LoadAfter("x".into()).severity(), Severity::Warning);
        let mut a = m("a");
        a.dependencies = vec!["gone".into()];
        a.load_after = vec!["b".into()];
        let mods = [a, m("b")];
        let r = run(&mods, &["a", "b", "ghost"]);
        assert_eq!(count(&r, Severity::Error), 2);
        assert_eq!(count(&r, Severity::Warning), 1);
        assert_eq!(count(&r, Severity::Info), 0);
    }

    #[test]
    fn messages_use_display_names() {
        let name = |id: &str| format!("<{id}>");
        assert_eq!(Issue::MissingDependency("b".into()).message(name), "Needs <b>");
        assert_eq!(Issue::WrongVersion("1.6".into()).message(name), "Made for 1.6, not this game version");
    }
}
