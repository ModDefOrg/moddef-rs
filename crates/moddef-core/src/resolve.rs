//! Import resolution (spec §19). Import URIs follow the package form
//! `moddef:<namespace>:<name>:<version>` (e.g. `moddef:stdlib:measurands:1.0.0`),
//! resolved against package roots as
//! `<root>/<name>/<version>/<name>.moddef.{yaml,json,binary}` — the layout of
//! moddef/stdlib and the Go resolver's MODDEF_PACKAGE_ROOTS.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::document::{parse_document, DocumentFormat, ParseError};
use crate::schema::{EnumType, MeasurandDefinition, ModDefDocument};

/// Supplies the bytes for an import uri. Implement over any store (fs, http,
/// embedded); [`DirResolver`] covers the standard directory layout.
pub trait PackageResolver {
    fn fetch(&self, uri: &str) -> Result<(Vec<u8>, DocumentFormat), ParseError>;
}

/// Resolver over `MODDEF_PACKAGE_ROOTS`-style directories.
pub struct DirResolver {
    roots: Vec<PathBuf>,
}

impl DirResolver {
    pub fn new<I, P>(roots: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        DirResolver {
            roots: roots.into_iter().map(Into::into).collect(),
        }
    }

    /// Roots from the `MODDEF_PACKAGE_ROOTS` environment variable (`:`-separated).
    pub fn from_env() -> Self {
        let roots = std::env::var("MODDEF_PACKAGE_ROOTS").unwrap_or_default();
        DirResolver::new(roots.split(':').filter(|s| !s.is_empty()))
    }
}

impl PackageResolver for DirResolver {
    fn fetch(&self, uri: &str) -> Result<(Vec<u8>, DocumentFormat), ParseError> {
        let parts: Vec<&str> = uri.split(':').collect();
        let [scheme, _ns, name, version] = parts[..] else {
            return Err(ParseError::new(format!("unsupported import uri: {uri}")));
        };
        if scheme != "moddef" {
            return Err(ParseError::new(format!("unsupported import uri: {uri}")));
        }
        let candidates = [
            (format!("{name}.moddef.yaml"), DocumentFormat::Yaml),
            (format!("{name}.moddef.json"), DocumentFormat::Json),
            (format!("{name}.moddef"), DocumentFormat::Binary),
        ];
        for root in &self.roots {
            for (file, format) in &candidates {
                let path = root.join(name).join(version).join(file);
                if let Ok(data) = std::fs::read(&path) {
                    return Ok((data, *format));
                }
            }
        }
        Err(ParseError::new(format!(
            "import not found under package roots: {uri}"
        )))
    }
}

/// A document's visible symbol tables after import resolution: local symbols
/// first, then imported ones (alias-prefixed as `<alias>:<id>`, never
/// overriding an existing entry).
pub struct ResolvedDocument {
    pub enums: BTreeMap<String, EnumType>,
    pub measurands: BTreeMap<String, MeasurandDefinition>,
    /// Imported documents keyed by uri.
    pub imports: BTreeMap<String, ModDefDocument>,
}

/// Resolve a document's imports and build the visible symbol tables.
pub fn resolve_imports(
    doc: &ModDefDocument,
    resolver: Option<&dyn PackageResolver>,
) -> Result<ResolvedDocument, ParseError> {
    let mut out = ResolvedDocument {
        enums: BTreeMap::new(),
        measurands: BTreeMap::new(),
        imports: BTreeMap::new(),
    };

    for e in &doc.enums {
        out.enums.insert(e.type_id.clone(), e.clone());
    }
    for m in &doc.measurands {
        out.measurands.insert(m.measurand_id.clone(), m.clone());
    }

    for imp in &doc.imports {
        let Some(resolver) = resolver else {
            return Err(ParseError::new(format!(
                "document imports {} but no resolver was supplied",
                imp.uri
            )));
        };
        let (data, format) = resolver.fetch(&imp.uri)?;
        let idoc = parse_document(&data, format)?;
        let prefix = if imp.alias.is_empty() {
            String::new()
        } else {
            format!("{}:", imp.alias)
        };
        for e in &idoc.enums {
            out.enums
                .entry(format!("{prefix}{}", e.type_id))
                .or_insert_with(|| e.clone());
        }
        for m in &idoc.measurands {
            out.measurands
                .entry(format!("{prefix}{}", m.measurand_id))
                .or_insert_with(|| m.clone());
        }
        out.imports.insert(imp.uri.clone(), idoc);
    }
    Ok(out)
}
