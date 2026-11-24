//! Assertions over container shape: size bands, synthetic buckets, and name
//! alignment.

use unlace_engine::result::{ContainerNode, Level};

use crate::harness::Verdict;
use crate::harness::inputs::EvalInputs;
use crate::metrics;
use crate::target::{BandScope, BucketName, FaceMode, NameAlignment, SizeBand};

/// `size_band`: every selected container's first-level member count lies
/// within the inclusive band. Only a folder's own children are its members —
/// nested sub-places contribute nothing — so splitting an over-cap folder into
/// halves that nest under it reads as two within-band places.
pub(super) fn evaluate_size_band(
    band: &SizeBand,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
) -> Verdict {
    let selector = match (&band.scope, &band.container) {
        (Some(BandScope::AnyContainer), _) => "any_container".to_owned(),
        (_, Some(name)) => format!("container({name})"),
        (None, None) => "unscoped".to_owned(),
    };
    let label = format!("size_band({selector},max={})#{face:?}", band.max_files);
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let mut worst: Option<(String, usize)> = None;
    {
        let mut visit = |node: &ContainerNode, _chain: &str| {
            let selected = match (&band.scope, &band.container) {
                (Some(BandScope::AnyContainer), _) => metrics::is_scoped_container(node),
                (_, Some(name)) => node.name == *name && node.level != Level::File,
                (None, None) => false,
            };
            if !selected {
                return;
            }
            let count = metrics::direct_members(node).len();
            let over_max = usize::try_from(band.max_files).is_ok_and(|max| count > max);
            let under_min = band
                .min_files
                .and_then(|min| usize::try_from(min).ok())
                .is_some_and(|min| count < min);
            if over_max || under_min {
                let worse = worst
                    .as_ref()
                    .is_none_or(|(_, worst_count)| count > *worst_count);
                if worse {
                    worst = Some((node.name.clone(), count));
                }
            }
        };
        metrics::walk_containers(tree, &mut visit);
    }

    match worst {
        None => Verdict {
            label,
            passed: true,
            detail: "every selected container sits within its band".to_owned(),
        },
        Some((name, count)) => {
            let bound = band.min_files.map_or_else(
                || format!("max {}", band.max_files),
                |min| format!("min {min}/max {}", band.max_files),
            );
            Verdict {
                label,
                passed: false,
                detail: format!(
                    "over-capacity: container {name:?} holds {count} members against {bound} (first-level members only)"
                ),
            }
        }
    }
}

/// `no_synthetic_bucket`: no non-file node may carry the forbidden last segment.
pub(super) fn evaluate_no_synthetic_bucket(
    bucket: &BucketName,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
) -> Verdict {
    let label = format!("no_synthetic_bucket({})#{face:?}", bucket.name);
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let mut offenders: Vec<String> = Vec::new();
    {
        let mut visit = |node: &ContainerNode, _chain: &str| {
            if node.level != Level::File && metrics::last_segment(&node.name) == bucket.name {
                offenders.push(format!("{:?} ({:?})", node.name, node.level));
            }
        };
        metrics::walk_containers(tree, &mut visit);
    }
    Verdict {
        label,
        passed: offenders.is_empty(),
        detail: if offenders.is_empty() {
            format!("no node carries the synthetic bucket {:?}", bucket.name)
        } else {
            format!(
                "workspace collapse: synthetic bucket {:?} appears at {}",
                bucket.name,
                offenders.join(", ")
            )
        },
    }
}

