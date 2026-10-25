//! Acceptance B04: the mechanical counterpart to the repository
//! boundaries that the multi-repository strategy relied on.
//!
//! the multi-repository strategy claimed that one repository per component made the layering rules'
//! dependency-direction rules enforceable at the repository boundary. Nothing
//! ever checked them: each repository only saw its own dependency subgraph.
//! Inside a single workspace the whole graph is available at once, so this test
//! reads it from `cargo metadata` and asserts the rules against the layer table
//! in `[workspace.metadata.inox]` — the single source of truth for who is in
//! which layer.
//!
//! Run it on its own with:
//!
//! ```text
//! cargo test -p inox-integration --test layering
//! ```
//!
//! The rules live in [`rules::check`], which takes a plain graph value; the
//! unit tests at the bottom of this file exercise it against synthetic graphs,
//! so a rule that stops working fails here rather than silently passing.

use std::{collections::BTreeMap, path::Path, process::Command};

use serde_json::Value;

/// The layer ranks the layering rules orders dependencies along: `L4 → L3 → L2 → L1`.
const RANKS: [(&str, u8); 4] = [("l1", 1), ("l2", 2), ("l3", 3), ("l4", 4)];

/// The layer whose members are exempt from the direction rules (the layering rules
/// rule 7: `inox-integration` aggregates the system, so it may depend on
/// anything). Nothing may depend on it in turn.
const TEST_LAYER: &str = "test";

/// How a dependency is declared in the manifest, for the failure message.
type Kind = &'static str;

mod rules {
    use super::*;

