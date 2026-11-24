//! The opaque parse-tree payload the Python adapter threads from `parse` to
//! `bind`: a payload it did not write is a bind error naming the file.

use smol_str::SmolStr;
use unlace_adapter_python::PythonAdapter;
use unlace_ir::{Adapter, AdapterError, ParseTree};

#[test]
fn should_reject_a_payload_that_is_not_a_parse_summary() {
    let tree = ParseTree {
        path: SmolStr::new("pkg/mod.py"),
        payload: "not a summary".to_owned(),
    };

    let error = PythonAdapter::new("repo").bind(vec![tree]).err();

    assert!(
        matches!(
            &error,
            Some(AdapterError::Bind { path, reason })
                if path == "pkg/mod.py" && reason.starts_with("failed to read parse summary: ")
        ),
        "{error:?}"
    );
}