/// `name_alignment`: every folder/domain container with at least `min_members`
/// first-level members keeps at least `min_ratio` of them sharing a naming
/// token. A folder is judged by its own children; descendants belong to their
/// own places and never dilute an ancestor's alignment.
pub(super) fn evaluate_name_alignment(
    alignment: &NameAlignment,
    face: FaceMode,
    inputs: &EvalInputs<'_>,
) -> Verdict {
    let min_ratio = alignment.min_ratio;
    let min_members_requested = alignment.min_members;
    let label = format!(
        "name_alignment(min_ratio={min_ratio},min_members={min_members_requested})#{face:?}"
    );
    let Some(tree) = inputs.faces.get(&face).map(|face_inputs| face_inputs.tree) else {
        return Verdict {
            label: format!("#{face:?}"),
            passed: false,
            detail: format!("the {face:?} mode was not evaluated"),
        };
    };
    let min_members = usize::try_from(alignment.min_members).unwrap_or(usize::MAX);
    let mut worst: Option<(String, usize, usize)> = None;
    {
        // `walk_containers` visits folder/domain nodes only, so every visited
        // node is a scoped container.
        let mut visit = |node: &ContainerNode, _chain: &str| {
            let members = metrics::direct_members(node);
            if members.len() < min_members || members.is_empty() {
                return;
            }
            let (aligned, total) = metrics::alignment_counts(&node.name, &members);
            let below_floor =
                metrics::alignment_sharing(&node.name, &members) < min_ratio - f64::EPSILON;
            // the worst container aligns the smallest share of its members:
            // aligned/total < worst_aligned/worst_total, cross-multiplied.
            let worse = worst
                .as_ref()
                .is_none_or(|(_, worst_aligned, worst_total)| {
                    aligned * worst_total < worst_aligned * total
                });
            if below_floor && worse {
                worst = Some((node.name.clone(), aligned, total));
            }
        };
        metrics::walk_containers(tree, &mut visit);
    }

    match worst {
        None => Verdict {
            label,
            passed: true,
            detail: "every qualifying container aligns with its members".to_owned(),
        },
        Some((name, aligned, total)) => Verdict {
            label,
            passed: false,
            detail: format!(
                "naming incoherence: container {name:?} aligns {aligned}/{total} members below min_ratio {min_ratio}"
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use unlace_engine::result::Level;

    use unlace_engine::result::ContainerNode;

    use crate::harness::assertions::fixtures::{file, inputs_with, laminar_candidate, node};
    use crate::target::{BandScope, BucketName, FaceMode, NameAlignment, SizeBand};

    use super::{evaluate_name_alignment, evaluate_no_synthetic_bucket, evaluate_size_band};

    fn band(max_files: u32, min_files: Option<u32>) -> SizeBand {
        SizeBand {
            max_files,
            min_files,
            scope: Some(BandScope::AnyContainer),
            container: None,
            mode: None,
            because: "places stay small".to_owned(),
        }
    }

    fn alignment(min_ratio: f64) -> NameAlignment {
        NameAlignment {
            min_ratio,
            min_members: 2,
            mode: None,
            because: "names align".to_owned(),
        }
    }

    /// One package holding the given folders.
    fn package(folders: Vec<ContainerNode>) -> ContainerNode {
        node(
            Level::PackageGroup,
            "root",
            vec![node(Level::Package, "app", folders)],
        )
    }

    #[test]
    fn no_synthetic_bucket_reads_last_segments_at_any_level() {
        let bucketed = node(
            Level::PackageGroup,
            "root",
            vec![node(
                Level::Package,
                "app",
                vec![node(Level::Domain, "workspace", vec![file("loose.py")])],
            )],
        );
        let bucketed_inputs = inputs_with(&bucketed);
        let bucket = BucketName {
            name: "workspace".to_owned(),
            mode: None,
            because: "real directories never collapse into workspace".to_owned(),
        };
        let verdict = evaluate_no_synthetic_bucket(&bucket, FaceMode::Anchored, &bucketed_inputs);
        assert!(!verdict.passed);
        assert!(verdict.detail.contains("workspace"));

        let laminar = laminar_candidate();
        let laminar_inputs = inputs_with(&laminar);
        let laminar_verdict =
            evaluate_no_synthetic_bucket(&bucket, FaceMode::Anchored, &laminar_inputs);
        assert!(laminar_verdict.passed);
    }

    #[test]
    fn size_band_container_selector_matches_full_prefix_names_any_level() {
        let wide = node(
            Level::PackageGroup,
            "root",
            vec![node(
                Level::Package,
                "app",
                vec![node(
                    Level::Folder,
                    "hub",
                    vec![
                        file("hub/a.py"),
                        file("hub/b.py"),
                        file("hub/c.py"),
                        file("hub/d.py"),
                    ],
                )],
            )],
        );
        let inputs = inputs_with(&wide);
        let band = SizeBand {
            max_files: 2,
            min_files: None,
            scope: None,
            container: Some("hub".to_owned()),
            mode: None,
            because: "hub splits".to_owned(),
        };
        let verdict = evaluate_size_band(&band, FaceMode::Anchored, &inputs);
        assert!(!verdict.passed);
        assert!(verdict.detail.contains("holds 4 members"));
    }

    #[test]
    fn shape_assertions_fail_loud_on_an_unevaluated_face() {
        let laminar = laminar_candidate();
        let inputs = inputs_with(&laminar);
        let face = FaceMode::Greenfield;
        let bucket = BucketName {
            name: "workspace".to_owned(),
            mode: None,
            because: "no synthetic bucket".to_owned(),
        };
        for verdict in [
            evaluate_size_band(&band(3, None), face, &inputs),
            evaluate_no_synthetic_bucket(&bucket, face, &inputs),
            evaluate_name_alignment(&alignment(0.5), face, &inputs),
        ] {
            assert_eq!(verdict.label, "#Greenfield");
            assert!(!verdict.passed);
            assert_eq!(verdict.detail, "the Greenfield mode was not evaluated");
        }
    }

    #[test]
    fn size_band_reports_the_largest_offender_against_both_bounds() {
        let tree = package(vec![
            node(
                Level::Folder,
                "hub",
                vec![
                    file("hub/a.py"),
                    file("hub/b.py"),
                    file("hub/c.py"),
                    file("hub/d.py"),
                ],
            ),
            node(
                Level::Domain,
                "spoke",
                vec![file("spoke/e.py"), file("spoke/f.py"), file("spoke/g.py")],
            ),
            node(Level::Folder, "lone", vec![file("lone/h.py")]),
        ]);
        let inputs = inputs_with(&tree);

        let over = evaluate_size_band(&band(2, None), FaceMode::Anchored, &inputs);
        assert_eq!(over.label, "size_band(any_container,max=2)#Anchored");
        assert!(!over.passed);
        assert_eq!(
            over.detail,
            "over-capacity: container \"hub\" holds 4 members against max 2 (first-level members only)"
        );

        let under = evaluate_size_band(&band(4, Some(2)), FaceMode::Anchored, &inputs);
        assert!(!under.passed);
        assert_eq!(
            under.detail,
            "over-capacity: container \"lone\" holds 1 members against min 2/max 4 (first-level members only)"
        );

        let within = evaluate_size_band(&band(4, Some(1)), FaceMode::Anchored, &inputs);
        assert!(within.passed, "{}", within.detail);
    }

    #[test]
    fn size_band_selects_only_its_named_container_or_nothing_unscoped() {
        let tree = package(vec![
            node(Level::Folder, "hub", vec![file("hub/a.py")]),
            node(
                Level::Folder,
                "wide",
                vec![file("wide/b.py"), file("wide/c.py")],
            ),
        ]);
        let inputs = inputs_with(&tree);
        let named = SizeBand {
            container: Some("hub".to_owned()),
            scope: None,
            ..band(1, None)
        };
        let verdict = evaluate_size_band(&named, FaceMode::Anchored, &inputs);
        assert_eq!(verdict.label, "size_band(container(hub),max=1)#Anchored");
        assert!(verdict.passed, "the over-band `wide` is not selected");

        let unscoped = SizeBand {
            scope: None,
            ..band(1, None)
        };
        let unscoped_verdict = evaluate_size_band(&unscoped, FaceMode::Anchored, &inputs);
        assert_eq!(unscoped_verdict.label, "size_band(unscoped,max=1)#Anchored");
        assert!(unscoped_verdict.passed, "an unscoped band selects nothing");
    }

    #[test]
    fn name_alignment_reports_the_least_aligned_qualifying_container() {
        // helpers aligns 0/2 and billing 2/5: both sit below 0.5, and the
        // later, better-aligned billing never displaces helpers as the worst.
        let tree = package(vec![
            node(
                Level::Domain,
                "helpers",
                vec![file("helpers/charge.py"), file("helpers/refund.py")],
            ),
            node(
                Level::Folder,
                "billing",
                vec![
                    file("billing/billing_core.py"),
                    file("billing/billing_api.py"),
                    file("billing/ledger.py"),
                    file("billing/tax.py"),
                    file("billing/audit.py"),
                ],
            ),
            node(Level::Folder, "solo", vec![file("solo/other.py")]),
        ]);
        let inputs = inputs_with(&tree);

        let verdict = evaluate_name_alignment(&alignment(0.5), FaceMode::Anchored, &inputs);
        assert_eq!(
            verdict.label,
            "name_alignment(min_ratio=0.5,min_members=2)#Anchored"
        );
        assert!(!verdict.passed);
        assert_eq!(
            verdict.detail,
            "naming incoherence: container \"helpers\" aligns 0/2 members below min_ratio 0.5"
        );

        let lenient = evaluate_name_alignment(&alignment(0.0), FaceMode::Anchored, &inputs);
        assert!(lenient.passed, "{}", lenient.detail);
    }
    #[test]
    fn name_alignment_ranks_offenders_by_their_aligned_share() {
        // wide aligns 2/10 and the later narrow 1/3: narrow aligns fewer files
        // but a larger share, so wide stays the worst.
        let wide_members = ["wide_a", "wide_b", "c", "d", "e", "f", "g", "h", "i", "j"]
            .iter()
            .map(|stem| file(&format!("wide/{stem}.py")))
            .collect();
        let tree = package(vec![
            node(Level::Folder, "wide", wide_members),
            node(
                Level::Folder,
                "narrow",
                vec![
                    file("narrow/narrow_a.py"),
                    file("narrow/b.py"),
                    file("narrow/c.py"),
                ],
            ),
        ]);
        let inputs = inputs_with(&tree);

        let verdict = evaluate_name_alignment(&alignment(0.5), FaceMode::Anchored, &inputs);

        assert!(!verdict.passed);
        assert_eq!(
            verdict.detail,
            "naming incoherence: container \"wide\" aligns 2/10 members below min_ratio 0.5"
        );
    }
}