    /// One directed edge of the Inox-internal dependency graph.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Edge {
        /// The dependent.
        pub from: String,
        /// The dependency.
        pub to: String,
        /// `normal`, `dev` or `build`.
        pub kind: Kind,
    }

    impl Edge {
        pub fn describe(&self) -> String {
            match self.kind {
                "normal" => format!("{} -> {}", self.from, self.to),
                kind => format!("{} -> {} ({kind} dependency)", self.from, self.to),
            }
        }
    }

    /// The whole picture the rules are checked against, so the rules can be
    /// unit-tested without a real workspace.
    #[derive(Debug, Default)]
    pub struct Graph {
        /// Workspace member name -> layer key (`l1`..`l4`, `test`).
        pub layers: BTreeMap<String, String>,
        /// Inox-internal edges, sorted and deduplicated.
        pub edges: Vec<Edge>,
    }

    impl Graph {
        fn rank(&self, crate_name: &str) -> Option<u8> {
            let layer = self.layers.get(crate_name)?;
            RANKS
                .iter()
                .find(|(key, _)| key == layer)
                .map(|(_, rank)| *rank)
        }

        fn layer_of(&self, crate_name: &str) -> &str {
            self.layers
                .get(crate_name)
                .map(String::as_str)
                .unwrap_or("unknown layer")
        }

        /// Human-readable layer label, for diagnostics.
        fn label(&self, crate_name: &str) -> String {
            let layer = self.layer_of(crate_name).to_uppercase();
            format!("`{crate_name}` ({layer})")
        }

        fn is_exempt(&self, crate_name: &str) -> bool {
            self.layer_of(crate_name) == TEST_LAYER
        }
    }

    /// The exception lists from `[workspace.metadata.inox.rules]`.
    #[derive(Debug, Default)]
    pub struct Exceptions {
        /// Rule 3: pairs that must have no Rust dependency in either direction.
        pub decoupled: Vec<(String, String)>,
        /// Rule 6: L3 crates allowed to depend on `inox-compositor`.
        pub l3_dependencies_on_compositor: Vec<String>,
        /// Rule 1: same-layer edges that an ADR has explicitly permitted.
        pub same_layer_exceptions: Vec<(String, String)>,
    }

    impl Exceptions {
        fn has_pair(pairs: &[(String, String)], a: &str, b: &str) -> bool {
            pairs
                .iter()
                .any(|(x, y)| (x == a && y == b) || (x == b && y == a))
        }
    }

    /// Evaluate every rule and return one message per violation, so a single
    /// run reports the whole set instead of stopping at the first.
    pub fn check(graph: &Graph, exceptions: &Exceptions, compositor: &str) -> Vec<String> {
        let mut violations = Vec::new();

        for edge in &graph.edges {
            let Edge { from, to, .. } = edge;

            // Rule 2: `inox-common` is the leaf of the Inox graph.
            if from == "inox-common" {
                violations.push(format!(
                    "rule 2 (leaf): {} must depend on no other Inox crate, but it depends on \
                     `{}`. {}",
                    graph.label(from),
                    to,
                    edge.describe()
                ));
                continue;
            }

            // Nothing may depend on the aggregation crate.
            if graph.is_exempt(to) {
                violations.push(format!(
                    "rule 7 (aggregation): {} depends on {}, which only aggregates the system; \
                     nothing may depend on it in turn. {}",
                    graph.label(from),
                    graph.label(to),
                    edge.describe()
                ));
                continue;
            }

            let (Some(from_rank), Some(to_rank)) = (graph.rank(from), graph.rank(to)) else {
                // An unlayered crate is reported by the coverage check.
                continue;
            };

            // Rule 3: the compositor/shell boundary carries no Rust dependency,
            // in either direction.
            if Exceptions::has_pair(&exceptions.decoupled, from, to) {
                violations.push(format!(
                    "rule 3 (decoupled): {} and {} must not have a Rust dependency in either \
                     direction — they talk over Wayland / `inox-ipc` / D-Bus. {}",
                    graph.label(from),
                    graph.label(to),
                    edge.describe()
                ));
                continue;
            }

            // Rule 6: L3 reaches down into L2, but only into the one crate ADR
            // 0002 names as an explicit exception.
            if to == compositor && graph.layer_of(from) == "l3" {
                if exceptions
                    .l3_dependencies_on_compositor
                    .iter()
                    .any(|c| c == from)
                {
                    continue;
                }
                violations.push(format!(
                    "rule 6 (single point): {} depends on {}, but layering rule 6 limits L3 \
                     access to the compositor to the crates listed in \
                     `[workspace.metadata.inox.rules] l3_dependencies_on_compositor`. Widen it \
                     with an ADR, or move the capability down to `inox-protocols`. {}",
                    graph.label(from),
                    graph.label(to),
                    edge.describe()
                ));
                continue;
            }

            // Rule 1: dependencies may only point downward, skipping layers is
            // fine. A same-layer edge needs an explicit, ADR-approved exception.
            if to_rank < from_rank {
                continue;
            }

            if to_rank == from_rank {
                if Exceptions::has_pair(&exceptions.same_layer_exceptions, from, to) {
                    continue;
                }
                violations.push(format!(
                    "rule 1 (same layer): {} depends on {}; layering rule 1 forbids same-layer \
                     dependencies. The decision index records this as an unresolved ambiguity in rule \
                     1 and the PRD repository table, so resolving it needs an ADR and an entry in \
                     `[workspace.metadata.inox.rules] same_layer_exceptions`, not a silent \
                     exception. {}",
                    graph.label(from),
                    graph.label(to),
                    edge.describe()
                ));
                continue;
            }

            violations.push(format!(
                "rule 1 (downward only): {} depends on {}; dependencies may only point downward \
                 (L4 -> L3 -> L2 -> L1). {}",
                graph.label(from),
                graph.label(to),
                edge.describe()
            ));
        }

        violations
    }

    /// The layer table must name every workspace member exactly once, and name
    /// nothing else. This is what stops a new crate from escaping the rules.
    pub fn check_coverage(
        members: &std::collections::BTreeSet<String>,
        layers: &BTreeMap<String, Vec<String>>,
    ) -> Vec<String> {
        let mut violations = Vec::new();
        let mut assigned: BTreeMap<&str, &str> = BTreeMap::new();

        for (layer, crates) in layers {
            for crate_name in crates {
                if let Some(previous) = assigned.insert(crate_name, layer) {
                    violations.push(format!(
                        "layer table: `{crate_name}` is listed in both `{previous}` and \
                         `{layer}`; a crate belongs to exactly one layer."
                    ));
                }
                if !members.contains(crate_name) {
                    violations.push(format!(
                        "layer table: `{crate_name}` is listed in `{layer}` but is not a \
                         workspace member."
                    ));
                }
                if !RANKS.iter().any(|(key, _)| key == layer) && layer != TEST_LAYER {
                    violations.push(format!(
                        "layer table: `{layer}` is not one of {} or `{TEST_LAYER}`.",
                        RANKS
                            .iter()
                            .map(|(key, _)| format!("`{key}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }

        for member in members {
            if !assigned.contains_key(member.as_str()) {
                violations.push(format!(
                    "layer table: `{member}` is a workspace member but has no layer. Add it to \
                     `[workspace.metadata.inox.layers]` in the root Cargo.toml so the \
                     dependency-direction rules apply to it."
                ));
            }
        }

        violations
    }
}

/// Read the workspace graph out of `cargo metadata`.
fn load_graph() -> (rules::Graph, rules::Exceptions, Vec<String>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root must resolve");

    let output = Command::new(env!("CARGO"))
        .current_dir(&root)
        // `--no-deps` keeps this to manifest data only: no registry access,
        // nothing to download, no build lock to contend with.
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .output()
        .expect("`cargo metadata` must run");

    assert!(
        output.status.success(),
        "`cargo metadata` failed in {}:\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("`cargo metadata` must emit valid JSON");

    let member_ids: Vec<&str> = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members must be an array")
        .iter()
        .map(|id| id.as_str().expect("workspace member id must be a string"))
        .collect();

    let mut members = std::collections::BTreeSet::new();
    let mut edges = Vec::new();

    for package in metadata["packages"]
        .as_array()
        .expect("packages must be an array")
    {
        let id = package["id"].as_str().expect("package id must be a string");
        if !member_ids.contains(&id) {
            continue;
        }
        let name = package["name"]
            .as_str()
            .expect("package name must be a string")
            .to_owned();
        members.insert(name.clone());

        for dependency in package["dependencies"]
            .as_array()
            .expect("dependencies must be an array")
        {
            let dep_name = dependency["name"]
                .as_str()
                .expect("dependency name must be a string");
            // Only Inox crates are ours to order; third-party crates are out of
            // scope for the layering rules' layering.
            if !dep_name.starts_with("inox-") {
                continue;
            }
            let kind = match dependency["kind"].as_str() {
                None => "normal",
                Some("dev") => "dev",
                Some("build") => "build",
                Some(other) => {
                    panic!("unexpected dependency kind `{other}` for `{dep_name}`")
                }
            };
            edges.push(rules::Edge {
                from: name.clone(),
                to: dep_name.to_owned(),
                kind,
            });
        }
    }

    edges.sort();
    edges.dedup();

    let inox_metadata = &metadata["metadata"]["inox"];

    let mut layers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut crate_layers: BTreeMap<String, String> = BTreeMap::new();
    for (layer, crates) in inox_metadata["layers"]
        .as_object()
        .expect("`[workspace.metadata.inox.layers]` must exist")
    {
        let names: Vec<String> = crates
            .as_array()
            .expect("each layer must be an array")
            .iter()
            .map(|name| {
                name.as_str()
                    .expect("a layer entry must be a string")
                    .to_owned()
            })
            .collect();
        for name in &names {
            crate_layers.insert(name.clone(), layer.clone());
        }
        layers.insert(layer.clone(), names);
    }

    let pair_list = |key: &str| -> Vec<(String, String)> {
        inox_metadata["rules"][key]
            .as_array()
            .expect("each rules entry must be an array")
            .iter()
            .map(|pair| {
                let pair = pair
                    .as_array()
                    .expect("a rule pair must be a two-element array");
                assert_eq!(pair.len(), 2, "a rule pair must have two elements");
                (
                    pair[0]
                        .as_str()
                        .expect("pair member must be a string")
                        .to_owned(),
                    pair[1]
                        .as_str()
                        .expect("pair member must be a string")
                        .to_owned(),
                )
            })
            .collect()
    };

    let string_list = |key: &str| -> Vec<String> {
        inox_metadata["rules"][key]
            .as_array()
            .expect("each rules entry must be an array")
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .expect("a rules entry must be a string")
                    .to_owned()
            })
            .collect()
    };

    let exceptions = rules::Exceptions {
        decoupled: pair_list("decoupled"),
        l3_dependencies_on_compositor: string_list("l3_dependencies_on_compositor"),
        same_layer_exceptions: pair_list("same_layer_exceptions"),
    };

    let violations = rules::check_coverage(&members, &layers);

    (
        rules::Graph {
            layers: crate_layers,
            edges,
        },
        exceptions,
        violations,
    )
}

#[test]
fn workspace_members_are_layered() {
    let (_, _, violations) = load_graph();
    assert!(
        violations.is_empty(),
        "the layer table in `[workspace.metadata.inox.layers]` does not describe the workspace:\n\n{}\n",
        violations.join("\n\n")
    );
}

#[test]
fn dependency_direction_follows_adr_0002() {
    let (graph, exceptions, coverage) = load_graph();
    assert!(
        coverage.is_empty(),
        "cannot check dependency direction while the layer table is incomplete:\n\n{}\n",
        coverage.join("\n\n")
    );

    let violations = rules::check(&graph, &exceptions, "inox-compositor");
    assert!(
        violations.is_empty(),
        "the layering rules dependency-direction violations:\n\n{}\n",
        violations.join("\n\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rules::{Edge, Exceptions, Graph};

    fn graph(edges: &[(&str, &str)]) -> Graph {
        let layers = [
            ("inox-common", "l1"),
            ("inox-config", "l1"),
            ("inox-protocols", "l2"),
            ("inox-compositor", "l2"),
            ("inox-xwayland", "l2"),
            ("inox-shell", "l3"),
            ("inox-panel", "l3"),
            ("inox-wallpaper", "l3"),
            ("inox-session", "l4"),
            ("inox-integration", "test"),
        ];
        Graph {
            layers: layers
                .iter()
                .map(|(name, layer)| ((*name).to_owned(), (*layer).to_owned()))
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                    kind: "normal",
                })
                .collect(),
        }
    }

    /// Mirrors `[workspace.metadata.inox.rules]` in the root manifest.
    fn default_exceptions() -> Exceptions {
        Exceptions {
            decoupled: vec![("inox-compositor".to_owned(), "inox-shell".to_owned())],
            l3_dependencies_on_compositor: vec!["inox-wallpaper".to_owned()],
            same_layer_exceptions: vec![("inox-config".to_owned(), "inox-common".to_owned())],
        }
    }

    fn check(edges: &[(&str, &str)]) -> Vec<String> {
        rules::check(&graph(edges), &default_exceptions(), "inox-compositor")
    }

    #[test]
    fn downward_edges_pass() {
        assert!(
            check(&[
                ("inox-config", "inox-common"),
                ("inox-compositor", "inox-common"),
                ("inox-session", "inox-config"),
                ("inox-integration", "inox-common"),
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_planned_same_layer_l2_edge_fails_until_an_adr_permits_it() {
        // PRD §3.1 lists `inox-compositor` as depending on `inox-protocols`,
        // which is an L2 -> L2 edge and therefore forbidden by rule 1 as
        // written. The dependency does not exist yet; when it lands, the check
        // goes red and the ambiguity has to be settled rather than absorbed.
        let violations = check(&[("inox-compositor", "inox-protocols")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 1 (same layer)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn skipped_layers_pass() {
        // Rule 1 allows reaching further down than one layer.
        assert!(check(&[("inox-session", "inox-common")]).is_empty());
    }

    #[test]
    fn upward_edge_fails() {
        let violations = check(&[("inox-config", "inox-protocols")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 1 (downward only)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn same_layer_edge_fails() {
        // The PRD lists this edge for the shell family, and rule 1 forbids it;
        // the decision index carries the ambiguity, and the check refuses to guess.
        let violations = check(&[("inox-panel", "inox-shell")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 1 (same layer)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn the_recorded_l1_ordering_passes() {
        // `inox-config -> inox-common` is in `same_layer_exceptions` because L1
        // has an internal order the layer model cannot express.
        assert!(check(&[("inox-config", "inox-common")]).is_empty());
    }

    #[test]
    fn common_is_a_leaf_even_against_a_recorded_exception() {
        let violations = check(&[("inox-common", "inox-config")]);
        // No exception can make this legal: the leaf rule is checked first.
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("rule 2 (leaf)"), "{}", violations[0]);
    }

    #[test]
    fn shell_must_not_reach_into_the_compositor() {
        let violations = check(&[("inox-shell", "inox-compositor")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 3 (decoupled)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn compositor_must_not_reach_into_the_shell() {
        let violations = check(&[("inox-compositor", "inox-shell")]);
        // Upward, but the named rule is the decoupling one.
        assert!(
            violations[0].contains("rule 3 (decoupled)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn the_documented_l3_to_compositor_exception_passes() {
        assert!(check(&[("inox-wallpaper", "inox-compositor")]).is_empty());
    }

    #[test]
    fn an_undocumented_l3_to_compositor_edge_fails() {
        let violations = check(&[("inox-panel", "inox-compositor")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 6 (single point)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn nothing_may_depend_on_the_aggregation_crate() {
        let violations = check(&[("inox-panel", "inox-integration")]);
        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].contains("rule 7 (aggregation)"),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn every_member_needs_a_layer() {
        let members: std::collections::BTreeSet<String> = ["inox-common", "inox-new"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let layers = BTreeMap::from([("l1".to_owned(), vec!["inox-common".to_owned()])]);
        let violations = rules::check_coverage(&members, &layers);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("inox-new"), "{}", violations[0]);
    }

    #[test]
    fn a_layer_may_not_name_a_phantom_crate() {
        let members: std::collections::BTreeSet<String> =
            std::iter::once("inox-common".to_owned()).collect();
        let layers = BTreeMap::from([(
            "l1".to_owned(),
            vec!["inox-common".to_owned(), "inox-ghost".to_owned()],
        )]);
        let violations = rules::check_coverage(&members, &layers);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("inox-ghost"), "{}", violations[0]);
    }

    #[test]
    fn a_crate_may_not_sit_in_two_layers() {
        let members: std::collections::BTreeSet<String> =
            std::iter::once("inox-common".to_owned()).collect();
        let layers = BTreeMap::from([
            ("l1".to_owned(), vec!["inox-common".to_owned()]),
            ("l2".to_owned(), vec!["inox-common".to_owned()]),
        ]);
        let violations = rules::check_coverage(&members, &layers);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("both"), "{}", violations[0]);
    }
}
