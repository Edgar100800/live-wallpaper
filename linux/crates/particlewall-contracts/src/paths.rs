//! Path containment for the local-only navigation policy.
//!
//! Mirrors WebViewFactory.swift LocalOnlyNavigationDelegate, but fixes its
//! hasPrefix weakness (PRD G-05): containment is component-based, so
//! /library/foo does not match /library/foobar.

use std::path::{Component, Path};

/// True when `target` is `root` itself or a descendant of `root`.
pub fn is_within_root(root: &Path, target: &Path) -> bool {
    let root = normalize(root);
    let target = normalize(target);
    target.starts_with(&root)
}

/// Removes `.`, resolves `..` lexically, and drops root-relative components
/// that would escape an absolute base. Non-lexical symlink resolution is the
/// host's responsibility (it must canonicalize before calling this).
fn normalize(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop unless we are at the (absolute or current) root.
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_prefix_is_rejected() {
        assert!(!is_within_root(
            Path::new("/library/foo"),
            Path::new("/library/foobar/x.html")
        ));
    }

    #[test]
    fn parent_of_root_is_rejected() {
        assert!(!is_within_root(Path::new("/a/b"), Path::new("/a")));
    }

    #[test]
    fn traversal_resolves_before_check() {
        assert!(is_within_root(
            Path::new("/library/wp"),
            Path::new("/library/wp/../wp/index.html")
        ));
        assert!(!is_within_root(
            Path::new("/library/wp"),
            Path::new("/library/wp/../other/x.html")
        ));
    }
}
