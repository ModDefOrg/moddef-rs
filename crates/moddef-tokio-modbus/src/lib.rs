//! tokio-modbus adapter: implements [`moddef_core::Transport`] over a
//! `tokio_modbus::client::Context` (spec §32.3).
//!
//! - Constructors: [`TokioModbusTransport::tcp`] (feature `tcp`, default),
//!   [`TokioModbusTransport::rtu`] (feature `rtu`, via tokio-serial), and
//!   [`TokioModbusTransport::wrap`] for any pre-built context.
//! - Reads are chunked to honor [`Options::max_read_words`] (Modbus caps a
//!   read at 125 registers; devices like the SDM630/EM24 need less — the
//!   same knob as the TS adapter).
//! - Every request runs under [`Options::timeout`] (`tokio::time::timeout`).
//! - Modbus exceptions surface as [`TokioModbusError::Exception`] with the
//!   device's exception code.
//!
//! `&mut self` on the Transport trait means no request queue is needed;
//! concurrent access is a caller decision (`Mutex<Device<...>>` or an actor
//! task).

use core::future::Future;
use core::time::Duration;
use std::fmt;
use std::io;

use moddef_core::Transport;
use tokio_modbus::client::{Context, Reader, Writer};
use tokio_modbus::{ExceptionCode, Slave};

/// Connection options shared by all constructors.
#[derive(Clone, Debug)]
pub struct Options {
    /// Modbus unit / slave id (default 1).
    pub unit_id: u8,
    /// Per-request timeout (default 5 s; `None` waits forever).
    pub timeout: Option<Duration>,
    /// Largest register window per read request (default and cap: 125).
    pub max_read_words: u16,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            unit_id: 1,
            timeout: Some(Duration::from_secs(5)),
            max_read_words: 125,
        }
    }
}

#[derive(Debug)]
pub enum TokioModbusError {
    /// The device answered a Modbus exception (illegal address, busy, …).
    Exception(ExceptionCode),
    /// Protocol or I/O failure below the Modbus layer.
    Transport(tokio_modbus::Error),
    /// The device answered fewer words than requested.
    ShortResponse,
    /// [`Options::timeout`] elapsed before the device answered.
    Timeout,
}

impl fmt::Display for TokioModbusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokioModbusError::Exception(e) => write!(f, "modbus exception: {e}"),
            TokioModbusError::Transport(e) => write!(f, "modbus transport: {e}"),
            TokioModbusError::ShortResponse => write!(f, "short modbus response"),
            TokioModbusError::Timeout => write!(f, "modbus request timed out"),
        }
    }
}

impl std::error::Error for TokioModbusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TokioModbusError::Transport(e) => Some(e),
            _ => None,
        }
    }
}

/// [`Transport`] over a tokio-modbus client context.
pub struct TokioModbusTransport {
    ctx: Context,
    timeout: Option<Duration>,
    max_read_words: u16,
}

impl TokioModbusTransport {
    /// Connect over Modbus TCP.
    #[cfg(feature = "tcp")]
    pub async fn tcp(addr: std::net::SocketAddr, opts: Options) -> io::Result<Self> {
        let ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(opts.unit_id)).await?;
        Ok(Self::wrap(ctx, opts))
    }

    /// Connect over Modbus RTU on a serial port.
    #[cfg(feature = "rtu")]
    pub async fn rtu(path: &str, baud_rate: u32, opts: Options) -> io::Result<Self> {
        let builder = tokio_serial::new(path, baud_rate);
        let stream = tokio_serial::SerialStream::open(&builder).map_err(io::Error::other)?;
        let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(opts.unit_id));
        Ok(Self::wrap(ctx, opts))
    }

    /// Wrap a pre-built context (custom framing, ASCII, tests…).
    pub fn wrap(ctx: Context, opts: Options) -> Self {
        TokioModbusTransport {
            ctx,
            timeout: opts.timeout,
            max_read_words: opts.max_read_words.clamp(1, 125),
        }
    }

    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }

    pub fn into_context(self) -> Context {
        self.ctx
    }
}

/// Run one Modbus call under the configured timeout and flatten the nested
/// `Result<Result<T, ExceptionCode>, Error>`.
async fn call<T>(
    timeout: Option<Duration>,
    fut: impl Future<Output = tokio_modbus::Result<T>>,
) -> Result<T, TokioModbusError> {
    let res = match timeout {
        Some(d) => tokio::time::timeout(d, fut)
            .await
            .map_err(|_| TokioModbusError::Timeout)?,
        None => fut.await,
    };
    match res {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(exception)) => Err(TokioModbusError::Exception(exception)),
        Err(e) => Err(TokioModbusError::Transport(e)),
    }
}

impl Transport for TokioModbusTransport {
    type Error = TokioModbusError;

    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error> {
        let mut off = offset;
        for chunk in out.chunks_mut(self.max_read_words as usize) {
            let words = call(
                self.timeout,
                self.ctx.read_holding_registers(off, chunk.len() as u16),
            )
            .await?;
            if words.len() < chunk.len() {
                return Err(TokioModbusError::ShortResponse);
            }
            chunk.copy_from_slice(&words[..chunk.len()]);
            off = off.wrapping_add(chunk.len() as u16);
        }
        Ok(())
    }

    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error> {
        let mut off = offset;
        for chunk in out.chunks_mut(self.max_read_words as usize) {
            let words = call(
                self.timeout,
                self.ctx.read_input_registers(off, chunk.len() as u16),
            )
            .await?;
            if words.len() < chunk.len() {
                return Err(TokioModbusError::ShortResponse);
            }
            chunk.copy_from_slice(&words[..chunk.len()]);
            off = off.wrapping_add(chunk.len() as u16);
        }
        Ok(())
    }

    async fn read_coils(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error> {
        let bits = call(self.timeout, self.ctx.read_coils(offset, out.len() as u16)).await?;
        if bits.len() < out.len() {
            return Err(TokioModbusError::ShortResponse);
        }
        out.copy_from_slice(&bits[..out.len()]);
        Ok(())
    }

    async fn read_discrete(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error> {
        let bits = call(
            self.timeout,
            self.ctx.read_discrete_inputs(offset, out.len() as u16),
        )
        .await?;
        if bits.len() < out.len() {
            return Err(TokioModbusError::ShortResponse);
        }
        out.copy_from_slice(&bits[..out.len()]);
        Ok(())
    }

    async fn write_holding(&mut self, offset: u16, regs: &[u16]) -> Result<(), Self::Error> {
        call(
            self.timeout,
            self.ctx.write_multiple_registers(offset, regs),
        )
        .await
    }

    async fn write_coil(&mut self, offset: u16, on: bool) -> Result<(), Self::Error> {
        call(self.timeout, self.ctx.write_single_coil(offset, on)).await
    }

    fn max_read_words(&self) -> u16 {
        self.max_read_words
    }
}
