//! Unit tests for mirror-template parsing, overlap detection, and validation.

use super::*;

fn mirror_template(template: &str) -> MirrorTemplate {
    let parsed = MirrorTemplate::parse(template);
    assert!(
        parsed.is_some(),
        "test mirror template must parse: {template}"
    );
    parsed.unwrap_or_default()
}

#[test]
fn should_detect_overlap_when_stem_captures_have_unequal_lengths() {
    let prefixed = mirror_template("root/{dir}/pre-{stem}.ts");
    let suffixed = mirror_template("root/{dir}/{stem}-post.ts");

    assert!(mirror_templates_overlap(&prefixed, &suffixed));
    assert!(mirror_templates_overlap(&suffixed, &prefixed));
}

#[test]
fn should_capture_zero_single_and_multiple_directory_segments() {
    let template = mirror_template("root/{dir}/item-{stem}.ts");

    for (path, expected_dir) in [
        ("root/item-alpha.ts", ""),
        ("root/one/item-alpha.ts", "one"),
        ("root/one/two/item-alpha.ts", "one/two"),
    ] {
        let captures = template.captures(path);
        assert_eq!(
            captures.as_ref().map(|value| value.dir.as_str()),
            Some(expected_dir)
        );
        assert_eq!(
            captures.as_ref().map(|value| value.stem.as_str()),
            Some("alpha")
        );
    }
}

#[test]
fn should_intersect_directory_languages_at_zero_single_and_multiple_segments() {
    for right in [
        "root/{dir}/{stem}.ts",
        "root/one/{dir}/{stem}.ts",
        "root/one/two/{dir}/{stem}.ts",
    ] {
        let left = mirror_template("root/{dir}/{stem}.ts");
        let right = mirror_template(right);
        assert!(mirror_templates_overlap(&left, &right));
        assert!(mirror_templates_overlap(&right, &left));
    }
}

#[test]
fn should_distinguish_intersecting_and_disjoint_stem_affixes() {
    let broad = mirror_template("root/{dir}/pre-{stem}-post.ts");
    let intersecting = mirror_template("root/{dir}/pre-x{stem}-post.ts");
    let disjoint = mirror_template("root/{dir}/other-{stem}-post.ts");

    assert!(mirror_templates_overlap(&broad, &intersecting));
    assert!(!mirror_templates_overlap(&broad, &disjoint));
}

#[test]
fn should_compute_template_overlap_symmetrically_and_deterministically() {
    let templates = [
        mirror_template("root/{dir}/{stem}.ts"),
        mirror_template("root/fixed/{dir}/pre-{stem}.ts"),
        mirror_template("other/{dir}/{stem}-post.ts"),
    ];

    for left in &templates {
        for right in &templates {
            let expected = mirror_templates_overlap(left, right);
            assert_eq!(mirror_templates_overlap(right, left), expected);
            assert_eq!(mirror_templates_overlap(left, right), expected);
        }
    }
}

#[test]
fn should_reject_templates_that_break_the_segment_grammar() {
    for template in [
        "root/{dir}/x{dir}/{stem}.ts",
        "{dir}/{stem}/item-{stem}.ts",
        "root/{dir}/pre{x}-{stem}.ts",
        "root/{dir}/{stem}-{x}.ts",
        "root/{dir}/{x}/{stem}.ts",
    ] {
        assert!(
            MirrorTemplate::parse(template).is_none(),
            "out-of-grammar template parsed: {template}"
        );
    }
}

#[test]
fn should_require_literal_segments_after_the_directory_capture_to_match() {
    let template = mirror_template("root/{dir}/sub/{stem}.ts");

    let matched = template.captures("root/one/sub/alpha.ts");

    assert_eq!(
        matched
            .as_ref()
            .map(|value| (value.dir.as_str(), value.stem.as_str())),
        Some(("one", "alpha"))
    );
    assert!(template.captures("root/one/other/alpha.ts").is_none());
    assert!(template.captures("root/one/sub/alpha.js").is_none());
}

#[test]
fn should_compare_literal_segments_after_the_directory_capture_when_overlapping() {
    let nested = mirror_template("root/{dir}/sub/{stem}.ts");
    let other_nested = mirror_template("root/{dir}/other/{stem}.ts");
    let broad = mirror_template("root/{dir}/{stem}.ts");

    assert!(mirror_templates_overlap(&nested, &broad));
    assert!(!mirror_templates_overlap(&nested, &other_nested));
    assert!(!mirror_templates_overlap(&other_nested, &nested));
}
