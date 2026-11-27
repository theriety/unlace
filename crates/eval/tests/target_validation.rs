//! Load-time target validation: every CONTRACT.md rule that needs no analyzed
//! result rejects a malformed target with a message naming the violated rule,
//! and accepts the well-formed shape of each assertion kind.

use std::error::Error;

use unlace_eval::target::Tier;
use unlace_eval::{EvalError, TargetSpec, load_target};

/// The case name every synthetic target below is validated under.
const PROBE: &str = "probe";

/// A valid header with a both-face run; each case appends its own blocks.
const BOTH_RUN: &str = r#"
schema = 1
fixture = "probe"

[meta]
failure_mode = "directory-tearing"
language = "python"
question = "Does the probe hold?"

[run]
mode = "both"
candidates = 3
seed = 42
config = "defaults"
"#;

/// The same header with an anchored-only run.
const ANCHORED_RUN: &str = r#"
schema = 1
fixture = "probe"

[meta]
failure_mode = "directory-tearing"
language = "python"
question = "Does the probe hold?"

[run]
mode = "anchored"
candidates = 3
seed = 42
config = "defaults"
"#;

/// A minimal valid assert block over the anchored face.
const ANCHORED_BLOCK: &str = r#"
[[assert]]
modes = ["anchored"]
"#;

type TestResult = Result<(), Box<dyn Error>>;

fn parse(document: &str) -> Result<TargetSpec, toml::de::Error> {
    toml::from_str(document)
}

/// Validates `document` under [`PROBE`] and returns the rejection message,
/// failing when the target is accepted or rejected with another error kind.
fn rejection(document: &str) -> Result<String, Box<dyn Error>> {
    let error = parse(document)?
        .validate(PROBE)
        .err()
        .ok_or("the target was accepted")?;
    assert!(
        matches!(&error, EvalError::TargetInvalid { target, .. } if target == PROBE),
        "a target defect names the invoked case: {error}"
    );
    Ok(error.to_string())
}

/// Asserts every `(document, fragment)` pair is rejected naming `fragment`.
fn assert_rejections(cases: &[(String, &str)]) -> TestResult {
    for (document, fragment) in cases {
        let message = rejection(document)?;
        assert!(
            message.contains(fragment),
            "expected a rejection naming {fragment:?}, got {message:?}"
        );
    }
    Ok(())
}

#[test]
fn header_shape_rejects_wrong_schema_fixture_and_block_count() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{}{ANCHORED_BLOCK}",
                BOTH_RUN.replace("schema = 1", "schema = 2")
            ),
            "schema 2 is not supported (this harness implements schema 1)",
        ),
        (
            format!(
                "{}{ANCHORED_BLOCK}",
                BOTH_RUN.replace(r#"fixture = "probe""#, r#"fixture = "renamed""#)
            ),
            r#"`fixture = "renamed"` does not match the invoked case name "probe""#,
        ),
        (
            BOTH_RUN.to_owned(),
            "exactly one [[assert]] block is required, found 0",
        ),
        (
            format!("{BOTH_RUN}{ANCHORED_BLOCK}{ANCHORED_BLOCK}"),
            "exactly one [[assert]] block is required, found 2",
        ),
    ])
}

#[test]
fn assert_block_rejects_empty_modes_zero_candidate_and_unproduced_modes() -> TestResult {
    assert_rejections(&[
        (
            format!("{BOTH_RUN}\n[[assert]]\nmodes = []\n"),
            "[assert].modes is empty",
        ),
        (
            format!("{BOTH_RUN}{ANCHORED_BLOCK}candidate = 0\n"),
            "[assert].candidate is 1-based",
        ),
        (
            format!("{ANCHORED_RUN}\n[[assert]]\nmodes = [\"greenfield\"]\n"),
            "[assert].modes lists Greenfield but [run].mode is Anchored",
        ),
    ])
}

#[test]
fn omitted_candidate_defaults_to_the_best_state() -> TestResult {
    let spec = parse(&format!("{BOTH_RUN}{ANCHORED_BLOCK}"))?;
    spec.validate(PROBE)?;
    assert_eq!(
        spec.assert.first().map(|block| block.candidate),
        Some(1),
        "an omitted candidate reads the best returned alternative"
    );
    Ok(())
}

