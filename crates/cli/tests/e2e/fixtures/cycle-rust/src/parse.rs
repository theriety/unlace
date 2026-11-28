//! Parses a token, rendering nested ones back.

use crate::render::render_token;

/// Parses `input`, rendering any nested token.
pub fn parse_token(input: &str) -> String {
    if let Some(inner) = input.strip_prefix('(') {
        return render_token(inner);
    }
    input.to_owned()
}
