//! Transport abstraction (spec §32.2). Implementations wrap a Modbus client
//! (see `moddef-tokio-modbus`) or an in-memory register map for tests.
//!
//! Design notes:
//! - Native async-fn-in-trait; no `async_trait` box. `no_std`-compatible.
//! - `&mut self` on every method: a Modbus connection is a serial
//!   request/response channel, so the borrow checker enforces one in-flight
//!   request per transport.
//! - Reads fill caller buffers (`&mut [u16]` / `&mut [bool]`) so the core
//!   never allocates; the requested count is `out.len()`.

/// Async register-level transport. `offset` is the zero-based data-model
/// offset within the given address space (spec §7.2); implementations apply
/// any unit/base-address mapping.
#[allow(async_fn_in_trait)]
pub trait Transport {
    type Error;

    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error>;

    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error>;

    async fn read_coils(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error>;

    async fn read_discrete(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error>;

    async fn write_holding(&mut self, offset: u16, regs: &[u16]) -> Result<(), Self::Error>;

    async fn write_coil(&mut self, offset: u16, on: bool) -> Result<(), Self::Error>;

    /// Largest register window one read may request (Modbus caps at 125;
    /// devices sometimes less). The facade chunks larger spans.
    fn max_read_words(&self) -> u16 {
        125
    }
}