#[test]
fn preconditions_need_a_rationale_and_their_kind_keys() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}\n[[precondition]]\nkind = \"file_count\"\nmin = 1\nmax = 2\nbecause = \"  \"\n{ANCHORED_BLOCK}"
            ),
            "precondition FileCount: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}\n[[precondition]]\nkind = \"file_count\"\nmin = 1\nbecause = \"bounded\"\n{ANCHORED_BLOCK}"
            ),
            "precondition FileCount: file_count requires both min and max",
        ),
        (
            format!(
                "{BOTH_RUN}\n[[precondition]]\nkind = \"violation_present\"\nbecause = \"a cycle exists\"\n{ANCHORED_BLOCK}"
            ),
            "precondition ViolationPresent: ViolationPresent requires a violation class",
        ),
        (
            format!(
                "{BOTH_RUN}\n[[precondition]]\nkind = \"violation_absent\"\nbecause = \"no capacity breach\"\n{ANCHORED_BLOCK}"
            ),
            "ViolationAbsent requires a violation class",
        ),
    ])?;

    let accepted = parse(&format!(
        "{BOTH_RUN}\n[[precondition]]\nkind = \"violation_absent\"\nviolation = \"test-only\"\nbecause = \"no test-only leak\"\n{ANCHORED_BLOCK}"
    ))?;
    accepted.validate(PROBE)?;
    Ok(())
}

#[test]
fn reference_containers_must_label_non_empty_file_paths() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[reference.container]]\npath = \"billing\"\nfiles = []\n"
            ),
            r#"reference container "billing" lists no files"#,
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[reference.container]]\npath = \"billing\"\nfiles = [\"billing/a.py\", \"\"]\n"
            ),
            r#"reference container "billing" carries an empty file path"#,
        ),
    ])?;

    let accepted = parse(&format!(
        "{BOTH_RUN}{ANCHORED_BLOCK}\n[[reference.container]]\npath = \"billing\"\nfiles = [\"billing/a.py\"]\n"
    ))?;
    accepted.validate(PROBE)?;
    Ok(())
}

#[test]
fn path_set_and_preserve_dir_assertions_enforce_their_rules() -> TestResult {
    let blank_because = "because = \" \"";
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.preserve_dir]]\npath = \"billing\"\n{blank_because}\n"
            ),
            "preserve_dir: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.separate]]\npaths = [\"a.py\", \"b.py\"]\n{blank_because}\n"
            ),
            "path-set: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.keep_together]]\npaths = [\"a.py\"]\nbecause = \"one file\"\n"
            ),
            "keep_together/separate needs at least two paths to relate",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.keep_together]]\npaths = [\"a.py\", \"\"]\nbecause = \"blank path\"\n"
            ),
            "keep_together/separate paths must be non-empty",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.separate]]\npaths = [\"a.py\", \"b.py\"]\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "assertion overrides its mode to Greenfield but [run].mode is Anchored",
        ),
    ])
}

#[test]
fn size_band_and_move_budget_need_exactly_one_selector_or_bound() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.size_band]]\nmax_files = 3\nscope = \"any_container\"\nbecause = \"\"\n"
            ),
            "size_band: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.size_band]]\nmax_files = 3\nscope = \"any_container\"\ncontainer = \"hub\"\nbecause = \"both selectors\"\n"
            ),
            "size_band needs exactly one of scope/container",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.size_band]]\nmax_files = 3\nbecause = \"no selector\"\n"
            ),
            "size_band needs exactly one of scope/container",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.size_band]]\nmax_files = 3\ncontainer = \"hub\"\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "assertion overrides its mode to Greenfield",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.move_budget]]\nmode = \"anchored\"\nmax_moved_files = 0\nbecause = \"\"\n"
            ),
            "move_budget: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.move_budget]]\nmode = \"anchored\"\nmax_moved_files = 2\nmin_moved_files = 1\nbecause = \"both bounds\"\n"
            ),
            "move_budget needs exactly one of max_moved_files/min_moved_files",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.move_budget]]\nmode = \"anchored\"\nbecause = \"no bound\"\n"
            ),
            "move_budget needs exactly one of max_moved_files/min_moved_files",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.move_budget]]\nmode = \"greenfield\"\nmax_moved_files = 0\nbecause = \"unproduced face\"\n"
            ),
            "move_budget targets Greenfield but [run].mode is Anchored",
        ),
    ])
}

