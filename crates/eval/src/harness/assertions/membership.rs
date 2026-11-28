//! Assertions over which files share, keep, or lose a container or home.

use std::collections::BTreeSet;

use unlace_engine::result::ContainerNode;

use crate::harness::Verdict;
use crate::harness::inputs::{EvalInputs, find_package_node, packages_holding};
use crate::metrics;
use crate::target::{FaceMode, PathSetAssertion, PreserveDir, PreserveSymbolHome};

/// `preserve_symbol_home`: the named symbol still resides in the named file of
/// the asserted candidate tree. File identity is the full repo-relative path
/// (stable across trees per the contract), and symbol membership is read off
/// the file's `symbols` list — exactly what FIX08's symbol-grain relocation
/// rewrites, so this is the assertion that catches gratuitous home churn.
pub(super) fn evaluate_preserve_symbol_home(
    assertion: &PreserveSymbolHome,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
) -> Verdict {
    let label = format!("preserve_symbol_home({})#{face:?}", assertion.symbol);
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let mut found_home = false;
    let mut now_in: Vec<String> = Vec::new();
    metrics::walk_files(tree, &mut |node| {
        let Some(symbols) = &node.symbols else {
            return;
        };
        let carries = symbols
            .iter()
            .any(|placement| placement.name == assertion.symbol);
        if carries && node.name == assertion.path {
            found_home = true;
        }
        if carries {
            now_in.push(node.name.clone());
        }
    });
    Verdict {
        label,
        passed: found_home,
        detail: if found_home {
            format!("{} still lives in {}", assertion.symbol, assertion.path)
        } else if now_in.is_empty() {
            format!(
                "{} does not appear in any file of the candidate tree",
                assertion.symbol
            )
        } else {
            format!(
                "{} left its pinned home {}: it now appears in {}",
                assertion.symbol,
                assertion.path,
                now_in.join(", ")
            )
        },
    }
}

/// `preserve_dir`: within each package that physically has the directory, some
/// folder/domain node whose container chain ends in `path` must still contain
/// all of D(P, path) (see [`surviving_container`]).
pub(super) fn evaluate_preserve_dir(
    assertion: &PreserveDir,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
) -> Verdict {
    let label = format!("preserve_dir({})#{face:?}", assertion.path);
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let holdings = packages_holding(&inputs.census, &inputs.packages, &assertion.path);
    if holdings.is_empty() {
        return Verdict {
            label,
            passed: false,
            detail: format!(
                "directory {:?} resolves under no analyzed package",
                assertion.path
            ),
        };
    }
    let mut missing_reports = Vec::new();
    for (package, expected) in &holdings {
        let satisfied = find_package_node(tree, &package.name).is_some_and(|package_node| {
            surviving_container(package_node, &assertion.path, expected)
        });
        if !satisfied {
            missing_reports.push(format!(
                "package {}: no folder/domain node named {:?} holds all of D = [{}]",
                package.name,
                assertion.path,
                expected.join(", ")
            ));
        }
    }
    Verdict {
        label,
        passed: missing_reports.is_empty(),
        detail: if missing_reports.is_empty() {
            format!(
                "directory {:?} survives as a named container in every holding package",
                assertion.path
            )
        } else {
            format!("directory tearing: {}", missing_reports.join("; "))
        },
    }
}

/// Whether some folder/domain descendant whose container chain ends in `path`
/// contains every file in `expected` anywhere in its subtree.
///
/// The engine renders a nested directory by its increment over the enclosing
/// domain: `tree/validation` is a `validation` folder under the `tree`
/// domain, and a domain's echo folder repeats its name (`tree` → `tree`). A
/// bare `name == path` test therefore never finds a nested directory. The
/// node is named by its chain of folder/domain names within the package
/// instead, with each echo folded, and `path` must equal that chain's last
/// segments, compared whole on `/`: `tree/validation` matches
/// `tree/validation` but not `parser/validation`, and never a `validation`
/// that only ends in the same characters. A one-segment `path` matches the
/// node's own name, as before.
///
/// Unlike the counting predicates this is deliberately SUBTREE-scoped:
/// `preserve_dir` pins tearing, and tearing means expected files LEAVING the
/// directory's reach — a directory that keeps its files across nested
/// sub-places inside itself has not been torn, whatever the first-level split
/// beneath it looks like.
fn surviving_container(package_node: &ContainerNode, path: &str, expected: &[String]) -> bool {
    let expected_set: BTreeSet<&String> = expected.iter().collect();
    let wanted: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let mut found = false;
    {
        let mut visit = |node: &ContainerNode, chain: &str| {
            if chain_ends_with(chain, &wanted) && metrics::is_scoped_container(node) {
                let member_list = metrics::members(node);
                let members: BTreeSet<&String> = member_list.iter().collect();
                if expected_set.is_subset(&members) {
                    found = true;
                }
            }
        };
        metrics::walk_containers(package_node, &mut visit);
    }
    found
}

