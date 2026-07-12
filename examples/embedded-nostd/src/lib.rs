// SPDX-License-Identifier: Apache-2.0

//! Generated ModDef client on bare metal: no std, no alloc, no heap.
//!
//! The generated `GrowattSph<T>` drives the codec core over its static
//! descriptor table; all buffers are fixed-size stack arrays. Swap
//! [`LoopbackTransport`] for a real Modbus RTU transport built on your HAL's
//! UART (the `Transport` trait is `async`, so it slots into embassy
//! executors directly).

#![no_std]

#[allow(clippy::all, dead_code, unused_imports, unused_variables)]
pub mod growatt {
    include!(concat!(env!("OUT_DIR"), "/growatt_sph.rs"));
}

use growatt::{GrowattSph, InverterRunState};
use moddef_core::Transport;

/// Stand-in transport: serves reads/writes from a fixed register bank.
pub struct LoopbackTransport {
    pub input: [u16; 128],
    pub holding: [u16; 128],
}

pub struct OutOfRange;

impl Transport for LoopbackTransport {
    type Error = OutOfRange;

    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        let s = offset as usize;
        let bank = self.holding.get(s..s + out.len()).ok_or(OutOfRange)?;
        out.copy_from_slice(bank);
        Ok(())
    }

    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        let s = offset as usize;
        let bank = self.input.get(s..s + out.len()).ok_or(OutOfRange)?;
        out.copy_from_slice(bank);
        Ok(())
    }

    async fn read_coils(&mut self, _offset: u16, _out: &mut [bool]) -> Result<(), OutOfRange> {
        Err(OutOfRange)
    }

    async fn read_discrete(&mut self, _offset: u16, _out: &mut [bool]) -> Result<(), OutOfRange> {
        Err(OutOfRange)
    }

    async fn write_holding(&mut self, offset: u16, regs: &[u16]) -> Result<(), OutOfRange> {
        let s = offset as usize;
        let bank = self.holding.get_mut(s..s + regs.len()).ok_or(OutOfRange)?;
        bank.copy_from_slice(regs);
        Ok(())
    }

    async fn write_coil(&mut self, _offset: u16, _on: bool) -> Result<(), OutOfRange> {
        Err(OutOfRange)
    }
}

/// Poll the inverter once; returns (state, pv1 voltage in volts).
pub async fn poll(
    dev: &mut GrowattSph<LoopbackTransport>,
) -> Result<(InverterRunState, f64), moddef_core::Error<OutOfRange>> {
    let state = dev.inverter_status().await?;
    let pv1 = dev.pv1_voltage().await?;
    Ok((state, pv1))
}
