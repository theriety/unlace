//! Loading with no working directory. This binary holds a single test: it
//! removes its own working directory, which every thread of a process shares.

use std::fs;

use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, AdapterError};

#[test]
fn should_fail_to_load_when_the_working_directory_is_gone() -> Result<(), String> {
    let original = std::env::current_dir().map_err(|error| error.to_string())?;
    let gone = std::env::temp_dir().join(format!("unlace-no-cwd-{}", std::process::id()));
    fs::create_dir_all(&gone).map_err(|error| error.to_string())?;
    std::env::set_current_dir(&gone).map_err(|error| error.to_string())?;
    fs::remove_dir(&gone).map_err(|error| error.to_string())?;

    let adapter = RustAdapter::new("Cargo.toml");
    let result = adapter.bind(Vec::new());
    std::env::set_current_dir(&original).map_err(|error| error.to_string())?;

    assert!(
        matches!(&result, Err(AdapterError::Bind { reason, .. })
            if reason.starts_with("failed to load cargo workspace: ")),
        "{result:?}"
    );
    Ok(())
}
