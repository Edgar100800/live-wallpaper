//! URL scheme policy. Mirrors WebViewFactory.swift:23-35.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeDecision {
    /// Always allowed regardless of path.
    Allow,
    /// Allowed only if path containment passes.
    File,
    /// Always denied.
    Deny,
}

pub fn scheme_decision(scheme: &str) -> SchemeDecision {
    match scheme {
        "about" | "blob" | "data" => SchemeDecision::Allow,
        "file" => SchemeDecision::File,
        _ => SchemeDecision::Deny,
    }
}
