//! Name-based resolution fallback checks.
//!
//! When the semantic database cannot pin a reference, these checks decide
//! whether a uniquely-named in-workspace declaration may stand in for it, so the
//! fallback never captures receiver calls, foreign types, or values-as-names.

use std::path::Path;

use ra_ap_ide::TextSize;
use ra_ap_syntax::{AstNode as _, T, algo::previous_non_trivia_token, ast};

use super::assignment::NodeAssignment;
use super::database::{Database, ResolvedTarget};
use crate::parse::{RefKind, Reference};

impl Database {
    /// Allows fallback only for a verified nonreceiver reference token. Token
    /// inspection also covers method names inside unexpanded macro arguments.
    pub(super) fn allows_name_fallback(
        &self,
        path: &str,
        reference: &Reference,
        workspace_root: &Path,
    ) -> bool {
        let offset = TextSize::new(reference.offset);
        let Some(token) = self.token_at(path, offset, workspace_root) else {
            return false;
        };
        let previous = previous_non_trivia_token(token.clone());
        if token.text_range().start() != offset
            || token.text() != reference.name.as_str()
            || previous.as_ref().is_some_and(|prior| prior.kind() == T![.])
        {
            return false;
        }
        // A bare call-kind name that is not a callee is a function used as a
        // value; only the semantic database may bind it, never its name.
        let bare = previous.is_none_or(|prior| prior.kind() != T![::]);
        !(reference.kind == RefKind::Call && bare && !is_followed_by_call_or_path(&token))
    }

    /// Whether the path a qualifier belongs to starts inside the workspace, so a
    /// name fallback on the qualifier cannot capture a foreign type: in
    /// `std::io::Error::new` the qualifier `Error` must not bind to a workspace
    /// `Error`. A qualifier that leads its path (`Type::new`) keeps the fallback;
    /// a longer path needs a `crate`/`self`/`super`/`Self` root, a root the
    /// semantic database places in the workspace, or, when the database cannot
    /// resolve it, a root named after a workspace module file.
    pub(super) fn roots_in_workspace(
        &self,
        path: &str,
        reference: &Reference,
        assignment: &NodeAssignment,
        workspace_root: &Path,
    ) -> bool {
        let Some(qualified) = self
            .token_at(path, TextSize::new(reference.offset), workspace_root)
            .and_then(|token| token.parent_ancestors().find_map(ast::Path::cast))
        else {
            return false;
        };
        if qualified.qualifier().is_none() {
            return true;
        }
        // a qualified path always has a first segment, its root.
        let root = qualified.first_segment();
        match root.as_ref().and_then(ast::PathSegment::kind) {
            Some(
                ast::PathSegmentKind::CrateKw
                | ast::PathSegmentKind::SelfKw
                | ast::PathSegmentKind::SuperKw
                | ast::PathSegmentKind::SelfTypeKw,
            ) => true,
            Some(ast::PathSegmentKind::Name(name))
                if root
                    .as_ref()
                    .is_some_and(|root| root.coloncolon_token().is_none()) =>
            {
                let start = u32::from(name.syntax().text_range().start());
                match self.resolve(path, start, workspace_root) {
                    Some(ResolvedTarget::Workspace { .. }) => true,
                    Some(ResolvedTarget::External) => false,
                    None => assignment.names_a_module(path, &name.text()),
                }
            }
            _ => false,
        }
    }

    /// The token starting at or after `offset` in the on-disk `path`, when the
    /// workspace holds the file and the offset lies inside it (the recorded
    /// source may be newer than the file on disk).
    fn token_at(
        &self,
        path: &str,
        offset: TextSize,
        workspace_root: &Path,
    ) -> Option<ra_ap_syntax::SyntaxToken> {
        // a parse is only cancelled by a concurrent change, which never comes.
        let parsed = self
            .file_id_for(path, workspace_root)
            .and_then(|file_id| self.host.analysis().parse(file_id).ok())?;
        let syntax = parsed.syntax();
        // a non-empty range holds a token at each of its offsets.
        syntax
            .text_range()
            .contains(offset)
            .then(|| syntax.token_at_offset(offset).right_biased())
            .flatten()
    }
}

/// Whether the next non-trivia token after `token` opens a call (`(`) or
/// continues a path (`::`), i.e. `token` is a callee or a leading path segment.
fn is_followed_by_call_or_path(token: &ra_ap_syntax::SyntaxToken) -> bool {
    let mut next = token.next_token();
    while let Some(current) = next {
        if !current.kind().is_trivia() {
            return matches!(current.kind(), T!['('] | T![::]);
        }
        next = current.next_token();
    }
    false
}
