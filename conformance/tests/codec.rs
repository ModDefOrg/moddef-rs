// SPDX-License-Identifier: Apache-2.0

//! Codec unit vectors mirroring go/codec/decode_test.go and moddef-ts
//! codec.test.ts: integer widths, endianness, scaling, refs, strings, BCD,
//! flags, fields, datetime, sentinels, composed values, encode round-trips,
//! and §11.4 write constraint validation — all at the heap-free
//! [`PointDesc`] level the no_std core exposes.

use moddef_core::codec::{decode, decode_str, encode, encode_str, validate_write, Ctx};
use moddef_core::desc::point;
use moddef_core::value::{field_value, flag_names};
use moddef_core::{
    AddressSpace, ConstraintKind, DateTimeEncoding, DecodeError, FieldDesc, NaDesc, PointDesc,
    Rational, ScaleMode, ScaleRefDesc, SelectorCaseDesc, SelectorDesc, StorageType, StringPadding,
    StringTermination, Value, ValueKind, WriteDesc,
};

const H: AddressSpace = AddressSpace::HoldingRegister;

fn rat(num: i64, den: i64) -> Option<Rational> {
    Some(Rational { num, den })
}

fn scaled(id: &str, storage: StorageType, num: i64, den: i64) -> PointDesc<'_> {
    let mut p = point(id, H, 0, storage);
    p.scale = rat(num, den);
    p
}

