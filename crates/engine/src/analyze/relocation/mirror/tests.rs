#![allow(clippy::assertions_on_constants)]

use std::collections::{BTreeMap, BTreeSet};

use smol_str::SmolStr;
use unlace_core::cluster::{ClusterId, Partition};
use unlace_ir::{Container, Node, Polarity, ScopeLevel};

use crate::config::{AnalyzeConfig, ObjectiveConfig, TestMirrorRule};
#[cfg(test)]
use crate::narrate::physical_file_folders;

use super::*;
use crate::analyze::relocation::*;
use crate::analyze::test_policy::TestPolicy;
use crate::analyze::test_support::*;

#[test]
#[allow(clippy::too_many_lines)]
fn should_price_one_exact_mirror_before_finish_and_replay_idempotently() {
    let snap = snapshot(
        vec![
            node(0, "perform_task", 4, Polarity::Production),
            node(1, "own_task", 5, Polarity::Production),
            node(2, "task_spec", 6, Polarity::TestCase),
        ],
        vec![edge(0, 1), edge(2, 0)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "source/module", ScopeLevel::Folder, Some(0)),
            container(2, "source/target", ScopeLevel::Folder, Some(0)),
            container(3, "spec/module", ScopeLevel::Folder, Some(0)),
            container(4, "source/module/task.ts", ScopeLevel::File, Some(1)),
            container(5, "source/target/owner.ts", ScopeLevel::File, Some(2)),
            container(6, "spec/module/task.spec.ts", ScopeLevel::File, Some(3)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut before = solver.real_partition.clone();
    let source = scc_of(4);
    let owner = scc_of(5);
    let mirror = scc_of(6);
    let owner_cluster = before.cluster_of(owner);
    assert!(owner_cluster.is_some(), "owner is placed");
    let Some(owner_cluster) = owner_cluster else {
        return;
    };
    assert!(before.move_node(source, owner_cluster));

    let score_before = solver.evaluate(&before);
    let mut after = before.clone();
    let evidence = solver.shadow_tests(&mut after, Vec::new());
    assert_eq!(evidence.outcomes.len(), 1);
    assert_eq!(
        evidence.outcomes.first().map(|outcome| outcome.disposition),
        Some(MirrorDisposition::Applied)
    );
    let changed: Vec<usize> = before
        .assignment()
        .iter()
        .zip(after.assignment())
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some(index))
        .collect();
    assert_eq!(changed, vec![mirror as usize]);

    let recompute = |parts: &Partition, replay: &PolishEvidence| {
        let assembled = solver.assemble_with_polish_evidence(parts, replay);
        let placement = |node: &Node| assembled.placement.get(&node.id.0).copied();
        render_tree(
            &assembled.tree,
            &snap.ir().nodes,
            &placement,
            &assembled.key_by_id,
        )
        .ok()
        .map(|rendered| {
            (
                solver.evaluate_with_polish_evidence(parts, replay),
                solver.capacity_remainder_with_polish_evidence(
                    parts,
                    replay,
                    &rendered,
                    &config.profiles.anchored.capacity,
                ),
            )
        })
    };
    let projected =
        physical_file_folders(&solver.assemble_with_polish_evidence(&after, &evidence).tree);
    let follower_folder = projected
        .iter()
        .find(|(path, _)| path.ends_with("spec/module/task.spec.ts"))
        .map(|(_, folder)| folder);
    assert!(
        follower_folder
            .is_some_and(|folder| { folder.ends_with(&["spec".to_owned(), "target".to_owned()]) }),
        "an applied exact follower is projected under its test root: {projected:?}"
    );
    let post = recompute(&after, &evidence);
    assert!(post.is_some(), "candidate tree renders");
    let Some(post) = post else {
        return;
    };
    assert!(
        (post.0 - solver.evaluate_with_polish_evidence(&after, &evidence)).abs() < f64::EPSILON
    );
    assert!(
        (score_before - post.0).abs() >= f64::EPSILON,
        "the follower must enter final pricing"
    );

    let mut replayed = after.clone();
    let replay_evidence = solver.shadow_tests(&mut replayed, Vec::new());
    assert_eq!(replayed.assignment(), after.assignment());
    let replayed_post = recompute(&replayed, &replay_evidence);
    assert!(replayed_post.is_some(), "replayed candidate tree renders");
    let Some(replayed_post) = replayed_post else {
        return;
    };
    assert!((replayed_post.0 - post.0).abs() < f64::EPSILON);
    assert_eq!(replayed_post.1, post.1);
    assert_eq!(replay_evidence, evidence);
}

#[test]
fn should_attempt_mirrors_for_every_physical_source_move() {
    let snap = snapshot(
        vec![
            node(0, "move_by_assignment", 6, Polarity::Production),
            node(1, "move_by_projection", 7, Polarity::Production),
            node(2, "receive_source", 8, Polarity::Production),
            node(3, "assignment_spec", 9, Polarity::TestCase),
            node(4, "projection_spec", 10, Polarity::TestCase),
        ],
        vec![edge(0, 2)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "source/assignment", ScopeLevel::Folder, Some(0)),
            container(2, "source/projection", ScopeLevel::Folder, Some(0)),
            container(3, "source/target", ScopeLevel::Folder, Some(0)),
            container(4, "spec/assignment", ScopeLevel::Folder, Some(0)),
            container(5, "spec/projection", ScopeLevel::Folder, Some(0)),
            container(
                6,
                "source/assignment/assignment.ts",
                ScopeLevel::File,
                Some(1),
            ),
            container(
                7,
                "source/projection/projection.ts",
                ScopeLevel::File,
                Some(2),
            ),
            container(8, "source/target/owner.ts", ScopeLevel::File, Some(3)),
            container(
                9,
                "spec/assignment/assignment.spec.ts",
                ScopeLevel::File,
                Some(4),
            ),
            container(
                10,
                "spec/projection/projection.spec.ts",
                ScopeLevel::File,
                Some(5),
            ),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let mut solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let target = parts.cluster_of(scc_of(8));
    assert!(target.is_some(), "target is placed");
    let Some(target) = target else {
        return;
    };
    assert!(parts.move_node(scc_of(6), target));

    let projected = parts.cluster_of(scc_of(7));
    assert!(projected.is_some(), "projected source is placed");
    let Some(projected) = projected else {
        return;
    };
    let Some(name) = solver.real_folder_names.get_mut(projected.0 as usize) else {
        return;
    };
    *name = SmolStr::new("source/projected");

    let evidence = solver.shadow_tests(&mut parts, Vec::new());
    let applied_sources: BTreeSet<&str> = evidence
        .outcomes
        .iter()
        .filter(|outcome| outcome.disposition == MirrorDisposition::Applied)
        .map(|outcome| outcome.source_path.as_str())
        .collect();

    assert_eq!(
        applied_sources,
        BTreeSet::from([
            "workspace/source/assignment/assignment.ts",
            "workspace/source/projection/projection.ts",
        ]),
        "every physical source move attempts its exact mirror"
    );
}

#[test]
fn should_attempt_a_mirror_when_the_source_moves_to_its_namespace_root() {
    let snap = snapshot(
        vec![
            node(0, "nested_source", 3, Polarity::Production),
            node(1, "root_owner", 4, Polarity::Production),
            node(2, "nested_spec", 5, Polarity::TestCase),
        ],
        vec![edge(0, 1)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "source/nested", ScopeLevel::Folder, Some(0)),
            container(2, "spec/nested", ScopeLevel::Folder, Some(0)),
            container(3, "source/nested/item.ts", ScopeLevel::File, Some(1)),
            container(4, "owner.ts", ScopeLevel::File, Some(0)),
            container(5, "spec/nested/item.spec.ts", ScopeLevel::File, Some(2)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let target = parts.cluster_of(scc_of(4));
    assert!(target.is_some(), "root owner is placed");
    let Some(target) = target else {
        return;
    };
    assert!(parts.move_node(scc_of(3), target));

    let evidence = solver.shadow_tests(&mut parts, Vec::new());

    assert_eq!(
        evidence
            .outcomes
            .iter()
            .map(|outcome| outcome.source_path.as_str())
            .collect::<Vec<_>>(),
        vec!["workspace/source/nested/item.ts"],
        "a source moved to its namespace root still attempts its exact mirror"
    );
    assert_eq!(
        evidence
            .outcomes
            .first()
            .map(|outcome| outcome.intended_to.as_str()),
        Some("workspace/spec"),
        "a synthetic source-root cluster projects to the test root, not a workspace folder"
    );
}

fn repeated_root_mirror_evidence(with_collision: bool) -> PolishEvidence {
    let mut containers = vec![
        container(0, "app", ScopeLevel::PackageGroup, None),
        container(1, "app/source/module", ScopeLevel::Folder, Some(0)),
        container(2, "app/source/target", ScopeLevel::Folder, Some(0)),
        container(3, "app/spec/module", ScopeLevel::Folder, Some(0)),
        container(4, "app/source/module/task.ts", ScopeLevel::File, Some(1)),
        container(5, "app/source/target/owner.ts", ScopeLevel::File, Some(2)),
        container(6, "app/spec/module/task.spec.ts", ScopeLevel::File, Some(3)),
    ];
    let mut nodes = vec![
        node(0, "perform_task", 4, Polarity::Production),
        node(1, "own_task", 5, Polarity::Production),
        node(2, "task_spec", 6, Polarity::TestCase),
    ];
    if with_collision {
        containers.extend([
            container(7, "app/spec/target", ScopeLevel::Folder, Some(0)),
            container(8, "app/spec/target/task.spec.ts", ScopeLevel::File, Some(7)),
        ]);
        nodes.push(node(3, "existing_spec", 8, Polarity::TestCase));
    }
    let snap = snapshot(nodes, vec![edge(0, 1)], containers);
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "app/source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["app/spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.pass_start_partition.clone();
    let target = parts.cluster_of(scc(5)).unwrap_or(ClusterId(0));
    assert!(parts.move_node(scc(4), target));
    solver.shadow_tests(&mut parts, Vec::new())
}

#[test]
fn should_link_an_exact_mirror_when_a_real_directory_repeats_the_repository_root() {
    let evidence = repeated_root_mirror_evidence(false);
    let outcome = evidence.outcomes.first();

    assert_eq!(evidence.outcomes.len(), 1);
    assert_eq!(
        outcome.map(|outcome| outcome.disposition),
        Some(MirrorDisposition::Applied)
    );
    assert_eq!(
        outcome.map(|outcome| outcome.source_path.as_str()),
        Some("app/app/source/module/task.ts")
    );
}

#[test]
fn should_detect_a_mirror_collision_beneath_a_repeated_repository_root() {
    let evidence = repeated_root_mirror_evidence(true);
    let outcome = evidence.outcomes.first();

    assert_eq!(evidence.outcomes.len(), 1);
    assert_eq!(
        outcome.map(|outcome| outcome.disposition),
        Some(MirrorDisposition::Blocked(MirrorBlock::PathCollision))
    );
}

#[test]
fn should_assemble_a_root_mirror_at_the_repository_root_exactly_once() {
    let snap = snapshot(
        vec![
            node(0, "nested_source", 3, Polarity::Production),
            node(1, "root_owner", 4, Polarity::Production),
            node(2, "nested_spec", 5, Polarity::TestCase),
        ],
        vec![edge(0, 1)],
        vec![
            container(0, "app", ScopeLevel::PackageGroup, None),
            container(1, "source/nested", ScopeLevel::Folder, Some(0)),
            container(2, "nested", ScopeLevel::Folder, Some(0)),
            container(3, "source/nested/item.ts", ScopeLevel::File, Some(1)),
            container(4, "owner.ts", ScopeLevel::File, Some(0)),
            container(5, "nested/item.spec.ts", ScopeLevel::File, Some(2)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let mut solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let target = parts.cluster_of(scc(4)).unwrap_or(ClusterId(0));
    if let Some(folder) = solver.real_folder_names.get_mut(target.0 as usize) {
        *folder = SmolStr::new("");
    }
    assert!(parts.move_node(scc(3), target));

    let evidence = solver.shadow_tests(&mut parts, Vec::new());
    assert_eq!(
        evidence.outcomes.first().map(|outcome| outcome.disposition),
        Some(MirrorDisposition::Applied)
    );
    assert_eq!(
        evidence
            .outcomes
            .first()
            .map(|outcome| outcome.intended_to.as_str()),
        Some("app")
    );
    let assembled = solver.assemble_with_polish_evidence(&parts, &evidence);
    let candidate_file = assembled
        .pass_start_file_by_candidate
        .iter()
        .find_map(|(candidate, original)| (original.0 == 5).then_some(*candidate));
    let by_id: BTreeMap<u32, &Container> = assembled
        .tree
        .containers()
        .iter()
        .map(|container| (container.id.0, container))
        .collect();
    let folder_name = candidate_file
        .and_then(|file| by_id.get(&file.0))
        .and_then(|file| file.parent)
        .and_then(|folder| by_id.get(&folder.0))
        .map(|folder| folder.name.as_str());
    assert_eq!(folder_name, Some(""));
}

#[derive(Clone, Copy)]
enum MirrorBlockFixture {
    Ambiguous,
    Package,
    Namespace,
    Capacity,
    Collision,
}

/// Makes the spec tree its own manifest package, so a follower would leave
/// its package to trail the source into `source/target`.
fn move_spec_tree_into_its_own_package(containers: &mut Vec<Container>) {
    containers.push(container(10, "spec", ScopeLevel::Package, Some(0)));
    for spec_folder in containers
        .iter_mut()
        .filter(|entry| entry.id.0 == 4 || entry.id.0 == 5)
    {
        spec_folder.parent = Some(ContainerId(10));
    }
}

fn actual_mirror_run(fixture: MirrorBlockFixture) -> (PolishEvidence, Partition, Partition) {
    let mut containers = vec![
        container(0, "workspace", ScopeLevel::PackageGroup, None),
        container(1, "source/a", ScopeLevel::Folder, Some(0)),
        container(2, "source/b", ScopeLevel::Folder, Some(0)),
        container(3, "source/target", ScopeLevel::Folder, Some(0)),
        container(4, "spec/a", ScopeLevel::Folder, Some(0)),
        container(5, "spec/target", ScopeLevel::Folder, Some(0)),
        container(6, "source/a/task.ts", ScopeLevel::File, Some(1)),
        container(7, "source/b/task.ts", ScopeLevel::File, Some(2)),
        container(8, "source/target/owner.ts", ScopeLevel::File, Some(3)),
        container(9, "spec/a/task.spec.ts", ScopeLevel::File, Some(4)),
    ];
    let mut nodes = vec![
        node(0, "perform_task", 6, Polarity::Production),
        node(1, "alternate_task", 7, Polarity::Production),
        node(2, "own_task", 8, Polarity::Production),
        node(3, "task_spec", 9, Polarity::TestCase),
    ];
    let mut edges = vec![edge(0, 2)];
    match fixture {
        MirrorBlockFixture::Package => move_spec_tree_into_its_own_package(&mut containers),
        MirrorBlockFixture::Namespace => {
            containers.push(container(10, "support", ScopeLevel::Folder, Some(0)));
            containers.push(container(
                11,
                "support/runtime.ts",
                ScopeLevel::File,
                Some(10),
            ));
            nodes.push(node(4, "support", 11, Polarity::Production));
            edges.extend([edge(3, 4), edge(4, 3)]);
        }
        MirrorBlockFixture::Capacity => {
            containers.push(container(
                10,
                "spec/a/support.spec.ts",
                ScopeLevel::File,
                Some(4),
            ));
            nodes.push(node(4, "support_spec", 10, Polarity::TestCase));
            edges.extend([edge(3, 4), edge(4, 3)]);
        }
        MirrorBlockFixture::Collision => {
            containers.push(container(
                10,
                "spec/target/task.spec.ts",
                ScopeLevel::File,
                Some(5),
            ));
            nodes.push(node(4, "existing_spec", 10, Polarity::TestCase));
        }
        MirrorBlockFixture::Ambiguous => {}
    }
    let snap = snapshot(nodes, edges, containers);
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules =
        if matches!(fixture, MirrorBlockFixture::Ambiguous) {
            ["a", "b"]
                .into_iter()
                .map(|branch| TestMirrorRule {
                    source: format!("source/{branch}/{{dir}}/{{stem}}.ts"),
                    tests: vec!["spec/a/{dir}/{stem}.spec.ts".to_owned()],
                })
                .collect()
        } else {
            vec![TestMirrorRule {
                source: "source/{dir}/{stem}.ts".to_owned(),
                tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
            }]
        };
    if matches!(fixture, MirrorBlockFixture::Capacity) {
        config.profiles.anchored.capacity.folder = 1;
    }
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.pass_start_partition.clone();
    let target = parts.cluster_of(scc(8));
    assert!(target.is_some(), "owner is placed");
    let target = target.unwrap_or(ClusterId(0));
    assert!(parts.move_node(scc(6), target));

    let evidence = solver.shadow_tests(&mut parts, Vec::new());
    (evidence, parts, solver.pass_start_partition.clone())
}

fn actual_mirror_evidence(fixture: MirrorBlockFixture) -> PolishEvidence {
    actual_mirror_run(fixture).0
}

fn assert_actual_mirror_block(fixture: MirrorBlockFixture, expected: MirrorBlock) {
    let (evidence, parts, pass_start) = actual_mirror_run(fixture);

    assert_eq!(evidence.outcomes.len(), 1, "one follower was attempted");
    assert_eq!(
        evidence.outcomes.first().map(|outcome| outcome.disposition),
        Some(MirrorDisposition::Blocked(expected))
    );
    // ADR-26: the refused test drops the whole move, so the source that had
    // moved is back in its pass-start cluster and the test never left.
    assert_eq!(
        parts.assignment(),
        pass_start.assignment(),
        "a source whose paired test cannot follow must not move"
    );
    assert!(evidence.dropped_a_move());
}

#[test]
fn should_leave_a_test_unpaired_when_two_rules_name_different_sources_for_it() {
    // two rules claim spec/a/task.spec.ts for source/a and source/b: unsure, so
    // no link at all (no follower attempted, nothing blocked) and the source move
    // proceeds as an ordinary unpaired move.
    let evidence = actual_mirror_evidence(MirrorBlockFixture::Ambiguous);
    assert_eq!(evidence.outcomes.len(), 0);
}

#[test]
fn should_report_actual_package_mirror_attempt() {
    assert_actual_mirror_block(MirrorBlockFixture::Package, MirrorBlock::PackageBoundary);
}

#[test]
fn should_report_actual_namespace_mirror_attempt() {
    assert_actual_mirror_block(
        MirrorBlockFixture::Namespace,
        MirrorBlock::NamespaceBoundary,
    );
}

#[test]
fn should_report_actual_capacity_mirror_attempt() {
    assert_actual_mirror_block(MirrorBlockFixture::Capacity, MirrorBlock::Capacity);
}

#[test]
fn should_report_actual_path_collision_mirror_attempt() {
    assert_actual_mirror_block(MirrorBlockFixture::Collision, MirrorBlock::PathCollision);
}

#[test]
fn should_not_shadow_by_basename_when_exact_mirroring_is_disabled() {
    let snap = snapshot(
        vec![
            node(0, "perform_task", 4, Polarity::Production),
            node(1, "own_task", 5, Polarity::Production),
            node(2, "task_spec", 6, Polarity::TestCase),
        ],
        vec![edge(0, 1), edge(2, 0)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "source/module", ScopeLevel::Folder, Some(0)),
            container(2, "source/target", ScopeLevel::Folder, Some(0)),
            container(3, "spec/module", ScopeLevel::Folder, Some(0)),
            container(4, "source/module/task.ts", ScopeLevel::File, Some(1)),
            container(5, "source/target/owner.ts", ScopeLevel::File, Some(2)),
            container(6, "spec/module/task.spec.ts", ScopeLevel::File, Some(3)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.enabled = false;
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let source = scc_of(4);
    let owner = scc_of(5);
    let mirror = scc_of(6);
    let owner_cluster = parts.cluster_of(owner);
    assert!(owner_cluster.is_some(), "owner is placed");
    let Some(owner_cluster) = owner_cluster else {
        return;
    };
    assert!(parts.move_node(source, owner_cluster));
    let before = parts.cluster_of(mirror);

    solver.shadow_tests(&mut parts, Vec::new());

    assert_eq!(parts.cluster_of(mirror), before);
}

#[test]
fn should_not_duplicate_an_unfollowed_namespace_during_assembly() {
    let snap = snapshot(
        vec![
            node(0, "subject", 3, Polarity::Production),
            node(1, "subject_spec", 4, Polarity::TestCase),
        ],
        vec![],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "src", ScopeLevel::Folder, Some(0)),
            container(2, "spec", ScopeLevel::Folder, Some(0)),
            container(3, "src/subject.ts", ScopeLevel::File, Some(1)),
            container(4, "spec/subject.spec.ts", ScopeLevel::File, Some(2)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.enabled = false;
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let spec_cluster = parts.cluster_of(scc_of(4));
    assert!(spec_cluster.is_some(), "spec folder is placed");
    let Some(spec_cluster) = spec_cluster else {
        return;
    };
    assert!(parts.move_node(scc_of(3), spec_cluster));

    let assembled = solver.assemble(&parts);
    let folders: Vec<&str> = assembled
        .tree
        .containers()
        .iter()
        .filter(|container| container.level == ScopeLevel::Folder)
        .map(|container| container.name.as_str())
        .collect();

    assert!(
        !folders.iter().any(|folder| folder.contains("spec/spec")),
        "an ordinary mixed cluster must not manufacture a duplicate namespace: {folders:?}"
    );
}

#[test]
fn should_keep_an_ambiguously_twinned_spec_where_it_is() {
    // Two production files reduce to the same stem in the same package —
    // no unique twin, so the shadow pass must leave the spec alone (ADR-16:
    // unchanged placements are never presented as suggested actions).
    let nodes = vec![
        node(0, "one", 4, Polarity::Production),
        node(1, "two", 5, Polarity::Production),
        node(2, "spec", 3, Polarity::TestCase),
    ];
    let edges = vec![edge(2, 0)];
    let containers = vec![
        container(0, "workspace", ScopeLevel::PackageGroup, None),
        container(1, "spec", ScopeLevel::Folder, Some(0)),
        container(2, "alt", ScopeLevel::Folder, Some(0)),
        container(3, "spec/openai.spec.ts", ScopeLevel::File, Some(1)),
        container(4, "alt/openai.ts", ScopeLevel::File, Some(2)),
        container(5, "alt/openai.tsx", ScopeLevel::File, Some(2)),
    ];
    let snap = snapshot(nodes, edges, containers);

    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &AnalyzeConfig::default(),
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );

    let scc_of = |file_container: u32| -> u32 {
        let vertex = solver
            .index_of
            .get(&file_container)
            .copied()
            .unwrap_or(u32::MAX);
        solver
            .condensation
            .membership
            .get(vertex as usize)
            .map_or(u32::MAX, |scc| scc.0)
    };

    let mut parts = solver.real_partition.clone();
    let unit = scc_of(3);
    let before = parts.cluster_of(unit);
    assert!(before.is_some(), "setup: the spec starts placed");
    assert_ne!(
        before,
        parts.cluster_of(scc_of(4)),
        "setup: the spec starts apart from its would-be twins"
    );

    solver.shadow_tests(&mut parts, Vec::new());

    assert_eq!(parts.cluster_of(unit), before);
}

#[test]
fn should_veto_a_shadow_move_that_would_overflow_the_folder_cap() {
    // A folder cap of one leaves no room next to the twin; the cap veto
    // binds exactly like it does for polish moves.
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.capacity.folder = 1;
    let nodes = vec![
        node(0, "openai", 3, Polarity::Production),
        node(1, "openai_spec", 4, Polarity::TestCase),
    ];
    let edges = vec![edge(1, 0)];
    let containers = vec![
        container(0, "workspace", ScopeLevel::PackageGroup, None),
        container(1, "src", ScopeLevel::Folder, Some(0)),
        container(2, "spec", ScopeLevel::Folder, Some(0)),
        container(3, "src/openai.ts", ScopeLevel::File, Some(1)),
        container(4, "spec/openai.spec.ts", ScopeLevel::File, Some(2)),
    ];
    let snap = snapshot(nodes, edges, containers);

    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );

    let scc_of = |file_container: u32| -> u32 {
        let vertex = solver
            .index_of
            .get(&file_container)
            .copied()
            .unwrap_or(u32::MAX);
        solver
            .condensation
            .membership
            .get(vertex as usize)
            .map_or(u32::MAX, |scc| scc.0)
    };

    let mut parts = solver.real_partition.clone();
    let unit = scc_of(4);
    let subject = scc_of(3);
    assert_ne!(
        parts.cluster_of(unit),
        parts.cluster_of(subject),
        "setup: real dirs must start the pair apart"
    );

    solver.shadow_tests(&mut parts, Vec::new());

    assert_ne!(
        parts.cluster_of(unit),
        parts.cluster_of(subject),
        "the cap veto binds even against a unique twin"
    );
}

#[test]
fn should_allow_a_childs_only_file_to_replace_its_folder_entry_at_the_parent_cap() {
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.capacity.folder = 2;
    let snap = snapshot(
        vec![
            node(0, "anchor", 3, Polarity::Production),
            node(1, "subject", 4, Polarity::Production),
        ],
        vec![edge(1, 0)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "area", ScopeLevel::Domain, Some(0)),
            container(2, "area/child", ScopeLevel::Folder, Some(1)),
            container(3, "area/anchor.ts", ScopeLevel::File, Some(1)),
            container(4, "area/child/subject.ts", ScopeLevel::File, Some(2)),
        ],
    );
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        false,
        &tests,
    );
    let unit_vertex = solver.index_of.get(&4).copied().unwrap_or(u32::MAX);
    let unit = solver
        .condensation
        .membership
        .get(unit_vertex as usize)
        .map_or(u32::MAX, |scc| scc.0);
    let target_vertex = solver.index_of.get(&3).copied().unwrap_or(u32::MAX);
    let target = solver
        .condensation
        .membership
        .get(target_vertex as usize)
        .and_then(|scc| solver.real_partition.cluster_of(scc.0));

    assert!(
        target.is_some_and(|target| {
            solver.permits_physical_capacity(&solver.real_partition, unit, target)
        }),
        "the child-folder entry is replaced by its only file, so the parent remains at cap"
    );
}

fn mirror_outcome(source: &str, test: &str, disposition: MirrorDisposition) -> MirrorOutcome {
    MirrorOutcome {
        mirror_scc: 0,
        source_path: source.to_owned(),
        path: test.to_owned(),
        from: "spec".to_owned(),
        intended_to: "lib".to_owned(),
        disposition,
    }
}

fn file_move(path: &str) -> crate::result::Move {
    crate::result::Move {
        kind: crate::result::MoveKind::Move,
        files: vec![crate::result::FileMove {
            path: path.to_owned(),
            from: "src".to_owned(),
        }],
        to: "lib".to_owned(),
        reason: crate::result::MoveReason::Follows {
            subject: path.to_owned(),
        },
        mirrors: Vec::new(),
    }
}

/// ADR-26: only applied followers attach, each to the move of its own source,
/// sorted and deduplicated; a follower's own move then disappears, while an
/// outcome whose source made no move attaches nowhere and removes nothing.
#[test]
fn should_attach_only_applied_followers_to_their_source_move() {
    let mut moves = vec![
        file_move("src/a.ts"),
        file_move("spec/a.spec.ts"),
        file_move("spec/orphan.spec.ts"),
    ];
    let evidence = PolishEvidence {
        outcomes: vec![
            mirror_outcome("src/a.ts", "spec/a.test.ts", MirrorDisposition::Applied),
            mirror_outcome("src/a.ts", "spec/a.spec.ts", MirrorDisposition::Applied),
            mirror_outcome("src/a.ts", "spec/a.spec.ts", MirrorDisposition::Applied),
            mirror_outcome(
                "src/a.ts",
                "spec/blocked.spec.ts",
                MirrorDisposition::Blocked(MirrorBlock::Capacity),
            ),
            mirror_outcome(
                "src/unmoved.ts",
                "spec/orphan.spec.ts",
                MirrorDisposition::Applied,
            ),
        ],
        folds: Vec::new(),
    };

    PipelineSolver::attach_test_mirrors(&mut moves, &evidence);

    let summary: Vec<(String, Vec<String>)> = moves
        .iter()
        .map(|entry| {
            (
                entry.files.iter().map(|file| file.path.clone()).collect(),
                entry
                    .mirrors
                    .iter()
                    .map(|mirror| mirror.path.clone())
                    .collect(),
            )
        })
        .map(|(files, mirrors): (Vec<String>, Vec<String>)| (files.join(","), mirrors))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "src/a.ts".to_owned(),
                vec!["spec/a.spec.ts".to_owned(), "spec/a.test.ts".to_owned()]
            ),
            ("spec/orphan.spec.ts".to_owned(), Vec::new()),
        ]
    );
}

fn bare_file(container: u32, path: &str) -> FileInfo {
    FileInfo {
        container,
        name: SmolStr::new(path),
        production_sloc: 1,
        namespace: SmolStr::new(""),
        home: crate::analyze::layout::LaminarHome {
            folder: SmolStr::new(""),
            domain: SmolStr::new(""),
            package: SmolStr::new(""),
            synthetic: false,
            label_keyed: false,
        },
        directory: SmolStr::new(path.rsplit_once('/').map_or("", |(directory, _)| directory)),
    }
}

#[test]
fn should_let_a_rule_pair_a_test_whose_adapter_pair_cannot_be_honored() {
    let files = [
        bare_file(0, "source/a/task.ts"),
        bare_file(1, "spec/a/task.spec.ts"),
        bare_file(2, "spec/a/helper.ts"),
    ];
    let test_zone = [false, true, true];
    let adapter_pairs = [
        // the stated test is not a present file: nothing to claim.
        (
            SmolStr::new("spec/a/missing.spec.ts"),
            SmolStr::new("source/a/task.ts"),
        ),
        // the stated production side sits in the test zone: not a pair.
        (
            SmolStr::new("spec/a/task.spec.ts"),
            SmolStr::new("spec/a/helper.ts"),
        ),
    ];
    let rules = [TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];

    let links = links::exact_links(&files, &test_zone, &adapter_pairs, &rules);

    let paired: Vec<(u32, Vec<u32>)> = links
        .iter()
        .map(|(test, sources)| (*test, sources.keys().copied().collect()))
        .collect();
    assert_eq!(paired, vec![(1, vec![0])]);
}

#[test]
fn should_let_an_adapter_pair_claim_its_test_before_any_rule() {
    let files = [
        bare_file(0, "source/b/job.ts"),
        bare_file(1, "source/b/other.ts"),
        bare_file(2, "spec/b/job.spec.ts"),
    ];
    let adapter_pairs = [(
        SmolStr::new("spec/b/job.spec.ts"),
        SmolStr::new("source/b/other.ts"),
    )];
    let rules = [TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];

    let links = links::exact_links(&files, &[false, false, true], &adapter_pairs, &rules);

    // the rule would pair the test with `job.ts`; the stated pair wins.
    let paired: Vec<(u32, Vec<u32>)> = links
        .iter()
        .map(|(test, sources)| (*test, sources.keys().copied().collect()))
        .collect();
    assert_eq!(paired, vec![(2, vec![1])]);
}

/// Runs `inspect` against a solver over `src/a/task.ts` and its
/// `spec/a/task.spec.ts` mirror, where `src` is a transparent source root the
/// laminar tree strips.
fn with_transparent_root_solver<T>(inspect: impl FnOnce(&PipelineSolver<'_>) -> T) -> T {
    let (source, spec) = (
        SmolStr::new("src/a/task.ts"),
        SmolStr::new("spec/a/task.spec.ts"),
    );
    let paths = [source.clone(), spec.clone()];
    let layout = unlace_ir::Layout {
        package_roots: Vec::new(),
        source_roots: vec![SmolStr::new("src")],
    };
    let built = unlace_ir::build_laminar_tree(&paths, "app", &layout);
    let file_of = |path: &SmolStr| built.files.get(path).map_or(u32::MAX, |file| file.0);
    let nodes = vec![
        node(0, "perform_task", file_of(&source), Polarity::Production),
        node(1, "task_spec", file_of(&spec), Polarity::TestCase),
    ];
    let snap = snapshot(nodes, vec![edge(1, 0)], built.tree.containers().to_vec());
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "src/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    inspect(&solver)
}

/// Projects the mirror into folder `b` three ways: through the real link, a
/// link without a source root, and a link rooted elsewhere.
fn transparent_root_projections(solver: &PipelineSolver<'_>) -> Vec<Option<String>> {
    let links: Vec<ExactMirrorLink> = solver
        .exact_mirror_links()
        .into_values()
        .flat_map(BTreeMap::into_values)
        .collect();
    let rootless = links.iter().map(|link| ExactMirrorLink {
        source_root: String::new(),
        ..link.clone()
    });
    let foreign = links.iter().map(|link| ExactMirrorLink {
        source_root: "lib".to_owned(),
        ..link.clone()
    });
    let folder = [solver.root_name.to_string(), "b".to_owned()];
    links
        .iter()
        .cloned()
        .chain(rootless)
        .chain(foreign)
        .map(|link| solver.project_mirror_path(&link, &folder))
        .collect()
}

#[test]
fn should_project_a_mirror_through_a_transparent_source_root() {
    let mirrored = Some("app/spec/b/task.spec.ts".to_owned());

    // the laminar folder `b` omits the transparent `src` the rule's source
    // root names, so it is reapplied; a rootless rule takes the folder as is;
    // a root the source never lived under cannot be projected.
    assert_eq!(
        with_transparent_root_solver(transparent_root_projections),
        vec![mirrored.clone(), mirrored, None]
    );
}

/// Whether the mirror's path is claimed by the source file, placed as at
/// pass start and then parked in a cluster with no real folder.
fn source_occupancy(solver: &PipelineSolver<'_>) -> (bool, bool) {
    let evidence = PolishEvidence {
        outcomes: Vec::new(),
        folds: Vec::new(),
    };
    let vertex_of = |path: &str| {
        solver
            .files
            .iter()
            .position(|file| file.name == path)
            .and_then(|vertex| u32::try_from(vertex).ok())
            .unwrap_or(u32::MAX)
    };
    let (source, mirror) = (vertex_of("src/a/task.ts"), vertex_of("spec/a/task.spec.ts"));
    let source_path = solver.repository_path("app/a/task.ts");
    let start = solver.pass_start_partition.clone();
    let folderless = ClusterId(u32::try_from(start.cluster_count()).unwrap_or(u32::MAX));
    let mut parked =
        Partition::from_assignment(start.assignment().to_vec(), start.cluster_count() + 1);
    let moved = solver
        .scc_of_vertex(source)
        .is_some_and(|scc| parked.move_node(scc, folderless));
    assert!(moved, "the source parks in the folderless cluster");
    (
        solver.mirror_path_occupied(&start, mirror, &source_path, &evidence),
        solver.mirror_path_occupied(&parked, mirror, &source_path, &evidence),
    )
}

#[test]
fn should_not_count_a_file_without_a_projected_folder_as_occupying_a_path() {
    assert_eq!(
        with_transparent_root_solver(source_occupancy),
        (true, false)
    );
}

#[test]
fn should_strip_only_the_repository_root_from_a_rooted_projected_folder() {
    let relative = |folder: ProjectedFolder| folder.repository_relative("app").to_string();

    assert_eq!(
        [
            relative(ProjectedFolder::Rooted(SmolStr::new("/app//spec/a"))),
            relative(ProjectedFolder::Rooted(SmolStr::new("vendor/spec"))),
            relative(ProjectedFolder::RepositoryRelative(SmolStr::new(
                "app/spec"
            ))),
        ],
        ["spec/a", "vendor/spec", "app/spec"]
    );
}

#[test]
fn should_send_a_drifted_test_home_while_its_source_stays_put() {
    let snap = snapshot(
        vec![
            node(0, "perform_task", 4, Polarity::Production),
            node(1, "own_task", 5, Polarity::Production),
            node(2, "task_spec", 6, Polarity::TestCase),
        ],
        vec![edge(0, 1), edge(2, 0)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "source/module", ScopeLevel::Folder, Some(0)),
            container(2, "source/target", ScopeLevel::Folder, Some(0)),
            container(3, "spec/module", ScopeLevel::Folder, Some(0)),
            container(4, "source/module/task.ts", ScopeLevel::File, Some(1)),
            container(5, "source/target/owner.ts", ScopeLevel::File, Some(2)),
            container(6, "spec/module/task.spec.ts", ScopeLevel::File, Some(3)),
        ],
    );
    let mut config = AnalyzeConfig::default();
    config.profiles.anchored.relocation.test_mirroring.builtins = false;
    config.profiles.anchored.relocation.test_mirroring.rules = vec![TestMirrorRule {
        source: "source/{dir}/{stem}.ts".to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    }];
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let mut parts = solver.real_partition.clone();
    let owner_cluster = parts.cluster_of(scc_of(5)).unwrap_or(ClusterId(u32::MAX));
    assert!(parts.move_node(scc_of(6), owner_cluster), "the test drifts");

    let evidence = solver.shadow_tests(&mut parts, Vec::new());

    assert_eq!(parts, solver.pass_start_partition, "the test is back home");
    assert_eq!(evidence.outcomes, Vec::new(), "nothing followed");
}

/// Runs `inspect` with the SCCs of `subject.ts`, `x/one.ts` and its spec, and the
/// clusters of `subject.ts`'s folder `s`, a folder `t` it never calls and the
/// synthetic workspace bucket holding a loose file. `subject.ts` calls one
/// file in `s` and two in a third folder `x`; its spec's call is unpriced.
fn with_veto_solver<T>(
    inspect: impl FnOnce(&PipelineSolver<'_>, &Partition, [u32; 3], [ClusterId; 3]) -> T,
) -> T {
    let snap = snapshot(
        vec![
            node(0, "subject", 4, Polarity::Production),
            node(1, "peer", 5, Polarity::Production),
            node(2, "one", 6, Polarity::Production),
            node(3, "two", 7, Polarity::Production),
            node(4, "goal", 8, Polarity::Production),
            node(5, "loose", 9, Polarity::Production),
            node(6, "subject_spec", 12, Polarity::TestCase),
        ],
        vec![edge(0, 1), edge(0, 2), edge(0, 3), edge(6, 0)],
        vec![
            container(0, "workspace", ScopeLevel::PackageGroup, None),
            container(1, "s", ScopeLevel::Folder, Some(0)),
            container(2, "t", ScopeLevel::Folder, Some(0)),
            container(3, "x", ScopeLevel::Folder, Some(0)),
            container(4, "s/subject.ts", ScopeLevel::File, Some(1)),
            container(5, "s/peer.ts", ScopeLevel::File, Some(1)),
            container(6, "x/one.ts", ScopeLevel::File, Some(3)),
            container(7, "x/two.ts", ScopeLevel::File, Some(3)),
            container(8, "t/goal.ts", ScopeLevel::File, Some(2)),
            container(9, "loose.ts", ScopeLevel::File, Some(10)),
            Container {
                synthetic: true,
                ..container(10, "workspace", ScopeLevel::Folder, Some(0))
            },
            container(11, "spec", ScopeLevel::Folder, Some(0)),
            container(12, "spec/subject.spec.ts", ScopeLevel::File, Some(11)),
        ],
    );
    let config = AnalyzeConfig::default();
    let tests = TestPolicy::defaults();
    let solver = PipelineSolver::new(
        &snap,
        &config,
        ObjectiveConfig::default().coefficients(),
        true,
        &tests,
    );
    let parts = solver.real_partition.clone();
    let scc_of = |file: u32| {
        solver
            .index_of
            .get(&file)
            .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
            .map_or(u32::MAX, |scc| scc.0)
    };
    let cluster_of = |file: u32| {
        parts
            .cluster_of(scc_of(file))
            .unwrap_or(ClusterId(u32::MAX))
    };
    let clusters = [cluster_of(4), cluster_of(8), cluster_of(9)];
    inspect(
        &solver,
        &parts,
        [scc_of(4), scc_of(6), scc_of(12)],
        clusters,
    )
}

#[test]
fn should_veto_leaving_pull_that_only_a_third_folder_outweighs() {
    let vetoes = with_veto_solver(|solver, parts, [subject, one, _], [source, target, _]| {
        let x = parts.cluster_of(one).unwrap_or(ClusterId(u32::MAX));
        [
            solver.absorbs_a_foreign_anchor(parts, subject, source, target),
            solver.strands_a_comparable_anchor(parts, subject, source, target),
            // the spec's unpriced call anchors nothing in its folder.
            solver.absorbs_a_foreign_anchor(parts, subject, source, x),
        ]
    });

    // the heavier pull into `x` awaits in neither folder, so it cannot tip
    // the balance toward the empty target.
    assert_eq!(vetoes, [true, true, false]);
}

#[test]
fn should_veto_fleeing_priced_company_only_into_the_workspace_bucket() {
    let flights = with_veto_solver(
        |solver, parts, [subject, one, spec], [source, target, bucket]| {
            let [x, spec_folder] =
                [one, spec].map(|scc| parts.cluster_of(scc).unwrap_or(ClusterId(u32::MAX)));
            [
                solver.real_folder_synthetic.get(bucket.0 as usize) == Some(&true),
                solver.flees_into_the_synthetic_bucket(parts, subject, source, bucket),
                solver.flees_into_the_synthetic_bucket(parts, subject, source, target),
                // `x/one.ts` has no priced company in its own folder.
                solver.flees_into_the_synthetic_bucket(parts, one, x, bucket),
                // nor does the spec's unpriced call keep `subject.ts` company.
                solver.flees_into_the_synthetic_bucket(parts, subject, spec_folder, bucket),
            ]
        },
    );

    assert_eq!(flights, [true, true, false, false, false]);
}

#[test]
fn should_pair_no_test_through_a_rule_whose_source_template_does_not_parse() {
    let files = [
        bare_file(0, "source/a/task.ts"),
        bare_file(1, "spec/a/task.spec.ts"),
    ];
    let rule = |source: &str| TestMirrorRule {
        source: source.to_owned(),
        tests: vec!["spec/{dir}/{stem}.spec.ts".to_owned()],
    };

    let parsed = links::exact_links(
        &files,
        &[false, true],
        &[],
        &[rule("source/{dir}/{stem}.ts")],
    );
    // `analyze` takes an unvalidated config: a source template without
    // `{dir}` matches no source, so it pairs nothing instead of failing.
    let unparsed = links::exact_links(&files, &[false, true], &[], &[rule("source/{stem}.ts")]);

    assert_eq!(parsed.keys().copied().collect::<Vec<_>>(), vec![1]);
    assert!(unparsed.is_empty());
}

/// A guard over `a/task.ts` (namespace `a`) and `b/peer.ts` (namespace `b`),
/// each alone in its own pass-start folder of one package.
fn two_namespace_guard() -> RelocationIdentityGuard {
    let file = |path: &str, namespace: &str| FileInfo {
        namespace: SmolStr::new(namespace),
        home: crate::analyze::layout::LaminarHome {
            package: SmolStr::new("p"),
            ..bare_file(0, path).home
        },
        ..bare_file(0, path)
    };
    let identity = Partition::from_assignment(vec![ClusterId(0), ClusterId(1)], 2);
    RelocationIdentityGuard::new(
        &[file("a/task.ts", "a"), file("b/peer.ts", "b")],
        &singleton_condensation(2),
        &identity,
        &identity,
        None,
    )
}

#[test]
fn should_admit_a_foreign_namespace_only_for_an_applied_follower() {
    let guard = two_namespace_guard();
    // `a/task.ts` (SCC 0) sits in `b`'s folder, which admits only namespace `b`.
    let crossed = Partition::from_assignment(vec![ClusterId(1), ClusterId(1)], 2);
    let follower = |disposition| PolishEvidence {
        outcomes: vec![mirror_outcome("a/task.ts", "a/task.ts", disposition)],
        folds: Vec::new(),
    };
    let applied = follower(MirrorDisposition::Applied);
    let blocked = follower(MirrorDisposition::Blocked(MirrorBlock::NamespaceBoundary));

    assert_eq!(applied.applied_sccs(), BTreeSet::from([0]));
    assert!(blocked.applied_sccs().is_empty());
    assert!(guard.accepts_with_mirrors(&crossed, &applied));
    assert!(!guard.accepts_with_mirrors(&crossed, &blocked));
    assert!(!guard.accepts_with_mirrors(&crossed, &PolishEvidence::default()));
}
