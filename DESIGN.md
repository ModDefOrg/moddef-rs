# moddef-rs — Rust Runtime + Generator

Design plan for the Rust implementation of ModDef (spec v0.4), following spec
§31 (Code Generation) and §32 (Client Library Architecture), with the Go
implementation as the behavioral reference and **moddef-ts as the porting
template** (same layering, same conformance strategy, same fixtures).

## Goals

- **`moddef-core`** runtime crate: `no_std`-friendly codec for embedded
  targets, transport trait, error types; document layer and untyped facade
  behind feature flags.
- **Runtime document parsing is a first-class path** (default `std`
  feature): load any `.moddef.{yaml,json,}` file at runtime and drive the
  untyped `Device` facade — no code generation required (gateway/CLI use
  case, parity with the Go client and the TS `Device`). Binary `.moddef`
  parsing additionally works under `no_std` + `alloc` (prost needs no std).
  The generated/static path exists *on top of* this for targets that cannot
  afford (or don't want) runtime parsing, not instead of it.
- **`moddef-codegen`**: generator (library + `moddef-rs` binary) that emits
  `struct <Device><T: Transport>` with typed async methods — real Rust enums
  for ModDef enums, structs for packed register fields, bitflag types for
  flag sets.
- **`moddef-tokio-modbus`**: `Transport` adapter for `tokio-modbus`
  (TCP + RTU via tokio-serial).
- Conformance against `moddef/fixtures` (§33), sharing the suite with Go/TS.

Non-goals for v0.1: the linter (stays in Go), YAML emission with comments,
an embassy/embedded-hal adapter (the `no_std` codec + Transport trait are
designed for it; the adapter itself is a follow-up), hardware-in-loop tests.

## The embedded architecture decision (drives everything)

An embedded target must never parse YAML/JSON/protobuf at runtime. The
generator therefore compiles the device profile *into the binary*:

```
        std / server path                        no_std embedded path
  ┌──────────────────────────┐           ┌───────────────────────────────┐
  │ document.rs (prost+serde)│           │ generated code:               │
  │ parse .moddef.{yaml,json,│           │   static POINTS: &[PointDesc] │
  │ binary} -> prost types   │           │   enum InverterRunState {...} │
  └───────────┬──────────────┘           └──────────────┬────────────────┘
              │ convert (alloc)                         │ &'static, zero heap
              ▼                                         ▼
        ┌──────────────────────────────────────────────────────┐
        │  codec core: decode/encode over `PointDesc` —        │
        │  no_std, no alloc, i128 rational math                │
        └──────────────────────────────────────────────────────┘
```

`PointDesc` is a plain, `'static`-friendly descriptor (no heap anywhere):

```rust
pub struct PointDesc<'a> {
    pub id: &'a str,
    pub space: AddressSpace,
    pub offset: u16,
    pub model_relative_offset: u16,   // §7.3, ID-relative (offset 0 = model id)
    pub length_words: u8,
    pub storage: StorageType,
    pub value: ValueKind<'a>,          // Decimal | Bool | Int | Uint | Enum | Flags(&'a [(u8, &'a str)]) | Fields(&'a [FieldDesc<'a>]) | Str | Bytes | DateTime(..)
    pub byte_big: bool,
    pub word_big: bool,
    pub scale: Option<Rational>,       // i64/i64
    pub offset_add: Option<Rational>,
    pub scale_ref: Option<ScaleRefDesc<'a>>,
    pub selector: Option<SelectorDesc<'a>>,
    pub na: &'a [NaDesc<'a>],
    pub access: Access,
    pub write: Option<WriteDesc<'a>>,  // §11.4 constraints
}
```

The codec core operates only on `PointDesc`; the `alloc` feature adds a
borrowing conversion `prost Point -> PointDesc` so the std facade reuses the
same core. Generated code emits `static` descriptor tables directly — this is
the Rust analog of the TS "embedded document", but with zero parse cost.

## Workspace layout

```
moddef-rs/
  Cargo.toml                  # workspace
  crates/
    moddef-core/              # runtime
      src/
        desc.rs               # PointDesc & friends (always available)
        codec/
          bytes.rs            # word/byte assembly (§9), no_std
          rat.rs              # i128 rational math (§10), no_std
          decode.rs           # decode(&PointDesc, &[u16], &Ctx) -> Value
          encode.rs           # encode(&PointDesc, &ValueRef, &Ctx, &mut [u16])
        value.rs              # Value / Reading<T> / Unavailable
        error.rs              # typed errors (§26.3/26.4), no_std core
        transport.rs          # async Transport trait (§32.3), no_std
        measurand.rs          # MeasurandQuery + matching (alloc)
        device.rs             # untyped Device facade (§32.4, alloc)
        schema/               # vendored prost (+ pbjson serde) output
        document.rs           # parse/serialize yaml|json|binary (std)
        resolve.rs            # import resolution (std)
    moddef-codegen/           # generator: lib + bin `moddef-rs`
      src/{lib.rs, naming.rs, emit_enums.rs, emit_points.rs, emit_device.rs, main.rs}
    moddef-tokio-modbus/      # tokio-modbus Transport adapter (std, tokio)
  conformance/                # test crate wired to ../moddef/fixtures + ../devices
  examples/
    growatt-tcp/              # tokio + generated client
    embedded-nostd/           # no_std build proof: codec + generated statics only
```

## moddef-core

### Feature flags

| feature | adds | pulls in |
|---|---|---|
| *(none)* | `PointDesc`, codec core, `Transport` trait, errors, `Value`/`Reading` | `core` only |
| `alloc` | owned `DecodedValue` (String/Vec), `decode_all`, prost schema types, prost→`PointDesc` conversion, `Device` facade, measurand queries | `alloc`, `prost` |
| `std` *(default, implies alloc)* | `std::error::Error` impls, `document.rs` file loading, `resolve.rs` package roots | `serde`, `serde_json`, `serde_yaml_ng`, `pbjson` |

CI enforces `cargo build --no-default-features --target thumbv7em-none-eabihf`
for the codec core (the `examples/embedded-nostd` crate links it plus a
generated profile).

### Codec (spec §8–§15, port of go/codec ↔ ts codec, kept in lockstep)

- **Numeric policy**: raw integers up to 64 bits; §10 transforms computed in
  an `i128`-backed rational (`(i128, i128)` with gcd normalization) — exact
  for every real register (64-bit raw × i64/i64 scale fits i128 with room),
  no bigint dependency, `no_std`-safe. Decoded numerics surface as `f64`
  (matching TS) plus `decode_raw()` for the pre-scale integer (billing
  counters).
- **Value model (no alloc)**:
  ```rust
  pub enum Value {
      Bool(bool), U64(u64), I64(i64), F64(f64),
      Flags(u64),           // raw mask; names iterate via PointDesc's table
      Fields(FieldsView),   // lazily extracts sub-values from the raw window
      DateTime(i64),        // epoch seconds or millis per spec
      Unavailable(&'static str),
  }
  ```
  Strings decode into caller buffers: `decode_str<'b>(&PointDesc, &[u16],
  &'b mut [u8]) -> Result<&'b str>`. With `alloc`, `DecodedValue` mirrors the
  TS union (owned `String`, `Vec<&'static str>` flag names, etc.).
- **Sentinels** (§8.4): masked raw match → `Value::Unavailable(meaning)`;
  typed accessors return `Reading<T> { Value(T), Unavailable(&str) }`.
- Full parity checklist as TS: endianness/word order, sign-magnitude,
  U24/U48/S48, IEEE754 f32/f64, strings (charset/padding/termination), BCD,
  composed mantissa/exponent, bit/register fields, flag sets, DATETIME,
  `scale_ref` POW10/MULTIPLY, `selector_ref` cases with transform fallback
  (post-fix Go semantics), write encoding.
- **Context**: `Ctx<'a>` = slice of `(point_id, i64)` pairs (no_std) with a
  map-backed variant under `alloc`.

### Transport trait (§32.3)

Native async-fn-in-trait (Rust ≥1.75; toolchain here is 1.95), buffer
out-params so `no_std` implementations (embassy RTU) allocate nothing:

```rust
pub trait Transport {
    type Error;
    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error>;
    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), Self::Error>;
    async fn read_coils(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error>;
    async fn read_discrete(&mut self, offset: u16, out: &mut [bool]) -> Result<(), Self::Error>;
    async fn write_holding(&mut self, offset: u16, values: &[u16]) -> Result<(), Self::Error>;
    async fn write_coil(&mut self, offset: u16, value: bool) -> Result<(), Self::Error>;
}
```

`&mut self` gives request serialization for free (no queue needed — the
borrow checker is the mutex). Unit-id selection and timeouts are adapter
constructor concerns; an `AbortSignal` analog is idiomatic `tokio::select!`
at the call site, not part of the trait.

### Errors (§26.3/26.4)

```rust
pub enum Error<T> {                    // T = Transport::Error
    Transport(T),
    PointNotFound,                     // facade paths carry the id (alloc)
    MeasurandNotSupported,
    AmbiguousMeasurand,
    UnsupportedMapping(&'static str),
    Decode(DecodeError), Encode(EncodeError),
    WriteAccess, WriteConstraint(ConstraintKind),
    BufferTooSmall,
}
```
`core::fmt::Display` always; `std::error::Error` under `std`. Structured
variants (constraint kind, decode cause) rather than strings, mirroring TS.

### Device facade (§32.4, `alloc`)

`Device<'d, T: Transport>` bound to a prost `DeviceProfile`: `read_point`,
`read_measurand(MeasurandQuery)` with §26.4 ambiguity error, `write_point`
with §11.4 constraint validation, SunSpec discovery (SunS anchor probe +
model-chain walk, **ID-relative** `model_relative_offset` per spec §7.3 —
the convention Go/TS now share), automatic `scale_ref`/`selector_ref`
companion reads, cached model base per block. Same declared limitation as
Go/TS: composed points decode via the codec directly.

## moddef-codegen

Library `generate(&ModDefDocument) -> Vec<GeneratedFile>` + `moddef-rs gen
-o src/generated <file.moddef.yaml>`. Emission via `quote!` +
`prettyplease` (rustfmt-clean output without invoking rustfmt), fully
deterministic (document order, no timestamps).

Per document it emits one module containing:

1. **Enums** — real Rust enums with lossless unknown handling:
   ```rust
   #[derive(Clone, Copy, Debug, PartialEq, Eq)]
   pub enum InverterRunState { Waiting, Normal, Fault, Unknown(u16) }
   impl From<u16> for InverterRunState { ... }   // never fails, Unknown(raw)
   impl InverterRunState { pub const fn value(self) -> u16 { ... } pub const fn name(self) -> &'static str { ... } }
   ```
2. **Flag types** — generated bitflag structs (no `bitflags` dep):
   `SafetyFunctionEnable(u16)` with `const SPI_ENABLE: Self`, `contains()`,
   `iter_names() -> impl Iterator<Item = &'static str>`.
3. **Field structs** — packed windows as plain structs with `From<u64>`:
   `pub struct GridFirstSlot1Start { pub hour: u8, pub minute: u8 }`.
4. **Descriptor table** — `pub static POINTS: &[PointDesc<'static>]` (the
   §32.1 catalog; usable standalone with the codec core on no_std).
5. **Typed device struct** (the user-facing API):
   ```rust
   pub struct GrowattSph<T: Transport> { pub transport: T, model_base: [Option<u16>; N_DISCOVERY_BLOCKS] }
   impl<T: Transport> GrowattSph<T> {
       pub async fn inverter_status(&mut self) -> Result<InverterRunState, Error<T::Error>>;
       pub async fn pv1_voltage(&mut self) -> Result<f64, Error<T::Error>>;
       pub async fn bms_soc(&mut self) -> Result<Reading<f64>, Error<T::Error>>;      // has na_values
       pub async fn grid_first_slot1_start(&mut self) -> Result<GridFirstSlot1Start, Error<T::Error>>;
       pub async fn set_ac_charge_enable(&mut self, v: EnableState) -> Result<(), Error<T::Error>>;  // §11.4 validated
       pub async fn serial_number<'b>(&mut self, buf: &'b mut [u8]) -> Result<&'b str, Error<T::Error>>;
   }
   ```
   The generated struct drives the codec core directly over `POINTS` — **it
   does not require the `alloc` facade**, so the same generated client runs
   on tokio and on embassy. Read buffers are fixed-size stack arrays
   (max `length_words` is known at generation time).
6. **Measurand convenience** (§26.2) — generated qualifier enums narrow to
   what the profile has (Rust's version of the TS literal unions):
   ```rust
   pub enum VoltagePhase { L1N, L2N, L3N, L1L2, L2L3, L3L1 }   // only present phases
   pub async fn get_voltage(&mut self, phase: VoltagePhase) -> Result<f64, ...>;
   pub async fn get_frequency(&mut self) -> Result<f64, ...>;   // unique match: no args
   ```
   Method names are `get_<base_quantity>` (never collide with point methods,
   same policy as TS). Permanently ambiguous tuples fall back to a doc
   comment pointing at the untyped facade.

Naming: snake_case point ids map 1:1 to method names (collision suffixes via
the shared seen-set policy); Pascal for types; enum members Pascal-ized from
SCREAMING_SNAKE. Reserved words get `r#`/suffix handling.

## moddef-tokio-modbus

Wraps `tokio_modbus::client::Context`:

- Constructors: `tcp(addr, Options)`, `rtu(serial_path, Options)` (via
  `tokio-serial`), `wrap(ctx)`; options carry `unit_id` (`Slave`),
  `timeout`, `max_read_words` (default 125; devices like the SDM630/EM24
  need less — same knob as the TS adapter).
- Implements `Transport` with chunked reads honoring `max_read_words`,
  `tokio::time::timeout` per request, and Modbus exceptions surfaced as a
  structured `TokioModbusError { kind, exception_code }`.
- `&mut self` on the trait means no queue is needed; concurrent access is a
  caller decision (`Mutex<GrowattSph<..>>` or an actor task).

## Testing & conformance (spec §33)

1. **Fixture equivalence** (conformance crate): YAML/JSON/binary triples
   parse to equal prost messages; re-serialization is lossless; **binary
   byte-equality with the Go-produced goldens** (requires `btree_map` maps —
   see upstream prerequisites); `invalid/` fixtures with
   `schema_valid: false` must fail to parse.
2. **Codec vectors**: the same unit vectors as Go/TS (scaled U16, S16, word
   orders, U64, S48, F32, sentinels, strings, BCD, flags, fields, datetime,
   `scale_ref` POW10, `selector_ref` cases + fallback, composed, encode
   round-trips incl. negative-offset transforms).
3. **Generator tests**: snapshot (insta or golden files) for the
   battery-control fixture; generated output for all goldens + all 8
   registry profiles must pass `cargo check` (a test build-dir crate, the
   Rust analog of the TS `tsc --noEmit` gate); e2e — generated
   `GrowattSph<MockTransport>` / `EastronSdm630<..>` against an in-memory
   register image (enum mapping, packed fields, flags, constrained writes,
   measurand convenience + ambiguity).
4. **Facade tests**: mock Transport incl. the SunSpec discovery walk with
   canonical ID-relative offsets (mirror of the Go/TS test).
5. **no_std proof**: `examples/embedded-nostd` (`#![no_std]` staticlib using
   codec core + a generated profile) built for `thumbv7em-none-eabihf` in CI.
6. **Adapter test**: `tokio-modbus` server feature — in-process TCP server,
   round-trip reads/writes/chunking/exception mapping (mirror of the
   modbus-serial ServerTCP test).

## Upstream prerequisites (moddef repo, same pattern as the TS fixes)

1. **prost maps → BTreeMap**: add `btree_map=.` to the prost plugin opts in
   `buf.gen.yaml`. Required for (a) `no_std`+alloc compatibility
   (`std::collections::HashMap` is std-only) and (b) deterministic binary
   encoding to match the Go golden bytes. Patch the checked-in
   `gen/rust/moddef/v1/moddef.v1.rs` the same way the TS `.js` extensions
   were patched.
2. **protojson serde impls**: add the pbjson companion plugin
   (`buf.build/community/neoeinstein-prost-serde`) so YAML/JSON parse with
   protojson semantics (field-name aliases, enum string values, unknown
   fields rejected) instead of hand-rolling a serde mirror. Vendored into
   `moddef-core/src/schema/` like the TS package vendors protobuf-es output,
   with a `sync-schema` script + CI drift check.
3. If the remote-plugin route is blocked (buf not installed locally),
   fallback: check in a `prost-build`/`pbjson-build` `build.rs` under
   `moddef-rs` that generates from `../moddef/proto` at build time, and keep
   the vendored copy for publishing. (Preferred: fix buf.gen.yaml, since CI
   regenerates there.)

## Milestones

1. **Workspace + schema + document layer** — cargo workspace, vendored
   prost+pbjson schema (after upstream prereqs), `document.rs` parse/
   serialize, fixture equivalence green. ✅ **Done.** protox compiles the
   vendored protos in build.rs (no buf/protoc needed). All golden fixtures
   parse equal across yaml/json/binary and round-trip losslessly; all 8
   registry profiles parse. One documented wire quirk: prost omits
   default-valued map-entry fields (Go/protobuf-es emit them), so binary
   byte-equality with Go goldens holds except for maps with a zero key
   (e.g. a flag on bit 0) — those are compared through a prost re-encode
   of the golden. Wire-equivalent either way.
2. **Codec core** — `PointDesc`, bytes/rat, decode then encode, full vector
   parity; `--no-default-features` builds. ✅ **Done.** 25 vector tests
   mirroring the Go/TS suites; no_std and alloc-only configs build clean.
3. **Transport trait + errors + facade** — mock-transport tests incl.
   SunSpec discovery and constrained writes. ✅ **Done.** 6 facade tests:
   reads/scaling, sentinels with meaning, measurand queries (qualified,
   ambiguous, unsupported), constrained writes (min/max/step/allowed,
   inverse transform), SunSpec chain walk with ID-relative offsets and
   anchor caching.
4. **Codegen** — enums/flags/fields → descriptor table → device struct →
   measurand convenience; snapshot + cargo-check gate over the registry;
   CLI. ✅ **Done.** quote+prettyplease emitter; descriptor contents come
   from `moddef_core::convert::point_desc`, so the generated `POINTS` table
   is by construction identical to the dynamic facade's view. Generated
   method bodies stay small via a `moddef_core::rt` no_std support module
   (fixed-size reads, SunSpec resolution, value extraction). Compile gate:
   conformance build.rs generates clients for all 8 registry profiles + the
   SunSpec fixture and includes them as modules; e2e mock-transport tests
   drive the growatt (enum/fields/write-constraints) and sunspec
   (discovery + scale_ref + Reading sentinel) clients. `moddef-rs gen -o
   <dir> <files…>` CLI ships in moddef-codegen. Note: generated files carry
   no inner `#![allow]` (rejected under `include!` in a mod block) — put
   the allow on the wrapping module.
5. **tokio-modbus adapter + examples** — in-process server test; growatt-tcp
   example; embedded-nostd example + thumb target build. ✅ **Done.**
   tokio-modbus 0.17 (`tcp` default, `rtu` feature via tokio-serial);
   chunked reads honor `max_read_words`, per-request `tokio::time::timeout`,
   exceptions surface as `TokioModbusError::Exception(ExceptionCode)`.
   Tests run against an in-process tokio-modbus TCP server (chunking,
   writes, exception mapping, dynamic facade over real TCP). Examples:
   `crates/moddef-tokio-modbus/examples/growatt_tcp.rs` (runtime-parsed
   profile, no codegen) and `examples/embedded-nostd` (generated Growatt
   client compiled for `thumbv7em-none-eabihf`, no std / no alloc —
   verified locally).
6. **CI + docs** — GitHub Actions (stable + MSRV job, thumb no_std job,
   schema drift check), READMEs, crate metadata for publishing
   (`moddef-core`, `moddef-codegen`, `moddef-tokio-modbus`). ✅ **Done**
   (first two items): `.github/workflows/ci.yml` with test/clippy/fmt,
   no_std + thumb, MSRV 1.85, and vendored-proto drift jobs; root README;
   `sync-schema.sh`. Remaining before a crates.io release: crate metadata
   polish (keywords/categories/docs.rs config) and version pinning.

## Decisions taken (defaults, flag if you disagree)

- **Crate names** `moddef-core` / `moddef-codegen` / `moddef-tokio-modbus`
  in the `moddef-rs` repo (mirrors `@moddef/*`); the `moddef-rs` name is the
  CLI binary. If you want the runtime published literally as `moddef-rs`,
  it's a one-line rename in the workspace.
- **f64 for scaled values** (parity with TS) with exact `i128` rational
  internals and `decode_raw()` for exactness — no bigint/num-rational dep.
- **Unknown enum values are lossless** (`Unknown(u16)` variant) rather than
  erroring — real devices return undocumented states; parity with TS
  returning the raw number.
- **quote + prettyplease** for emission (guaranteed-parseable output) over
  string templates.
- **Async-only API** (native AFIT). A blocking wrapper is trivial for users
  (`block_on`) and not worth a second API surface in v0.1.
- MSRV: 1.85 (edition 2024) unless you need older.
