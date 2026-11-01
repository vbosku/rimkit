//! Load-order sort: stable topological sort over loadAfter/loadBefore/dependencies.
//! Ties keep the current order, so sorting an already-valid list changes nothing.

use super::ModInfo;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Official content in the order the game loads it.
pub const OFFICIAL: [&str; 6] = [
    "ludeon.rimworld",
    "ludeon.rimworld.royalty",
    "ludeon.rimworld.ideology",
    "ludeon.rimworld.biotech",
    "ludeon.rimworld.anomaly",
    "ludeon.rimworld.odyssey",
];

pub fn sort<'a>(active: &[String], info: impl Fn(&str) -> Option<&'a ModInfo>) -> Vec<String> {
    let n = active.len();
    let index: HashMap<&str, usize> = active.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut edges: HashSet<(usize, usize)> = HashSet::new();

    for (i, id) in active.iter().enumerate() {
        let Some(m) = info(id) else { continue };
        for dep in m.load_after.iter().chain(&m.dependencies) {
            if let Some(&j) = index.get(dep.as_str()) {
                link(&mut edges, j, i);
            }
        }
        for later in &m.load_before {
            if let Some(&j) = index.get(later.as_str()) {
                link(&mut edges, i, j);
            }
        }
    }

    // Official content goes in its fixed order...
    let official: Vec<usize> = OFFICIAL.iter().filter_map(|id| index.get(id).copied()).collect();
    for w in official.windows(2) {
        link(&mut edges, w[0], w[1]);
    }
    // ...and before every other mod, unless that mod asked to load before it (e.g. Harmony).
    let before_official = reaches(n, &edges, &official);
    for i in (0..n).filter(|i| !official.contains(i) && !before_official.contains(i)) {
        for &o in &official {
            link(&mut edges, o, i);
        }
    }

    // Kahn's algorithm, picking the lowest current index among ready nodes.
    let mut succ = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in &edges {
        succ[a].push(b);
        indeg[b] += 1;
    }
    let mut ready: BinaryHeap<Reverse<usize>> = (0..n).filter(|&i| indeg[i] == 0).map(Reverse).collect();
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(i)) = ready.pop() {
        placed[i] = true;
        order.push(i);
        for &j in &succ[i] {
            indeg[j] -= 1;
            if indeg[j] == 0 {
                ready.push(Reverse(j));
            }
        }
    }
    // Cycles: leave whatever is stuck in its current relative order at the end.
    order.extend((0..n).filter(|&i| !placed[i]));
    order.into_iter().map(|i| active[i].clone()).collect()
}

/// Nodes that can reach any of `targets` by following edges.
fn reaches(n: usize, edges: &HashSet<(usize, usize)>, targets: &[usize]) -> HashSet<usize> {
    let mut pred = vec![Vec::new(); n];
    for &(a, b) in edges {
        pred[b].push(a);
    }
    let mut seen = HashSet::new();
    let mut stack = targets.to_vec();
    while let Some(x) = stack.pop() {
        for &p in &pred[x] {
            if !targets.contains(&p) && seen.insert(p) {
                stack.push(p);
            }
        }
    }
    seen
}

/// Records "a loads before b".
fn link(edges: &mut HashSet<(usize, usize)>, a: usize, b: usize) {
    if a != b {
        edges.insert((a, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn m(id: &str, after: &[&str], before: &[&str]) -> ModInfo {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        ModInfo { id: id.into(), name: id.into(), load_after: v(after), load_before: v(before), ..Default::default() }
    }

    #[test]
    fn harmony_first_core_before_mods_deps_respected() {
        let mods = [
            m("a", &["b"], &[]),
            m("b", &[], &[]),
            m("ludeon.rimworld.biotech", &[], &[]),
            m("ludeon.rimworld", &[], &[]),
            m("brrainz.harmony", &[], &["ludeon.rimworld"]),
        ];
        let active: Vec<String> = mods.iter().map(|m| m.id.clone()).collect();
        let sorted = sort(&active, |id| mods.iter().find(|m| m.id == id));
        assert_eq!(sorted, ["brrainz.harmony", "ludeon.rimworld", "ludeon.rimworld.biotech", "b", "a"]);
        // Sorting a sorted list is a no-op.
        assert_eq!(sort(&sorted, |id| mods.iter().find(|m| m.id == id)), sorted);
    }

    /// Reads the real install; run with `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_install() {
        let s = crate::model::Settings::default();
        let t = std::time::Instant::now();
        let ver = crate::model::mod_info::game_version(&s.game_dir);
        let roots = [(s.game_dir.join("Data"), Source::Official), (s.game_dir.join("Mods"), Source::Local), (s.workshop_dir.clone(), Source::Workshop)];
        let mods = crate::model::mod_info::scan(&roots, &ver);
        println!("game {ver}: scanned {} mods in {:?}", mods.len(), t.elapsed());
        let active = crate::model::mods_config::read_active(&s.mods_config()).unwrap();
        let sorted = sort(&active, |id| mods.iter().find(|m| m.id == id));
        let moved = active.iter().zip(&sorted).filter(|(a, b)| a != b).count();
        println!("active {}, positions changed by sort: {moved}", active.len());
        println!("first 8 after sort: {:?}", &sorted[..8.min(sorted.len())]);
        let missing: Vec<_> = active.iter().filter(|id| !mods.iter().any(|m| &m.id == *id)).collect();
        println!("missing: {missing:?}");
    }
}