/// Whether the `/`-joined container `chain` ends in the segments `wanted`,
/// once each repeated adjacent segment (a domain and its echo folder) is
/// folded into one.
fn chain_ends_with(chain: &str, wanted: &[&str]) -> bool {
    let mut segments: Vec<&str> = chain.split('/').filter(|part| !part.is_empty()).collect();
    segments.dedup();
    !wanted.is_empty() && segments.ends_with(wanted)
}

/// `keep_together` / `separate`: co-location required, or co-location banned,
/// over folder/domain containers only. Membership is first-level: a container
/// holds exactly its direct file children, so nesting one place under another
/// never reads as the two places merging.
pub(super) fn evaluate_path_set(
    assertion: &PathSetAssertion,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
    keep: bool,
) -> Verdict {
    let kind = if keep { "keep_together" } else { "separate" };
    let label = format!("{kind}({})#{face:?}", assertion.paths.join("+"));
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let wanted: BTreeSet<&String> = assertion.paths.iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    let mut satisfied = false;
    {
        // `walk_containers` visits folder/domain nodes only, so every visited
        // node is a scoped container.
        let mut visit = |node: &ContainerNode, _chain: &str| {
            let member_list = metrics::direct_members(node);
            let members: BTreeSet<&String> = member_list.iter().collect();
            let held: Vec<String> = wanted
                .intersection(&members)
                .map(|path| (*path).clone())
                .collect();
            if keep {
                if wanted.is_subset(&members) {
                    satisfied = true;
                }
            } else if held.len() >= 2 {
                offenders.push(format!(
                    "{:?} co-locates {}",
                    node.name,
                    held.join(" with ")
                ));
            }
        };
        metrics::walk_containers(tree, &mut visit);
    }

    if keep {
        Verdict {
            label,
            passed: satisfied,
            detail: if satisfied {
                "some folder/domain container co-locates the set".to_owned()
            } else {
                format!(
                    "no folder/domain container holds all of [{}]",
                    assertion.paths.join(", ")
                )
            },
        }
    } else {
        Verdict {
            label,
            passed: offenders.is_empty(),
            detail: if offenders.is_empty() {
                "no folder/domain container co-locates the set".to_owned()
            } else {
                format!("cross-directory welding: {}", offenders.join("; "))
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use unlace_engine::result::{ContainerNode, Level};

    use crate::harness::assertions::fixtures::{
        file, file_with_symbols, inputs_with, laminar_candidate, node, torn_candidate,
    };
    use crate::harness::inputs::{
        EvalInputs, FaceInputs, discover_packages, find_package_node, packages_holding,
    };
    use crate::metrics::{members, structural_placement};
    use crate::target::{FaceMode, PathSetAssertion, PreserveDir, PreserveSymbolHome};

    use super::{
        chain_ends_with, evaluate_path_set, evaluate_preserve_dir, evaluate_preserve_symbol_home,
        surviving_container,
    };

    fn symbol_home(symbol: &str, path: &str) -> PreserveSymbolHome {
        PreserveSymbolHome {
            symbol: symbol.to_owned(),
            path: path.to_owned(),
            mode: None,
            because: "the home holds in the best state".to_owned(),
            expect_fail: None,
        }
    }

    #[test]
    fn preserve_dir_requires_a_named_container_holding_every_member() {
        // The torn candidate has no `billing` container, so D(P, billing) =
        // [invoice, pricing] cannot be found under any folder/domain node.
        let torn = torn_candidate();
        let torn_inputs = inputs_with(&torn);
        let torn_packages = packages_holding(&torn_inputs.census, &torn_inputs.packages, "billing");
        assert_eq!(
            torn_packages.len(),
            1,
            "exactly the repository-rooted package holds billing/"
        );
        let anchored_tree = torn_inputs
            .faces
            .get(&FaceMode::Anchored)
            .map(|face| face.tree);
        let torn_satisfied = anchored_tree.is_some_and(|tree| {
            torn_packages.first().is_some_and(|(package, expected)| {
                find_package_node(tree, &package.name).is_some_and(|package_node| {
                    surviving_container(package_node, "billing", expected)
                })
            })
        });
        assert!(!torn_satisfied, "torn layout must dissolve billing");

        let laminar = laminar_candidate();
        let laminar_inputs = inputs_with(&laminar);
        let laminar_packages =
            packages_holding(&laminar_inputs.census, &laminar_inputs.packages, "billing");
        let laminar_tree = laminar_inputs
            .faces
            .get(&FaceMode::Anchored)
            .map(|face| face.tree);
        let laminar_satisfied = laminar_tree.is_some_and(|tree| {
            laminar_packages.first().is_some_and(|(package, expected)| {
                find_package_node(tree, &package.name).is_some_and(|package_node| {
                    surviving_container(package_node, "billing", expected)
                })
            })
        });
        assert!(laminar_satisfied, "laminar layout keeps billing");
    }

    #[test]
    fn membership_assertions_fail_loud_on_an_unevaluated_face() {
        let laminar = laminar_candidate();
        let inputs = inputs_with(&laminar);
        let face = FaceMode::Greenfield;
        let verdicts = [
            evaluate_preserve_symbol_home(
                &symbol_home("Invoice", "billing/invoice.py"),
                face,
                &inputs,
            ),
            evaluate_preserve_dir(
                &PreserveDir {
                    path: "billing".to_owned(),
                    mode: None,
                    because: "billing survives".to_owned(),
                    expect_fail: None,
                },
                face,
                &inputs,
            ),
            evaluate_path_set(
                &PathSetAssertion {
                    paths: vec!["billing/invoice.py".to_owned(), "pipeline.py".to_owned()],
                    mode: None,
                    because: "unrelated".to_owned(),
                    expect_fail: None,
                },
                face,
                &inputs,
                false,
            ),
        ];
        for verdict in verdicts {
            assert_eq!(verdict.label, "#Greenfield");
            assert!(!verdict.passed);
            assert_eq!(verdict.detail, "the Greenfield mode was not evaluated");
        }
    }

    #[test]
    fn preserve_symbol_home_reports_where_a_pinned_symbol_went() {
        let candidate = node(
            Level::PackageGroup,
            "root",
            vec![node(
                Level::Package,
                "app",
                vec![node(
                    Level::Folder,
                    "billing",
                    vec![
                        file_with_symbols("billing/invoice.py", &["Invoice"]),
                        file_with_symbols("billing/pricing.py", &["price", "Pricing"]),
                        file("billing/__init__.py"),
                    ],
                )],
            )],
        );
        let inputs = inputs_with(&candidate);

        let kept = evaluate_preserve_symbol_home(
            &symbol_home("Invoice", "billing/invoice.py"),
            FaceMode::Anchored,
            &inputs,
        );
        assert_eq!(kept.label, "preserve_symbol_home(Invoice)#Anchored");
        assert!(kept.passed);
        assert_eq!(kept.detail, "Invoice still lives in billing/invoice.py");

        let moved = evaluate_preserve_symbol_home(
            &symbol_home("price", "billing/invoice.py"),
            FaceMode::Anchored,
            &inputs,
        );
        assert!(!moved.passed);
        assert_eq!(
            moved.detail,
            "price left its pinned home billing/invoice.py: it now appears in billing/pricing.py"
        );

        let vanished = evaluate_preserve_symbol_home(
            &symbol_home("Refund", "billing/invoice.py"),
            FaceMode::Anchored,
            &inputs,
        );
        assert!(!vanished.passed);
        assert_eq!(
            vanished.detail,
            "Refund does not appear in any file of the candidate tree"
        );
    }

    #[test]
    fn a_same_named_container_missing_a_member_is_still_torn() {
        let partial = node(
            Level::PackageGroup,
            "root",
            vec![node(
                Level::Package,
                "app",
                vec![
                    node(Level::Folder, "billing", vec![file("billing/invoice.py")]),
                    node(
                        Level::Domain,
                        "helpers",
                        vec![file("billing/pricing.py"), file("pipeline.py")],
                    ),
                ],
            )],
        );
        let verdict = evaluate_preserve_dir(
            &PreserveDir {
                path: "billing".to_owned(),
                mode: None,
                because: "billing survives whole".to_owned(),
                expect_fail: None,
            },
            FaceMode::Anchored,
            &inputs_with(&partial),
        );
        assert!(
            !verdict.passed,
            "keeping the name while losing pricing.py is tearing: {}",
            verdict.detail
        );
    }

    #[test]
    fn preserve_dir_names_the_package_whose_directory_was_torn() {
        let torn = torn_candidate();
        let verdict = evaluate_preserve_dir(
            &PreserveDir {
                path: "billing".to_owned(),
                mode: None,
                because: "billing survives".to_owned(),
                expect_fail: None,
            },
            FaceMode::Anchored,
            &inputs_with(&torn),
        );
        assert_eq!(verdict.label, "preserve_dir(billing)#Anchored");
        assert!(!verdict.passed);
        assert_eq!(
            verdict.detail,
            "directory tearing: package app: no folder/domain node named \"billing\" holds all of D = [billing/invoice.py, billing/pricing.py]"
        );
    }

    #[test]
    fn preserve_dir_holds_for_surviving_directories_and_rejects_unknown_ones() {
        let laminar = laminar_candidate();
        let inputs = inputs_with(&laminar);
        let preserve = |path: &str| PreserveDir {
            path: path.to_owned(),
            mode: None,
            because: "the directory survives".to_owned(),
            expect_fail: None,
        };

        let kept = evaluate_preserve_dir(&preserve("billing"), FaceMode::Anchored, &inputs);
        assert!(kept.passed);
        assert_eq!(
            kept.detail,
            "directory \"billing\" survives as a named container in every holding package"
        );

        let unknown = evaluate_preserve_dir(&preserve("ledger"), FaceMode::Anchored, &inputs);
        assert!(!unknown.passed);
        assert_eq!(
            unknown.detail,
            "directory \"ledger\" resolves under no analyzed package"
        );
    }

    /// The engine's render of nested directories (the companion-folders
    /// shape): `tree/validation` is a `validation` folder beside the `tree`
    /// echo folder under the `tree` domain, and `inline` exists twice.
    fn nested_tree(validation_home: &str) -> ContainerNode {
        let validation = vec![
            file("src/tree/validation/shape.ts"),
            file("src/tree/validation/record.ts"),
        ];
        let (tree_children, parser_children) = if validation_home == "tree" {
            (
                vec![
                    node(Level::Folder, "tree", vec![file("src/tree/validation.ts")]),
                    node(Level::Folder, "validation", validation),
                ],
                vec![node(
                    Level::Folder,
                    "inline",
                    vec![file("src/parser/inline/annotation.ts")],
                )],
            )
        } else {
            (
                vec![node(
                    Level::Folder,
                    "tree",
                    vec![file("src/tree/validation.ts")],
                )],
                vec![
                    node(
                        Level::Folder,
                        "inline",
                        vec![file("src/parser/inline/annotation.ts")],
                    ),
                    node(Level::Folder, "validation", validation),
                ],
            )
        };
        node(
            Level::PackageGroup,
            "root",
            vec![node(
                Level::Package,
                "app",
                vec![
                    node(Level::Domain, "tree", tree_children),
                    node(Level::Domain, "parser", parser_children),
                    node(
                        Level::Domain,
                        "stringifier",
                        vec![node(
                            Level::Folder,
                            "inline",
                            vec![file("src/stringifier/inline/reference.ts")],
                        )],
                    ),
                ],
            )],
        )
    }

    /// Inputs whose census and current layout are `current`, judging
    /// `candidate` as the anchored face.
    fn inputs_over<'a>(current: &ContainerNode, candidate: &'a ContainerNode) -> EvalInputs<'a> {
        let census: BTreeSet<String> = members(current).into_iter().collect();
        EvalInputs {
            current_placement: structural_placement(current),
            packages: discover_packages(current, &census),
            census,
            faces: [(
                FaceMode::Anchored,
                FaceInputs {
                    tree: candidate,
                    capacity_remaining: None,
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    fn preserve(path: &str) -> PreserveDir {
        PreserveDir {
            path: path.to_owned(),
            mode: None,
            because: "the directory survives".to_owned(),
            expect_fail: None,
        }
    }

    #[test]
    fn preserve_dir_finds_a_nested_directory_rendered_under_its_parent_domain() {
        let current = nested_tree("tree");
        let inputs = inputs_over(&current, &current);

        let nested =
            evaluate_preserve_dir(&preserve("tree/validation"), FaceMode::Anchored, &inputs);
        let parent = evaluate_preserve_dir(&preserve("tree"), FaceMode::Anchored, &inputs);
        let sibling =
            evaluate_preserve_dir(&preserve("parser/inline"), FaceMode::Anchored, &inputs);

        assert!(nested.passed, "{}", nested.detail);
        assert!(parent.passed, "{}", parent.detail);
        assert!(sibling.passed, "{}", sibling.detail);
    }

    #[test]
    fn preserve_dir_rejects_a_same_named_directory_elsewhere() {
        let current = nested_tree("tree");
        let moved = nested_tree("parser");
        let inputs = inputs_over(&current, &moved);

        let verdict =
            evaluate_preserve_dir(&preserve("tree/validation"), FaceMode::Anchored, &inputs);

        assert!(
            !verdict.passed,
            "a validation folder under parser holding every member is not tree/validation: {}",
            verdict.detail
        );
        assert_eq!(
            verdict.detail,
            "directory tearing: package app: no folder/domain node named \"tree/validation\" holds all of D = [src/tree/validation/record.ts, src/tree/validation/shape.ts]"
        );
    }

    #[test]
    fn container_chains_match_whole_trailing_segments() {
        assert!(chain_ends_with("tree/validation", &["tree", "validation"]));
        assert!(chain_ends_with("tree/tree", &["tree"]));
        assert!(chain_ends_with(
            "helpers/tree/validation",
            &["tree", "validation"]
        ));
        assert!(!chain_ends_with(
            "parser/validation",
            &["tree", "validation"]
        ));
        assert!(!chain_ends_with(
            "tree/subvalidation",
            &["tree", "validation"]
        ));
        assert!(!chain_ends_with("validation", &["tree", "validation"]));
        assert!(!chain_ends_with("tree", &[]));
    }

    #[test]
    fn path_sets_judge_co_location_over_first_level_members() {
        let pair = |left: &str, right: &str| PathSetAssertion {
            paths: vec![left.to_owned(), right.to_owned()],
            mode: None,
            because: "the pair's relation holds in the best state".to_owned(),
            expect_fail: None,
        };
        let laminar = laminar_candidate();
        let laminar_inputs = inputs_with(&laminar);
        let torn = torn_candidate();
        let torn_inputs = inputs_with(&torn);

        let apart = evaluate_path_set(
            &pair("billing/invoice.py", "telemetry/sink.py"),
            FaceMode::Anchored,
            &laminar_inputs,
            false,
        );
        assert_eq!(
            apart.label,
            "separate(billing/invoice.py+telemetry/sink.py)#Anchored"
        );
        assert!(apart.passed);
        assert_eq!(
            apart.detail,
            "no folder/domain container co-locates the set"
        );

        let welded = evaluate_path_set(
            &pair("billing/invoice.py", "pipeline.py"),
            FaceMode::Anchored,
            &torn_inputs,
            false,
        );
        assert!(!welded.passed);
        assert_eq!(
            welded.detail,
            "cross-directory welding: \"helpers\" co-locates billing/invoice.py with pipeline.py"
        );

        let together = evaluate_path_set(
            &pair("billing/invoice.py", "billing/pricing.py"),
            FaceMode::Anchored,
            &laminar_inputs,
            true,
        );
        assert!(together.passed);
        assert_eq!(
            together.detail,
            "some folder/domain container co-locates the set"
        );

        let scattered = evaluate_path_set(
            &pair("billing/invoice.py", "telemetry/sink.py"),
            FaceMode::Anchored,
            &laminar_inputs,
            true,
        );
        assert!(!scattered.passed);
        assert_eq!(
            scattered.detail,
            "no folder/domain container holds all of [billing/invoice.py, telemetry/sink.py]"
        );
    }
}
