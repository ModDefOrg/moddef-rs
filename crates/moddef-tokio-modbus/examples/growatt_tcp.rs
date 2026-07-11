//! Read live values from a Growatt SPH (or any ModDef-described device) over
//! Modbus TCP using the runtime-parsed document — no codegen involved.
//!
//! ```sh
//! cargo run --example growatt_tcp -- 192.168.1.50:502 \
//!     ../../devices/solar-inverter/growatt-sph/growatt-sph.moddef.yaml \
//!     inverter_status pv1_voltage output_power
//! ```

use moddef_core::Device;
use moddef_tokio_modbus::{Options, TokioModbusTransport};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(addr), Some(profile)) = (args.next(), args.next()) else {
        eprintln!("usage: growatt_tcp <host:port> <profile.moddef.yaml> [point_id...]");
        std::process::exit(2);
    };
    let points: Vec<String> = args.collect();

    let doc = moddef_core::load(&profile)?;
    let transport = TokioModbusTransport::tcp(addr.parse()?, Options::default()).await?;
    let mut dev = Device::new(&doc, None, transport)?;

    let ids: Vec<String> = if points.is_empty() {
        dev.points().take(8).map(|p| p.point_id.clone()).collect()
    } else {
        points
    };

    for id in &ids {
        match dev.read_point(id).await {
            Ok(v) => println!("{id} = {v:?}"),
            Err(e) => println!("{id}: error: {e}"),
        }
    }
    Ok(())
}
