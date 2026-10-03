//! `--help`, `--help-types`, `--man`, `--version` (Phase 4).

/// First line of `--version` output. `t/ack-version.t` only checks that the
/// ack version number appears on it.
pub fn version_text() -> String {
    format!(
        "ack {} (rask {})",
        crate::ACK_VERSION,
        env!("CARGO_PKG_VERSION")
    )
}
