// SPDX-License-Identifier: Apache-2.0

//! Shared in-memory transport for facade and generated-client tests.

use moddef_core::Transport;

/// In-memory register map; reads beyond the configured size fail like a
/// device answering a Modbus exception (used to skip discovery anchors).
pub struct MockTransport {
    pub holding: Vec<u16>,
    pub input: Vec<u16>,
    pub coils: Vec<bool>,
    pub discrete: Vec<bool>,
    pub read_log: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct OutOfRange;

impl MockTransport {
    pub fn new(size: usize) -> Self {
        MockTransport {
            holding: vec![0; size],
            input: vec![0; size],
            coils: vec![false; size],
            discrete: vec![false; size],
            read_log: Vec::new(),
        }
    }
}

fn copy_range<T: Copy>(src: &[T], offset: u16, out: &mut [T]) -> Result<(), OutOfRange> {
    let s = offset as usize;
    let e = s.checked_add(out.len()).ok_or(OutOfRange)?;
    if e > src.len() {
        return Err(OutOfRange);
    }
    out.copy_from_slice(&src[s..e]);
    Ok(())
}

impl Transport for MockTransport {
    type Error = OutOfRange;

    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        self.read_log.push(format!("H@{offset}x{}", out.len()));
        copy_range(&self.holding, offset, out)
    }

    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        self.read_log.push(format!("I@{offset}x{}", out.len()));
        copy_range(&self.input, offset, out)
    }

    async fn read_coils(&mut self, offset: u16, out: &mut [bool]) -> Result<(), OutOfRange> {
        copy_range(&self.coils, offset, out)
    }

    async fn read_discrete(&mut self, offset: u16, out: &mut [bool]) -> Result<(), OutOfRange> {
        copy_range(&self.discrete, offset, out)
    }

    async fn write_holding(&mut self, offset: u16, regs: &[u16]) -> Result<(), OutOfRange> {
        let s = offset as usize;
        if s + regs.len() > self.holding.len() {
            return Err(OutOfRange);
        }
        self.holding[s..s + regs.len()].copy_from_slice(regs);
        Ok(())
    }

    async fn write_coil(&mut self, offset: u16, on: bool) -> Result<(), OutOfRange> {
        let s = offset as usize;
        if s >= self.coils.len() {
            return Err(OutOfRange);
        }
        self.coils[s] = on;
        Ok(())
    }
}
