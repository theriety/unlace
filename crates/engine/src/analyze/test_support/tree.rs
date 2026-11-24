//! Hand-built result-tree nodes for tests: files, folders, and interior containers.

use crate::result::{ContainerNode, Level};

/// Builds a file node with `sloc` production SLOC.
pub(in crate::analyze) fn file(name: &str, sloc: u32) -> ContainerNode {
    ContainerNode {
        name: name.to_owned(),
        level: Level::File,
        children: None,
        symbols: Some(Vec::new()),
        production_sloc: Some(sloc),
    }
}

/// Builds a folder node holding `children`.
pub(in crate::analyze) fn folder(name: &str, children: Vec<ContainerNode>) -> ContainerNode {
    interior(name, Level::Folder, children)
}

/// Builds an interior node at `level` holding `children`.
pub(in crate::analyze) fn interior(
    name: &str,
    level: Level,
    children: Vec<ContainerNode>,
) -> ContainerNode {
    ContainerNode {
        name: name.to_owned(),
        level,
        children: Some(children),
        symbols: None,
        production_sloc: None,
    }
}

/// Returns the node's only child, or `None` when it has zero or several.
pub(in crate::analyze) fn only_child(node: ContainerNode) -> Option<ContainerNode> {
    node.children.and_then(|children| {
        if children.len() == 1 {
            children.into_iter().next()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{file, folder, only_child};

    #[test]
    fn should_return_the_child_only_when_there_is_exactly_one() {
        assert_eq!(
            only_child(folder("a", vec![file("a/x.ts", 1)])),
            Some(file("a/x.ts", 1))
        );
        assert_eq!(
            only_child(folder("a", vec![file("a/x.ts", 1), file("a/y.ts", 2)])),
            None
        );
        assert_eq!(only_child(file("x.ts", 1)), None);
    }
}
