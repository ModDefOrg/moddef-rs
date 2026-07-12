// SPDX-License-Identifier: Apache-2.0

//! Identifier derivation: ModDef ids (snake/kebab/free-form) → Rust idents,
//! with keyword escaping and collision suffixes (same seen-set policy as the
//! TS generator's Scope).

use std::collections::BTreeSet;

use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use proc_macro2::Ident;
use quote::format_ident;

/// Keywords that cannot be raw identifiers — suffix instead.
const NO_RAW: &[&str] = &["self", "Self", "super", "crate", "_"];

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield",
];

fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'n');
    }
    out
}

pub fn snake(s: &str) -> String {
    sanitize(s).to_snake_case()
}

pub fn pascal(s: &str) -> String {
    let p = sanitize(s).to_pascal_case();
    if p.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("N{p}")
    } else {
        p
    }
}

pub fn shouty(s: &str) -> String {
    sanitize(s).to_shouty_snake_case()
}

/// A name → syn Ident, escaping keywords (`r#type`, or `_`-suffixed where raw
/// idents are not allowed).
pub fn ident(name: &str) -> Ident {
    if NO_RAW.contains(&name) {
        format_ident!("{name}_")
    } else if KEYWORDS.contains(&name) {
        format_ident!("r#{name}")
    } else {
        format_ident!("{name}")
    }
}

/// Collision-avoiding name set: `claim("soc")` twice yields `soc`, `soc_2`.
#[derive(Default)]
pub struct Scope {
    used: BTreeSet<String>,
}

impl Scope {
    pub fn new() -> Self {
        Scope::default()
    }

    pub fn claim(&mut self, base: &str) -> String {
        self.claim_fmt(base, "_")
    }

    /// Suffix without the underscore, keeping Pascal names camel-case
    /// (`Unknown` → `Unknown2`).
    pub fn claim_pascal(&mut self, base: &str) -> String {
        self.claim_fmt(base, "")
    }

    fn claim_fmt(&mut self, base: &str, sep: &str) -> String {
        if self.used.insert(base.to_owned()) {
            return base.to_owned();
        }
        for i in 2.. {
            let name = format!("{base}{sep}{i}");
            if self.used.insert(name.clone()) {
                return name;
            }
        }
        unreachable!()
    }
}