#[test]
fn bucket_and_alignment_assertions_enforce_names_ratios_and_modes() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.no_synthetic_bucket]]\nname = \"workspace\"\nbecause = \"\"\n"
            ),
            "no_synthetic_bucket: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.no_synthetic_bucket]]\nname = \"  \"\nbecause = \"blank name\"\n"
            ),
            "no_synthetic_bucket name must be non-empty",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.no_synthetic_bucket]]\nname = \"workspace\"\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "assertion overrides its mode to Greenfield",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.name_alignment]]\nmin_ratio = 0.5\nmin_members = 2\nbecause = \"\"\n"
            ),
            "name_alignment: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.name_alignment]]\nmin_ratio = 1.5\nmin_members = 2\nbecause = \"ratio above one\"\n"
            ),
            "name_alignment min_ratio must lie within [0, 1]",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.name_alignment]]\nmin_ratio = 0.5\nmin_members = 2\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "assertion overrides its mode to Greenfield",
        ),
    ])
}

#[test]
fn relief_inversion_and_symbol_home_assertions_enforce_their_rules() -> TestResult {
    assert_rejections(&[
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.capacity_relief]]\nmode = \"anchored\"\nbecause = \"\"\n"
            ),
            "capacity_relief: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.capacity_relief]]\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "capacity_relief targets Greenfield but [run].mode is Anchored",
        ),
        (
            format!("{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.non_inversion]]\nbecause = \"\"\n"),
            "non_inversion: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.non_inversion]]\nbecause = \"one face only\"\n"
            ),
            "non_inversion compares both faces and requires [run].mode = both",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.preserve_symbol_home]]\nsymbol = \"Shape\"\npath = \"shape.py\"\nbecause = \"\"\n"
            ),
            "preserve_symbol_home: every block carries a non-empty `because rationale",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.preserve_symbol_home]]\nsymbol = \" \"\npath = \"shape.py\"\nbecause = \"blank symbol\"\n"
            ),
            "preserve_symbol_home needs a non-empty symbol and path",
        ),
        (
            format!(
                "{BOTH_RUN}{ANCHORED_BLOCK}\n[[assert.preserve_symbol_home]]\nsymbol = \"Shape\"\npath = \"\"\nbecause = \"blank path\"\n"
            ),
            "preserve_symbol_home needs a non-empty symbol and path",
        ),
        (
            format!(
                "{ANCHORED_RUN}{ANCHORED_BLOCK}\n[[assert.preserve_symbol_home]]\nsymbol = \"Shape\"\npath = \"shape.py\"\nmode = \"greenfield\"\nbecause = \"unproduced face\"\n"
            ),
            "assertion overrides its mode to Greenfield",
        ),
    ])
}

#[test]
fn every_assertion_kind_validates_in_its_well_formed_shape() -> TestResult {
    let spec = parse(&format!(
        r#"{BOTH_RUN}
[[assert]]
modes = ["anchored", "greenfield"]

[[assert.preserve_dir]]
path = "billing"
because = "billing survives"

[[assert.keep_together]]
paths = ["billing/a.py", "billing/b.py"]
mode = "anchored"
because = "billing stays whole"

[[assert.separate]]
paths = ["billing/a.py", "metrics/c.py"]
because = "unrelated features"

[[assert.size_band]]
max_files = 4
min_files = 1
scope = "any_container"
mode = "greenfield"
because = "small places"

[[assert.move_budget]]
mode = "greenfield"
min_moved_files = 1
because = "greenfield must move something"

[[assert.no_synthetic_bucket]]
name = "workspace"
mode = "anchored"
because = "no synthetic bucket"

[[assert.name_alignment]]
min_ratio = 1.0
min_members = 2
mode = "anchored"
because = "names align"

[[assert.capacity_relief]]
mode = "anchored"
because = "relieved"

[[assert.non_inversion]]
because = "anchored never out-churns greenfield"

[[assert.preserve_symbol_home]]
symbol = "Invoice"
path = "billing/a.py"
mode = "greenfield"
because = "Invoice stays home"
"#
    ))?;
    spec.validate(PROBE)?;
    Ok(())
}

