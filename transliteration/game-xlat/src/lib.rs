//! Strict, AST-audited translation of the recovered game application.
//!
//! Keep game methods here and reusable JVM/MIDP behavior in the neutral crates.
//! This crate intentionally uses `std`: fidelity, explicit state, and exact AST
//! ownership matter here; `no_std` is reserved for serialization codecs.

/// Marker proving the generated translation crate is wired into the workspace.
/// Replace this scaffold with the first bytecode/AST/oracle-backed method.
pub fn transliteration_started() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_does_not_claim_game_coverage() {
        assert!(!transliteration_started());
    }
}
