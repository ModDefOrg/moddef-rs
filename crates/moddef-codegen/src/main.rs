// SPDX-License-Identifier: Apache-2.0

//! `moddef-rs` CLI: generate typed Rust clients from ModDef documents.
//!
//! Usage: `moddef-rs gen [-o <dir>] <file.moddef[.yaml|.json]>...`

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => gen(&args[1..]),
        _ => {
            eprintln!("usage: moddef-rs gen [-o <dir>] <file.moddef[.yaml|.json]>...");
            ExitCode::from(2)
        }
    }
}

fn gen(args: &[String]) -> ExitCode {
    let mut out_dir = PathBuf::from(".");
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => {
                let Some(d) = it.next() else {
                    eprintln!("moddef-rs gen: {a} requires a directory argument");
                    return ExitCode::from(2);
                };
                out_dir = PathBuf::from(d);
            }
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        eprintln!("moddef-rs gen: no input files");
        return ExitCode::from(2);
    }

    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("moddef-rs gen: cannot create {}: {e}", out_dir.display());
        return ExitCode::FAILURE;
    }
    for f in files {
        let doc = match moddef_core::load(f) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!("moddef-rs gen: {e}");
                return ExitCode::FAILURE;
            }
        };
        for g in moddef_codegen::generate(&doc) {
            let path = out_dir.join(&g.path);
            if let Err(e) = std::fs::write(&path, &g.content) {
                eprintln!("moddef-rs gen: cannot write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            println!("{}", path.display());
        }
    }
    ExitCode::SUCCESS
}
