// SPDX-License-Identifier: Apache-2.0

//! Document layer (spec §4): parse and serialize the three equivalent
//! encodings — `.moddef.yaml`, `.moddef.json`, `.moddef` (proto binary).
//! YAML and JSON follow proto3 JSON (protojson) semantics via the
//! pbjson-generated serde impls; unknown fields and invalid enum values are
//! rejected, matching the Go implementation and the fixtures under
//! moddef/fixtures/invalid. YAML goes through a YAML→JSON value conversion
//! (numeric keys stringified) — the same round-trip the Go loader uses.

use std::fmt;
use std::path::Path;

use prost::Message;

use crate::schema::ModDefDocument;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentFormat {
    Yaml,
    Json,
    Binary,
}

/// Document-layer failure: unreadable file, malformed input, protojson
/// violations (unknown field, bad enum value), or a bad import uri.
#[derive(Debug)]
pub struct ParseError(String);

impl ParseError {
    pub fn new(msg: impl Into<String>) -> Self {
        ParseError(msg.into())
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Infer the format from a file path per the spec §4 extensions.
pub fn detect_format(path: &str) -> Result<DocumentFormat, ParseError> {
    if path.ends_with(".moddef.yaml") || path.ends_with(".moddef.yml") {
        Ok(DocumentFormat::Yaml)
    } else if path.ends_with(".moddef.json") {
        Ok(DocumentFormat::Json)
    } else if path.ends_with(".moddef") {
        Ok(DocumentFormat::Binary)
    } else {
        Err(ParseError::new(format!(
            "cannot detect ModDef format from path: {path}"
        )))
    }
}

/// Parse a document from bytes in the given format.
pub fn parse_document(data: &[u8], format: DocumentFormat) -> Result<ModDefDocument, ParseError> {
    match format {
        DocumentFormat::Binary => ModDefDocument::decode(data)
            .map_err(|e| ParseError::new(format!("failed to parse ModDef binary document: {e}"))),
        DocumentFormat::Json => serde_json::from_slice(data)
            .map_err(|e| ParseError::new(format!("failed to parse ModDef json document: {e}"))),
        DocumentFormat::Yaml => {
            let y: serde_yaml::Value = serde_yaml::from_slice(data).map_err(|e| {
                ParseError::new(format!("failed to parse ModDef yaml document: {e}"))
            })?;
            let j = yaml_to_json(y)?;
            serde_json::from_value(j)
                .map_err(|e| ParseError::new(format!("failed to parse ModDef yaml document: {e}")))
        }
    }
}

/// Serialize a document. The binary form is deterministic (BTreeMap-backed
/// maps), so it byte-matches Go-produced `.moddef` files.
pub fn serialize_document(
    doc: &ModDefDocument,
    format: DocumentFormat,
) -> Result<Vec<u8>, ParseError> {
    match format {
        DocumentFormat::Binary => Ok(doc.encode_to_vec()),
        DocumentFormat::Json => {
            let mut out = serde_json::to_vec_pretty(doc)
                .map_err(|e| ParseError::new(format!("failed to serialize json: {e}")))?;
            out.push(b'\n');
            Ok(out)
        }
        DocumentFormat::Yaml => {
            let j = serde_json::to_value(doc)
                .map_err(|e| ParseError::new(format!("failed to serialize yaml: {e}")))?;
            serde_yaml::to_string(&j)
                .map(String::into_bytes)
                .map_err(|e| ParseError::new(format!("failed to serialize yaml: {e}")))
        }
    }
}

/// Load and parse a `.moddef.yaml` / `.moddef.json` / `.moddef` file.
pub fn load(path: impl AsRef<Path>) -> Result<ModDefDocument, ParseError> {
    let path = path.as_ref();
    let format = detect_format(&path.to_string_lossy())?;
    let data = std::fs::read(path)
        .map_err(|e| ParseError::new(format!("failed to read {}: {e}", path.display())))?;
    parse_document(&data, format)
}

/// Serialize and write a document; the format comes from the extension.
pub fn save(doc: &ModDefDocument, path: impl AsRef<Path>) -> Result<(), ParseError> {
    let path = path.as_ref();
    let format = detect_format(&path.to_string_lossy())?;
    let data = serialize_document(doc, format)?;
    std::fs::write(path, data)
        .map_err(|e| ParseError::new(format!("failed to write {}: {e}", path.display())))
}

/// YAML value → JSON value with protojson-compatible keys: YAML allows
/// numeric/bool mapping keys, JSON (and protojson int64-keyed maps) want
/// strings — stringify them, like the Go yaml→JSON conversion does.
fn yaml_to_json(v: serde_yaml::Value) -> Result<serde_json::Value, ParseError> {
    use serde_json::Value as J;
    use serde_yaml::Value as Y;
    Ok(match v {
        Y::Null => J::Null,
        Y::Bool(b) => J::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                J::from(i)
            } else if let Some(u) = n.as_u64() {
                J::from(u)
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(J::Number)
                    .ok_or_else(|| ParseError::new("non-finite number in yaml document"))?
            } else {
                return Err(ParseError::new("unrepresentable number in yaml document"));
            }
        }
        Y::String(s) => J::String(s),
        Y::Sequence(seq) => J::Array(
            seq.into_iter()
                .map(yaml_to_json)
                .collect::<Result<_, _>>()?,
        ),
        Y::Mapping(m) => {
            let mut obj = serde_json::Map::with_capacity(m.len());
            for (k, v) in m {
                let key = match k {
                    Y::String(s) => s,
                    Y::Number(n) => n.to_string(),
                    Y::Bool(b) => b.to_string(),
                    other => {
                        return Err(ParseError::new(format!(
                            "unsupported yaml mapping key: {other:?}"
                        )))
                    }
                };
                obj.insert(key, yaml_to_json(v)?);
            }
            J::Object(obj)
        }
        Y::Tagged(t) => yaml_to_json(t.value)?,
    })
}
