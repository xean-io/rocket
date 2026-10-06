//! Target expansion and dependency ordering (Go: `graph.go`).

use crate::error::Error;
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet};

/// The group member that expands to every service.
pub const ALL_SERVICES: &str = "*";

/// Sorted, unique union of every layer's profiles.
pub fn merge_profiles(layers: &[Vec<String>]) -> Vec<String> {
    layers
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

impl Project {
    /// All service names, sorted.
    pub fn service_names(&self) -> Vec<String> {
        self.services.keys().cloned().collect()
    }

    fn has_service(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }

    /// Resolves service and group names to a sorted, unique set of service
    /// names. An empty input selects every service.
    pub fn expand_targets(&self, targets: &[String]) -> Result<Vec<String>, Error> {
        self.expand_with_wildcard(targets, self.service_names())
    }

    /// Like [`expand_targets`](Self::expand_targets) but wildcard members are
    /// filtered by the active profiles. Explicit service/group members and
    /// dependencies remain selectable.
    pub fn expand_startup_targets(
        &self,
        targets: &[String],
        profiles: &[String],
    ) -> Result<Vec<String>, Error> {
        let active: BTreeSet<&str> = profiles.iter().map(String::as_str).collect();
        let eligible = self
            .services
            .iter()
            .filter(|(_, svc)| {
                svc.profiles.is_empty() || svc.profiles.iter().any(|p| active.contains(p.as_str()))
            })
            .map(|(name, _)| name.clone())
            .collect();
        self.expand_with_wildcard(targets, eligible)
    }

    fn expand_with_wildcard(
        &self,
        targets: &[String],
        wildcard: Vec<String>,
    ) -> Result<Vec<String>, Error> {
        if targets.is_empty() {
            return Ok(wildcard);
        }
        let mut set = BTreeSet::new();
        for t in targets {
            if t == ALL_SERVICES {
                set.extend(wildcard.iter().cloned());
            } else if self.has_service(t) {
                set.insert(t.clone());
            } else if let Some(members) = self.groups.get(t) {
                for m in members {
                    if m == ALL_SERVICES {
                        set.extend(wildcard.iter().cloned());
                    } else {
                        set.insert(m.clone());
                    }
                }
            } else {
                return Err(Error::UnknownTarget {
                    target: t.clone(),
                    project: self.name.clone(),
                });
            }
        }
        Ok(set.into_iter().collect())
    }

    fn dependencies(&self, name: &str) -> &[String] {
        self.services
            .get(name)
            .map_or(&[], |s| s.depends_on.as_slice())
    }

    /// The targets plus their transitive dependencies, every dependency before
    /// its dependents. Ties are broken alphabetically so the order is
    /// deterministic.
    pub fn start_order(&self, targets: &[String]) -> Result<Vec<String>, Error> {
        let mut closure: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = targets.iter().map(String::as_str).collect();
        while let Some(n) = stack.pop() {
            if closure.insert(n) {
                stack.extend(self.dependencies(n).iter().map(String::as_str));
            }
        }

        let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for &n in &closure {
            indegree.entry(n).or_insert(0);
            for d in self.dependencies(n) {
                if closure.contains(d.as_str()) {
                    *indegree.entry(n).or_insert(0) += 1;
                    dependents.entry(d.as_str()).or_default().push(n);
                }
            }
        }
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(n, _)| *n)
            .collect();
        ready.sort_unstable();
        let mut order: Vec<String> = Vec::with_capacity(closure.len());
        while !ready.is_empty() {
            let n = ready.remove(0);
            order.push(n.to_string());
            for &m in dependents.get(n).map_or(&[][..], Vec::as_slice) {
                let deg = indegree.get_mut(m).expect("dependent is in closure");
                *deg -= 1;
                if *deg == 0 {
                    ready.push(m);
                    ready.sort_unstable();
                }
            }
        }
        if order.len() != closure.len() {
            let services = indegree
                .into_iter()
                .filter(|(_, deg)| *deg > 0)
                .map(|(n, _)| n.to_string())
                .collect();
            return Err(Error::DependencyCycle { services });
        }
        Ok(order)
    }

    /// Sorts `services` so dependents stop before their dependencies. Unknown
    /// services are kept at the end (sorted). On a dependency cycle the plain
    /// sorted list is returned.
    pub fn stop_order(&self, services: &[String]) -> Vec<String> {
        let (known, mut unknown): (Vec<String>, Vec<String>) =
            services.iter().cloned().partition(|s| self.has_service(s));
        let Ok(full) = self.start_order(&known) else {
            let mut out = services.to_vec();
            out.sort();
            return out;
        };
        let want: BTreeSet<&str> = known.iter().map(String::as_str).collect();
        let mut out: Vec<String> = full
            .into_iter()
            .rev()
            .filter(|s| want.contains(s.as_str()))
            .collect();
        unknown.sort();
        out.extend(unknown);
        out
    }
}
