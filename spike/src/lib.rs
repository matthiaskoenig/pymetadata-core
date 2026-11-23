//! Spike: throwaway. Lazy in-memory OMEX reader/writer plus a wasm surface.

pub mod wasm;

use std::io::{Cursor, Read, Write};

/// A manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub location: String,
    pub format: String,
    pub master: bool,
}

const MANIFEST_NS: &str = "http://identifiers.org/combine.specifications/omex-manifest";

/// Parse a `manifest.xml` into entries.
pub fn parse_manifest(xml: &str) -> Result<Vec<Entry>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "omexManifest" {
        return Err(format!("root is not omexManifest: {}", root.tag_name().name()));
    }
    let mut entries = Vec::new();
    for node in root.children().filter(|n| n.is_element()) {
        if node.tag_name().name() != "content" {
            continue;
        }
        let location = node.attribute("location").ok_or("content without location")?;
        let format = node.attribute("format").ok_or("content without format")?;
        let master = matches!(node.attribute("master"), Some("true") | Some("1"));
        entries.push(Entry {
            location: normalize_location(location),
            format: format.to_string(),
            master,
        });
    }
    let _ = MANIFEST_NS;
    Ok(entries)
}

/// Normalize a location to a `./`-prefixed relative path.
pub fn normalize_location(location: &str) -> String {
    let s = location.replace('\\', "/");
    let s = s.trim_start_matches("./");
    let s = s.trim_start_matches('/');
    if s.is_empty() || s == "." {
        ".".to_string()
    } else {
        format!("./{s}")
    }
}

/// Serialize entries into a `manifest.xml`.
pub fn write_manifest(entries: &[Entry]) -> String {
    let mut out = String::with_capacity(128 + entries.len() * 96);
    out.push_str("<?xml version='1.0' encoding='utf-8'?>\n");
    out.push_str(&format!("<omexManifest xmlns=\"{MANIFEST_NS}\">\n"));
    for e in entries {
        out.push_str(&format!(
            "  <content location=\"{}\" format=\"{}\"{}/>\n",
            e.location,
            e.format,
            if e.master { " master=\"true\"" } else { "" }
        ));
    }
    out.push_str("</omexManifest>\n");
    out
}

/// Read only the manifest of an archive. Nothing else is inflated.
pub fn manifest_only(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    za.by_name("manifest.xml")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    parse_manifest(&xml)
}

/// Inflate every entry into memory. Returns the total inflated size.
pub fn read_all(bytes: &[u8]) -> Result<usize, String> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut total = 0usize;
    let mut buf = Vec::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        buf.clear();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        total += buf.len();
    }
    Ok(total)
}

/// Replace one entry and write a new archive, copying untouched entries as
/// compressed blobs (no re-deflate).
pub fn modify_rawcopy(bytes: &[u8], location: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(bytes.len() + data.len())));
    for i in 0..za.len() {
        let f = za.by_index_raw(i).map_err(|e| e.to_string())?;
        let fname = f.name().map_err(|e| e.to_string())?.to_string();
        if fname == location {
            continue;
        }
        out.raw_copy_file(f).map_err(|e| e.to_string())?;
    }
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    out.start_file(location, opts).map_err(|e| e.to_string())?;
    out.write_all(data).map_err(|e| e.to_string())?;
    Ok(out.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Write a new archive, deflating every entry again. What python does today.
pub fn write_recompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(bytes.len())));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut buf = Vec::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().map_err(|e| e.to_string())?.to_string();
        buf.clear();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        out.start_file(name, opts).map_err(|e| e.to_string())?;
        out.write_all(&buf).map_err(|e| e.to_string())?;
    }
    Ok(out.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Inflate every entry and write it into `dir`, as python's `from_omex` does.
pub fn extract_all(bytes: &[u8], dir: &std::path::Path) -> Result<usize, String> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut total = 0usize;
    let mut buf = Vec::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().map_err(|e| e.to_string())?.to_string();
        let name = name.replace('\\', "/");
        let rel = name.trim_start_matches('/').replace("../", "");
        let dest = dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        buf.clear();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&dest, &buf).map_err(|e| e.to_string())?;
        total += buf.len();
    }
    Ok(total)
}