fn f64_of(v: Value) -> f64 {
    match v {
        Value::F64(f) => f,
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn u16_with_scale() {
    let p = scaled("v", StorageType::U16, 1, 10);
    assert!((f64_of(decode(&p, &[2305], &Ctx::EMPTY).unwrap()) - 230.5).abs() < 1e-10);
}

#[test]
fn s16_negative_twos_complement() {
    let p = scaled("t", StorageType::S16, 1, 10);
    assert!((f64_of(decode(&p, &[0xfff6], &Ctx::EMPTY).unwrap()) + 1.0).abs() < 1e-10);
}

#[test]
fn u32_word_orders() {
    let mut p = point("e", H, 0, StorageType::U32);
    p.value = ValueKind::Uint;
    assert_eq!(
        decode(&p, &[0x0001, 0x86a0], &Ctx::EMPTY).unwrap(),
        Value::U64(100000)
    );

    p.word_big = false;
    assert_eq!(
        decode(&p, &[0x86a0, 0x0001], &Ctx::EMPTY).unwrap(),
        Value::U64(100000)
    );
}

#[test]
fn u64_uint() {
    let mut p = point("acc", H, 0, StorageType::U64);
    p.value = ValueKind::Uint;
    assert_eq!(
        decode(&p, &[0, 1, 0, 0], &Ctx::EMPTY).unwrap(),
        Value::U64(4294967296)
    );
}

#[test]
fn s48_sign_extension() {
    let mut p = point("n", H, 0, StorageType::S48);
    p.value = ValueKind::Int;
    assert_eq!(
        decode(&p, &[0xffff, 0xffff, 0xfffe], &Ctx::EMPTY).unwrap(),
        Value::I64(-2)
    );
}

#[test]
fn enum_backed_returns_raw() {
    let mut p = point("mode", H, 0, StorageType::U16);
    p.value = ValueKind::Enum;
    assert_eq!(decode(&p, &[5], &Ctx::EMPTY).unwrap(), Value::U64(5));
}

#[test]
fn ieee754_f32() {
    // 230.5f = 0x43668000
    let p = point("v", H, 0, StorageType::F32);
    assert!((f64_of(decode(&p, &[0x4366, 0x8000], &Ctx::EMPTY).unwrap()) - 230.5).abs() < 1e-4);
}

#[test]
fn sentinel_u16() {
    let mut p = point("a", H, 0, StorageType::U16);
    p.na = &[NaDesc {
        raw: 65535,
        meaning: "not_implemented",
    }];
    assert_eq!(
        decode(&p, &[0xffff], &Ctx::EMPTY).unwrap(),
        Value::Unavailable
    );
    assert_ne!(
        decode(&p, &[0xfffe], &Ctx::EMPTY).unwrap(),
        Value::Unavailable
    );
}

#[test]
fn sentinel_s16_masked() {
    let mut p = point("a", H, 0, StorageType::S16);
    p.na = &[NaDesc {
        raw: 32768,
        meaning: "",
    }];
    assert_eq!(
        decode(&p, &[0x8000], &Ctx::EMPTY).unwrap(),
        Value::Unavailable
    );
    assert_ne!(
        decode(&p, &[0x7fff], &Ctx::EMPTY).unwrap(),
        Value::Unavailable
    );
}

#[test]
fn fixed_length_space_padded_ascii() {
    let mut p = point("sn", H, 0, StorageType::StringAscii);
    p.length_words = 3;
    p.value = ValueKind::Str {
        padding: StringPadding::Space,
        termination: StringTermination::FixedLength,
    };
    let mut buf = [0u8; 6];
    // "AB12  "
    assert_eq!(
        decode_str(&p, &[0x4142, 0x3132, 0x2020], &mut buf).unwrap(),
        "AB12"
    );
}

#[test]
fn bcd_digits() {
    let p = point("b", H, 0, StorageType::Bcd);
    assert_eq!(
        decode(&p, &[0x1234], &Ctx::EMPTY).unwrap(),
        Value::I64(1234)
    );
}

#[test]
fn flag_set_names() {
    const TABLE: &[(u8, &str)] = &[(0, "over_voltage"), (2, "over_temp"), (7, "door_open")];
    let mut p = point("alarms", H, 0, StorageType::U16);
    p.value = ValueKind::Flags(TABLE);

    let Value::Flags(mask) = decode(&p, &[0b1000_0101], &Ctx::EMPTY).unwrap() else {
        panic!("expected flags");
    };
    let names: Vec<&str> = flag_names(&p, mask).collect();
    assert_eq!(names, ["over_voltage", "over_temp", "door_open"]);

    let Value::Flags(mask) = decode(&p, &[0], &Ctx::EMPTY).unwrap() else {
        panic!("expected flags");
    };
    assert_eq!(flag_names(&p, mask).count(), 0);
}

#[test]
fn register_fields_hour_minute() {
    const FIELDS: &[FieldDesc<'_>] = &[
        FieldDesc {
            id: "hour",
            bit_offset: 8,
            bit_length: 8,
        },
        FieldDesc {
            id: "minute",
            bit_offset: 0,
            bit_length: 8,
        },
    ];
    let mut p = point("slot", H, 0, StorageType::U16);
    p.value = ValueKind::Fields(FIELDS);

    let Value::Fields(window) = decode(&p, &[(21 << 8) | 45], &Ctx::EMPTY).unwrap() else {
        panic!("expected fields");
    };
    assert_eq!(field_value(&FIELDS[0], window), 21);
    assert_eq!(field_value(&FIELDS[1], window), 45);
}

#[test]
fn datetime_epoch_seconds() {
    let mut p = point("rtc", H, 0, StorageType::U32);
    p.value = ValueKind::DateTime(DateTimeEncoding::EpochSeconds);
    assert_eq!(
        decode(&p, &[0x6543, 0x2100], &Ctx::EMPTY).unwrap(),
        Value::DateTime(0x65432100)
    );
}

#[test]
fn scale_ref_pow10() {
    let mut p = point("w", H, 0, StorageType::S16);
    p.scale_ref = Some(ScaleRefDesc {
        point_id: "w_sf",
        mode: ScaleMode::Pow10,
        denominator: 0,
    });

    let ctx = Ctx {
        refs: &[("w_sf", -1)],
    };
    assert!((f64_of(decode(&p, &[2301], &ctx).unwrap()) - 230.1).abs() < 1e-10);

    let ctx = Ctx {
        refs: &[("w_sf", 2)],
    };
    assert_eq!(f64_of(decode(&p, &[15], &ctx).unwrap()), 1500.0);
}

#[test]
fn scale_ref_missing_errors() {
    let mut p = point("w", H, 0, StorageType::S16);
    p.scale_ref = Some(ScaleRefDesc {
        point_id: "w_sf",
        mode: ScaleMode::Pow10,
        denominator: 0,
    });
    assert_eq!(
        decode(&p, &[2301], &Ctx::EMPTY),
        Err(DecodeError::UnresolvedRef)
    );
}

#[test]
fn composed_mantissa_exponent() {
    let mut p = point("pwr", H, 0, StorageType::Composed);
    p.length_words = 2;
    p.value = ValueKind::Composed {
        base: 10,
        mantissa_offset: 0,
        mantissa_words: 1,
        exponent_offset: 1,
        exponent_words: 1,
    };
    // 1500 * 10^-1
    assert!((f64_of(decode(&p, &[1500, 0xffff], &Ctx::EMPTY).unwrap()) - 150.0).abs() < 1e-10);
}

#[test]
fn selector_cases() {
    const CASES: &[SelectorCaseDesc] = &[
        SelectorCaseDesc {
            key: 0,
            scale: Some(Rational { num: 1, den: 1000 }),
            offset: None,
        },
        SelectorCaseDesc {
            key: 1,
            scale: Some(Rational { num: 1, den: 1 }),
            offset: None,
        },
    ];
    let mut p = point("energy", H, 0, StorageType::U32);
    p.selector = Some(SelectorDesc {
        point_id: "fmt",
        cases: CASES,
    });

    let ctx = Ctx {
        refs: &[("fmt", 0)],
    };
    assert!((f64_of(decode(&p, &[0, 5000], &ctx).unwrap()) - 5.0).abs() < 1e-10);
    let ctx = Ctx {
        refs: &[("fmt", 1)],
    };
    assert_eq!(f64_of(decode(&p, &[0, 5000], &ctx).unwrap()), 5000.0);
}

// --- encode round-trips ---------------------------------------------------- //

#[test]
fn encode_scaled_u16_round_trip() {
    let p = scaled("sp", StorageType::U16, 1, 10);
    let mut regs = [0u16; 1];
    encode(&p, &Value::F64(230.5), &Ctx::EMPTY, &mut regs).unwrap();
    assert_eq!(regs, [2305]);
    assert!((f64_of(decode(&p, &regs, &Ctx::EMPTY).unwrap()) - 230.5).abs() < 1e-10);
}

#[test]
fn encode_negative_offset_round_trip() {
    // value = raw*0.1 - 1 (Growatt EPS power factor style)
    let mut p = scaled("pf", StorageType::U16, 1, 10);
    p.offset_add = rat(-1, 1);
    assert!((f64_of(decode(&p, &[15], &Ctx::EMPTY).unwrap()) - 0.5).abs() < 1e-10);

    let mut regs = [0u16; 1];
    encode(&p, &Value::F64(0.5), &Ctx::EMPTY, &mut regs).unwrap();
    assert_eq!(regs, [15]);
}

#[test]
fn encode_scale_ref_divides() {
    let mut p = point("w", H, 0, StorageType::S16);
    p.scale_ref = Some(ScaleRefDesc {
        point_id: "w_sf",
        mode: ScaleMode::Pow10,
        denominator: 0,
    });
    let ctx = Ctx {
        refs: &[("w_sf", -1)],
    };
    let mut regs = [0u16; 1];
    encode(&p, &Value::F64(230.1), &ctx, &mut regs).unwrap();
    assert_eq!(regs, [2301]);
}

#[test]
fn encode_string_round_trip() {
    let mut p = point("s", H, 0, StorageType::StringAscii);
    p.length_words = 2;
    p.value = ValueKind::Str {
        padding: StringPadding::Null,
        termination: StringTermination::FixedLength,
    };
    let mut regs = [0u16; 2];
    encode_str(&p, "Hi!", &mut regs).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(decode_str(&p, &regs, &mut buf).unwrap(), "Hi!");
}

#[test]
fn encode_flags_bool_datetime_f32_round_trips() {
    const TABLE: &[(u8, &str)] = &[(1, "a"), (3, "b")];
    let mut fl = point("f", H, 0, StorageType::U16);
    fl.value = ValueKind::Flags(TABLE);
    let mut regs = [0u16; 1];
    encode(&fl, &Value::Flags(1 << 3), &Ctx::EMPTY, &mut regs).unwrap();
    let Value::Flags(mask) = decode(&fl, &regs, &Ctx::EMPTY).unwrap() else {
        panic!("expected flags");
    };
    assert_eq!(flag_names(&fl, mask).collect::<Vec<_>>(), ["b"]);

    let mut b = point("b", H, 0, StorageType::U16);
    b.value = ValueKind::Bool;
    encode(&b, &Value::Bool(true), &Ctx::EMPTY, &mut regs).unwrap();
    assert_eq!(decode(&b, &regs, &Ctx::EMPTY).unwrap(), Value::Bool(true));

    let mut dt = point("d", H, 0, StorageType::U32);
    dt.value = ValueKind::DateTime(DateTimeEncoding::EpochSeconds);
    let mut regs2 = [0u16; 2];
    encode(
        &dt,
        &Value::DateTime(1_700_000_000),
        &Ctx::EMPTY,
        &mut regs2,
    )
    .unwrap();
    assert_eq!(
        decode(&dt, &regs2, &Ctx::EMPTY).unwrap(),
        Value::DateTime(1_700_000_000)
    );

    let f32p = point("f32", H, 0, StorageType::F32);
    encode(&f32p, &Value::F64(230.5), &Ctx::EMPTY, &mut regs2).unwrap();
    assert!((f64_of(decode(&f32p, &regs2, &Ctx::EMPTY).unwrap()) - 230.5).abs() < 1e-4);
}

#[test]
fn encode_s16_negative() {
    let p = scaled("t", StorageType::S16, 1, 10);
    let mut regs = [0u16; 1];
    encode(&p, &Value::F64(-1.0), &Ctx::EMPTY, &mut regs).unwrap();
    assert_eq!(regs, [0xfff6]);
}

// --- §11.4 write constraints ----------------------------------------------- //

#[test]
fn write_constraints() {
    let mut p = point("soc", H, 0, StorageType::U16);
    p.write = Some(WriteDesc {
        min: Some(Rational { num: 0, den: 1 }),
        max: Some(Rational { num: 100, den: 1 }),
        step: Some(Rational { num: 1, den: 1 }),
        allowed: &[],
    });
    assert_eq!(validate_write(&p, &Value::F64(80.0)), Ok(()));
    assert_eq!(
        validate_write(&p, &Value::F64(101.0)),
        Err(ConstraintKind::Max)
    );
    assert_eq!(
        validate_write(&p, &Value::F64(-1.0)),
        Err(ConstraintKind::Min)
    );
    assert_eq!(
        validate_write(&p, &Value::F64(50.5)),
        Err(ConstraintKind::Step)
    );

    let mut m = point("mode", H, 0, StorageType::U16);
    m.write = Some(WriteDesc {
        min: None,
        max: None,
        step: None,
        allowed: &[0, 1, 2],
    });
    assert_eq!(validate_write(&m, &Value::U64(2)), Ok(()));
    assert_eq!(
        validate_write(&m, &Value::U64(3)),
        Err(ConstraintKind::AllowedValues)
    );
}
