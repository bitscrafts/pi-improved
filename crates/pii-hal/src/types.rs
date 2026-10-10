//! The small value types of the HAL surface.

/// Which way a claimed pin is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Read-only.
    In,
    /// Driven by the HAL (and readable back).
    Out,
}

impl Direction {
    /// `in` / `out`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }

    /// Parses `in` / `out` (also `input` / `output`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "in" | "input" => Some(Self::In),
            "out" | "output" => Some(Self::Out),
            _ => None,
        }
    }
}

/// A digital line level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLevel {
    /// Logic low.
    Low,
    /// Logic high.
    High,
}

impl PinLevel {
    /// `low` / `high`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }

    /// Parses `low`/`high`, `0`/`1`, `false`/`true`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" | "0" | "false" => Some(Self::Low),
            "high" | "1" | "true" => Some(Self::High),
            _ => None,
        }
    }

    /// `true` for high.
    #[must_use]
    pub const fn is_high(self) -> bool {
        matches!(self, Self::High)
    }
}

impl From<bool> for PinLevel {
    fn from(high: bool) -> Self {
        if high { Self::High } else { Self::Low }
    }
}
