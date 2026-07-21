// SPDX-License-Identifier: Apache-2.0

//! Command (multi-step register procedure) support, spec §11.7. The executor
//! itself is [`crate::device::Device::run_command`]; this module holds the
//! pure pieces — poll-condition evaluation, the single-PDU write cap, the
//! caller-facing param value type — and the [`Delay`] abstraction the poll
//! loop is parameterized over ([`Transport`](crate::transport::Transport)
//! has no time primitive, and none exists in the `alloc`-only tier).

use crate::schema;
use crate::value::Value;

/// Modbus single-PDU practical write cap (FC16); larger writes chunk inside
/// the executor. (Reads are chunked by transports per `max_read_words`.)
pub const MAX_WRITE_WORDS: usize = 123;

/// Default poll interval when a `PollStep` omits `interval_ms`.
pub const DEFAULT_POLL_INTERVAL_MS: u32 = 250;

/// Async sleep used by poll steps. `std`/tokio callers get an impl from
/// `moddef-tokio-modbus`; embedded callers supply their own (e.g. an
/// embassy timer). Elapsed poll time is accounted by accumulating the
/// requested delays, so no wall clock is required.
#[allow(async_fn_in_trait)]
pub trait Delay {
    async fn delay_ms(&mut self, ms: u32);
}

/// Caller-supplied value for a command param (numeric, string, or bytes —
/// matching the split encode paths of the codec).
#[derive(Clone, Copy, Debug)]
pub enum ParamValue<'a> {
    Value(Value),
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl From<i64> for ParamValue<'_> {
    fn from(v: i64) -> Self {
        ParamValue::Value(Value::I64(v))
    }
}
impl From<u64> for ParamValue<'_> {
    fn from(v: u64) -> Self {
        ParamValue::Value(Value::U64(v))
    }
}
impl From<f64> for ParamValue<'_> {
    fn from(v: f64) -> Self {
        ParamValue::Value(Value::F64(v))
    }
}
impl From<bool> for ParamValue<'_> {
    fn from(v: bool) -> Self {
        ParamValue::Value(Value::Bool(v))
    }
}
impl<'a> From<&'a str> for ParamValue<'a> {
    fn from(v: &'a str) -> Self {
        ParamValue::Str(v)
    }
}
impl<'a> From<&'a [u8]> for ParamValue<'a> {
    fn from(v: &'a [u8]) -> Self {
        ParamValue::Bytes(v)
    }
}

/// Evaluate a §11.7 poll exit condition against a raw integer.
pub fn condition_met(c: Option<&schema::Condition>, raw: i64) -> bool {
    let Some(c) = c else { return false };
    match c.op() {
        schema::ConditionOp::Eq => raw == c.value,
        schema::ConditionOp::Ne => raw != c.value,
        schema::ConditionOp::Mask => raw & c.mask == c.value,
        schema::ConditionOp::Range => c.min <= raw && raw <= c.max,
        schema::ConditionOp::Unspecified => false,
    }
}
