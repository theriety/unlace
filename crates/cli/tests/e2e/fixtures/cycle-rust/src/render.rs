//! Renders a token, parsing quoted ones first.

use crate::parse::parse_token;

/// Renders `input`, parsing any quoted token.
pub fn render_token(input: &str) -> String {
    if let Some(inner) = input.strip_prefix('"') {
        return parse_token(inner);
    }
    format!("<{input}>")
}