#[test]
fn advice_tier_parses_min_tier_and_rejects_a_vacuous_or_doubled_floor() -> TestResult {
    let pin = |keys: &str| {
        format!(
            r#"{BOTH_RUN}
[[assert]]
modes = ["anchored", "greenfield"]

[[assert.advice_tier]]
symbol = "formatRow"
path = "src/report/format.ts"
{keys}
because = "the move survives into the advice"
"#
        )
    };
    let spec = parse(&pin(r#"min_tier = "review""#))?;
    let floors: Vec<_> = spec
        .assert
        .iter()
        .flat_map(|block| &block.advice_tier)
        .map(|advice| (advice.min_tier, advice.max_tier, advice.exact_tier))
        .collect();
    assert_eq!(floors, vec![(Some(Tier::Review), None, None)]);
    spec.validate(PROBE)?;

    assert!(
        parse(&pin(r#"min_tier = "best""#)).is_err(),
        "an unknown tier name fails to parse"
    );
    assert_rejections(&[
        (pin(r#"min_tier = "absent""#), "floors nothing"),
        (
            pin("min_tier = \"review\"\nmax_tier = \"review\""),
            "exactly one of max_tier/min_tier/exact_tier",
        ),
    ])
}

#[test]
fn load_target_reports_unreadable_and_unparsable_targets() -> TestResult {
    let missing = load_target("no-such-target")
        .err()
        .ok_or("a missing target loaded")?;
    assert!(
        matches!(&missing, EvalError::TargetInvalid { target, message }
            if target == "no-such-target" && message.starts_with("reading ")),
        "a missing file is a read failure: {missing}"
    );

    // The malformed target lives outside `targets/` so the committed-target
    // inventory stays exact; the name resolves relative to that directory.
    let malformed_name = "../tests/fixtures/unknown-key-target";
    let malformed = load_target(malformed_name)
        .err()
        .ok_or("a malformed target parsed")?;
    assert!(
        matches!(&malformed, EvalError::TargetInvalid { target, message }
            if target == malformed_name
                && message.starts_with("parsing ")
                && message.contains("unknown field `wobble`")),
        "a parse failure carries the TOML error verbatim: {malformed}"
    );
    Ok(())
}

#[test]
fn advice_pins_and_the_recommended_guard_need_a_rationale() -> TestResult {
    let unexplained_pin = format!(
        r#"{BOTH_RUN}
[[assert]]
modes = ["anchored", "greenfield"]

[[assert.advice_tier]]
path = "src/report/format.ts"
max_tier = "review"
because = "  "
"#
    );
    let unexplained_guard = format!(
        r#"{BOTH_RUN}
[[assert]]
modes = ["anchored", "greenfield"]

[[assert.keep_together]]
paths = ["src/report/format.ts", "src/report/table.ts"]
because = "the report helpers stay together"

[[assert.recommended_guard]]
because = "  "
"#
    );

    assert_rejections(&[
        (unexplained_pin, "advice_tier: every block carries"),
        (unexplained_guard, "recommended_guard: every block carries"),
    ])
}

#[test]
fn cycle_import_cut_needs_a_rationale_two_files_and_a_well_formed_expect() -> TestResult {
    let pin = |keys: &str| {
        format!(
            r#"{BOTH_RUN}
[[assert]]
modes = ["anchored"]

[[assert.cycle_import_cut]]
{keys}
"#
        )
    };
    let files = r#"files = ["src/parser/adapters.ts", "src/parser/build.ts"]"#;
    let because = r#"because = "C1: the cycle names an import cut""#;
    let expect = |importer: &str, imported: &str, line: u32| {
        format!(
            "{files}\nexpect = {{ importer = \"{importer}\", imported = \"{imported}\", line = {line} }}\n{because}"
        )
    };
    let expect_rule =
        "cycle_import_cut.expect needs two different non-empty files and a 1-based line";

    parse(&pin(&format!("{files}\n{because}")))?.validate(PROBE)?;
    parse(&pin(&expect(
        "src/parser/adapters.ts",
        "src/parser/build.ts",
        7,
    )))?
    .validate(PROBE)?;
    assert_rejections(&[
        (
            pin(&format!("{files}\nbecause = \"  \"")),
            "cycle_import_cut: every block carries",
        ),
        (
            pin(&format!("files = [\"src/parser/adapters.ts\"]\n{because}")),
            "cycle_import_cut needs at least two non-empty file paths",
        ),
        (
            pin(&format!(
                "files = [\"src/parser/adapters.ts\", \" \"]\n{because}"
            )),
            "cycle_import_cut needs at least two non-empty file paths",
        ),
        (
            pin(&expect(
                "src/parser/adapters.ts",
                "src/parser/adapters.ts",
                7,
            )),
            expect_rule,
        ),
        (pin(&expect(" ", "src/parser/build.ts", 7)), expect_rule),
        (
            pin(&expect("src/parser/adapters.ts", "src/parser/build.ts", 0)),
            expect_rule,
        ),
    ])
}
