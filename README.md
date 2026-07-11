# moddef-rs

Rust runtime + code generator for [ModDef](../moddef) (spec v0.4) — declarative
Modbus device definitions.

| Crate | What it is |
| --- | --- |
| [`moddef-core`](crates/moddef-core) | Runtime: `no_std` codec core, `Transport` trait, typed errors; document parsing (`.moddef.yaml` / `.moddef.json` / binary `.moddef`) and an untyped `Device` facade under the default `std` feature. |
| [`moddef-codegen`](crates/moddef-codegen) | Generator: emits a typed `struct <Device><T: Transport>` per profile, plus the `moddef-rs gen` CLI. |
| [`moddef-tokio-modbus`](crates/moddef-tokio-modbus) | `Transport` adapter over tokio-modbus (TCP by default, RTU behind the `rtu` feature). |

## Parse a profile at runtime (no codegen)

```rust
use moddef_core::{Device, MeasurandQuery};
use moddef_tokio_modbus::{Options, TokioModbusTransport};

let doc = moddef_core::load("growatt-sph.moddef.yaml")?;
let transport = TokioModbusTransport::tcp("192.168.1.50:502".parse()?, Options::default()).await?;
let mut dev = Device::new(&doc, None, transport)?;

let soc = dev.read_point("state_of_charge").await?;                    // DecodedValue
let hz  = dev.read_measurand(&MeasurandQuery::base("frequency")).await?;
dev.write_point("ac_charge_enable", 1.0.into()).await?;               // §11.4 validated
```

## Or generate a typed client

```sh
moddef-rs gen -o src/generated growatt-sph.moddef.yaml
```

```rust
let mut dev = GrowattSph::new(transport);
let state = dev.inverter_status().await?;      // InverterRunState (lossless Unknown(u16))
let pv1   = dev.pv1_voltage().await?;          // f64, transform applied exactly
let soc   = dev.get_state_of_charge().await?;  // §26.2 measurand convenience
```

Generated clients drive the codec core directly over a `static` descriptor
table — no allocation, no `std` — so the same client runs on tokio and on a
Cortex-M with embassy. See [examples/embedded-nostd](examples/embedded-nostd)
(builds for `thumbv7em-none-eabihf`).

## Feature flags (`moddef-core`)

| Feature | Adds |
| --- | --- |
| *(none)* | Codec core, `PointDesc`, `Transport`, errors — `no_std`, no alloc. |
| `alloc` | prost schema types, owned `DecodedValue`, `Device` facade, binary `.moddef` parsing. |
| `std` *(default)* | YAML/JSON (protojson semantics), fs helpers, import resolution, `std::error::Error`. |

## Development

```sh
cargo test          # unit + conformance (needs sibling moddef/ and devices/ checkouts)
cargo build -p moddef-core --no-default-features   # no_std gate
```

Conformance tests parse the golden fixtures from `../moddef/fixtures` and all
blessed registry profiles from `../devices`, and compile-gate generated
clients for every registry profile. `DESIGN.md` documents the architecture
and the one known wire quirk (prost omits default-valued map-entry fields;
wire-equivalent with Go/TS, not always byte-identical).
