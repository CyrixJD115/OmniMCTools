//! The conversion loss ledger (PLAN §5C).
//!
//! Findings Task A (data-loss ledger rows): every lossy mapping is
//! *returned* to the caller — converters never silently drop data. A
//! [`ConversionReport`] is cheap to accumulate and human-printable; the M3
//! pipeline will aggregate these per world into the conversion summary.

use std::fmt;

/// One recorded data loss or coercion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loss {
    pub kind: LossKind,
    /// Dotted path of the affected field inside the converted tree
    /// (e.g. `components."minecraft:custom_name"` or `WasPickedUp`).
    pub path: String,
    /// Short human explanation; safe to surface in CLI/GUI output.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LossKind {
    /// A field with no equivalent on the target edition was dropped
    /// (findings row 1: Java component without a Bedrock home; row 2:
    /// `WasPickedUp` / Bedrock-only fields going to Java).
    FieldDropped,
    /// The value survived but its type/width changed (e.g. Int → Byte
    /// count, Int → Short damage with clamping).
    ValueCoerced,
    /// A mixed-era stack (`tag` + `components` both present) had one side
    /// discarded per policy (findings row 3 / UMT `mixedComp` precedent).
    MixedEra,
    /// An id could not be mapped because the registry table that would
    /// map it does not exist yet (M3 `tools/registry-gen`); the entry was
    /// dropped rather than guessed.
    UnmappedId,
    /// A component/field whose mapping is not implemented yet; recorded so
    /// the M4 rule set can be extended without re-auditing call sites.
    UnsupportedComponent,
}

impl LossKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FieldDropped => "dropped",
            Self::ValueCoerced => "coerced",
            Self::MixedEra => "mixed-era",
            Self::UnmappedId => "unmapped-id",
            Self::UnsupportedComponent => "unsupported",
        }
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.kind.label(), self.path, self.detail)
    }
}

/// Losses recorded during one conversion (one item stack, one chunk, …).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionReport {
    pub losses: Vec<Loss>,
}

impl ConversionReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_clean(&self) -> bool {
        self.losses.is_empty()
    }

    pub fn record(&mut self, kind: LossKind, path: impl Into<String>, detail: impl Into<String>) {
        self.losses.push(Loss {
            kind,
            path: path.into(),
            detail: detail.into(),
        });
    }

    /// Convenience: `FieldDropped` row.
    pub fn dropped(&mut self, path: impl Into<String>, detail: impl Into<String>) {
        self.record(LossKind::FieldDropped, path, detail);
    }

    pub fn count_with_kind(&self, kind: LossKind) -> usize {
        self.losses.iter().filter(|l| l.kind == kind).count()
    }

    /// Merge another report into this one (chunk-level aggregation).
    pub fn extend(&mut self, other: &ConversionReport) {
        self.losses.extend(other.losses.iter().cloned());
    }
}

impl fmt::Display for ConversionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.losses.is_empty() {
            return write!(f, "no data loss");
        }
        write!(f, "{} loss(es):", self.losses.len())?;
        for loss in &self.losses {
            write!(f, "\n  {loss}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_starts_clean_and_displays() {
        let mut report = ConversionReport::new();
        assert!(report.is_clean());
        report.dropped("WasPickedUp", "Bedrock-only field has no Java home");
        report.record(
            LossKind::ValueCoerced,
            "Count",
            "Int 300 clamped to Byte 127",
        );
        assert_eq!(report.count_with_kind(LossKind::FieldDropped), 1);
        assert_eq!(report.count_with_kind(LossKind::ValueCoerced), 1);
        let text = report.to_string();
        assert!(text.contains("[dropped] WasPickedUp"));
        assert!(text.contains("[coerced] Count"));
        assert!(text.starts_with("2 loss(es):"));
    }

    #[test]
    fn reports_merge() {
        let mut a = ConversionReport::new();
        a.dropped("x", "a");
        let mut b = ConversionReport::new();
        b.record(LossKind::UnmappedId, "ench[0]", "no registry");
        a.extend(&b);
        assert_eq!(a.losses.len(), 2);
    }
}
