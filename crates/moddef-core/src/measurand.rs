// SPDX-License-Identifier: Apache-2.0

//! Measurand queries (spec §22, §26.1). A query selects points by their
//! semantic tuple; unspecified qualifiers are wildcards, base_quantity is
//! required. Mirrors go/client measurandMatches and moddef-ts measurand.ts.

use crate::schema;

/// Semantic point selector. Build with [`MeasurandQuery::base`] and narrow
/// with the qualifier builders.
#[derive(Clone, Copy, Debug)]
pub struct MeasurandQuery<'a> {
    pub base_quantity: &'a str,
    pub direction: Option<schema::Direction>,
    pub phase_ref: Option<schema::PhaseRef>,
    pub aggregation: Option<schema::Aggregation>,
    pub location: Option<schema::MeasurementLocation>,
    pub accumulation: Option<schema::Accumulation>,
}

impl<'a> MeasurandQuery<'a> {
    /// Query by base quantity only (all qualifiers wildcard).
    pub fn base(base_quantity: &'a str) -> Self {
        MeasurandQuery {
            base_quantity,
            direction: None,
            phase_ref: None,
            aggregation: None,
            location: None,
            accumulation: None,
        }
    }

    pub fn direction(mut self, d: schema::Direction) -> Self {
        self.direction = Some(d);
        self
    }

    pub fn phase(mut self, p: schema::PhaseRef) -> Self {
        self.phase_ref = Some(p);
        self
    }

    pub fn aggregation(mut self, a: schema::Aggregation) -> Self {
        self.aggregation = Some(a);
        self
    }

    pub fn location(mut self, l: schema::MeasurementLocation) -> Self {
        self.location = Some(l);
        self
    }

    pub fn accumulation(mut self, a: schema::Accumulation) -> Self {
        self.accumulation = Some(a);
        self
    }
}

/// Does the point's measurand tuple satisfy the query? `None` and
/// `*_UNSPECIFIED` qualifiers are wildcards.
pub fn measurand_matches(m: Option<&schema::MeasurandRef>, q: &MeasurandQuery<'_>) -> bool {
    let Some(m) = m else { return false };
    if m.base_quantity != q.base_quantity {
        return false;
    }
    if let Some(d) = q.direction {
        if d != schema::Direction::Unspecified && m.direction() != d {
            return false;
        }
    }
    if let Some(p) = q.phase_ref {
        if p != schema::PhaseRef::Unspecified && m.phase_ref() != p {
            return false;
        }
    }
    if let Some(a) = q.aggregation {
        if a != schema::Aggregation::Unspecified && m.aggregation() != a {
            return false;
        }
    }
    if let Some(l) = q.location {
        if l != schema::MeasurementLocation::LocationUnspecified && m.location() != l {
            return false;
        }
    }
    if let Some(a) = q.accumulation {
        if a != schema::Accumulation::Unspecified && m.accumulation() != a {
            return false;
        }
    }
    true
}
