//! `google.type.Money` as integer nanos.
//!
//! The API encodes `units` as a JSON *string* holding an int64 and `nanos` as
//! an int32, with both fields carrying the same sign (-$1.75 is units=-1,
//! nanos=-750000000). Reconstructing that through `f64` loses cents once
//! totals get large, so the value is held as i128 nanos from parse to render
//! and rounded exactly once, at render.

use serde::{Deserialize, Serialize};

const NANOS_PER_UNIT: i128 = 1_000_000_000;
const NANOS_PER_HUNDREDTH: i128 = 10_000_000;

/// Seconds in the 30-day month the cards normalise to. Matches the
/// `2592000s` duration the cost recommenders commonly return, but the
/// normalisation divides by the projection's own duration rather than
/// assuming this one.
pub const MONTH_SECONDS: i128 = 2_592_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("units field is not an int64: {0}")]
    Units(String),
}

/// An amount in one currency, held as integer nanos so nothing rounds before
/// [`Money::render`]. `currency` is the API's own code and is never converted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub currency: String,
    pub nanos: i128,
}

impl Money {
    pub fn parse(units: &str, nanos: i32, currency: &str) -> Result<Self, MoneyError> {
        let units: i64 = units
            .parse()
            .map_err(|_| MoneyError::Units(units.to_string()))?;
        Ok(Self {
            currency: currency.to_string(),
            nanos: i128::from(units) * NANOS_PER_UNIT + i128::from(nanos),
        })
    }

    /// Flips the sign. The Recommender API states a saving as a negative cost,
    /// so this is what turns a reported cost delta into a positive saving.
    pub fn negate(self) -> Self {
        Self {
            currency: self.currency,
            nanos: -self.nanos,
        }
    }

    /// `self * numerator / denominator`, refusing a zero denominator rather
    /// than dividing by it. Used to normalise a projection stated over its own
    /// duration into a monthly figure.
    pub fn scaled(self, numerator: i128, denominator: i128) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        Some(Self {
            nanos: self.nanos * numerator / denominator,
            currency: self.currency,
        })
    }

    /// `"USD 1.20"`. The only place rounding happens.
    ///
    /// Half away from zero, rather than the truncation an integer division
    /// gives for free: truncating is not a neutral choice on a savings card,
    /// because every row would round toward zero and the total would
    /// understate the estimate by up to a cent per row, always in the same
    /// direction.
    pub fn render(&self) -> String {
        // signum rather than a comparison: the half-cent leans the way the
        // amount already does, and at exactly zero it leans nowhere, so there
        // is no boundary for a branch to get wrong.
        let half = self.nanos.signum() * (NANOS_PER_HUNDREDTH / 2);
        let hundredths = (self.nanos + half) / NANOS_PER_HUNDREDTH;
        let sign = if hundredths < 0 { "-" } else { "" };
        let magnitude = hundredths.abs();
        format!(
            "{} {sign}{}.{:02}",
            self.currency,
            magnitude / 100,
            magnitude % 100
        )
    }
}

/// Per-currency accumulator. Adding two currencies yields two subtotals, never
/// one converted figure: the app has no exchange rate, and inventing one turns
/// a reported number into a computed claim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoneyTotal {
    parts: Vec<Money>,
}

impl MoneyTotal {
    /// Adds into the subtotal for this money's currency, starting a new
    /// subtotal if the total has not seen that currency before.
    pub fn add(&mut self, money: Money) {
        match self
            .parts
            .iter_mut()
            .find(|part| part.currency == money.currency)
        {
            Some(part) => part.nanos += money.nanos,
            None => self.parts.push(money),
        }
    }

    /// The subtotals, one per currency, in the order each currency first
    /// appeared.
    pub fn into_parts(self) -> Vec<Money> {
        self.parts
    }

    /// Whether anything has been added at all. Not the same as summing to
    /// zero, which still has a currency to name.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}
