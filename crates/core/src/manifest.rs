//! The manifest of a COMBINE archive.
//!
//! The `manifest.xml` lists the files of the archive, each with its location, its format and
//! whether it is the master file a tool opens first.

use std::collections::HashMap;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::error::OmexError;
use crate::formats::lookup_format;
use crate::location::normalize_location;

/// The location of the archive itself.
const ARCHIVE_LOCATION: &str = ".";
/// The location of the manifest.
const MANIFEST_LOCATION: &str = "./manifest.xml";
/// The XML namespace of the manifest.
const NAMESPACE: &str = "http://identifiers.org/combine.specifications/omex-manifest";

/// A single file of the archive, as listed in the `manifest.xml`.
///
/// Build an entry from untrusted input with [`Entry::new`]. A deserialized `Entry` carries the
/// location as written, it is normalized and checked when [`Manifest::add`] takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Location of the file, relative and starting with `./`, e.g. `./models/model.xml`.
    pub location: String,
    /// Format URI of the file.
    pub format: String,
    /// Marks the entry a tool should open first.
    pub master: bool,
}

impl Entry {
    /// Build an entry, normalizing the location, see [`normalize_location`].
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the location is empty, absolute or points outside of the
    /// archive, or if the location or the format holds a character XML 1.0 forbids.
    pub fn new(location: &str, format: &str, master: bool) -> Result<Entry, OmexError> {
        let location = normalize_location(location)?;
        check_xml_chars(&location)?;
        check_xml_chars(format)?;
        Ok(Entry {
            location,
            format: format.to_string(),
            master,
        })
    }
}

/// Content of the `manifest.xml`, i.e. the entries of an archive in order.
///
/// The manifest is keyed by location and always holds the two entries the specification
/// requires: the archive itself (`.`) and the manifest (`./manifest.xml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
}

impl Default for Manifest {
    /// A manifest with the two required entries.
    fn default() -> Self {
        let mut manifest = Manifest {
            entries: Vec::new(),
            index: HashMap::new(),
        };
        manifest.insert(required_entry(ARCHIVE_LOCATION, "omex"));
        manifest.insert(required_entry(MANIFEST_LOCATION, "omex_manifest"));
        manifest
    }
}

/// The required entry for a location, with the format of the given key.
fn required_entry(location: &str, format_key: &str) -> Entry {
    Entry {
        location: location.to_string(),
        format: lookup_format(format_key).to_string(),
        master: false,
    }
}

/// Refuse a character XML 1.0 forbids, so that a written manifest can always be read back.
fn check_xml_chars(value: &str) -> Result<(), OmexError> {
    match value.chars().find(|c| {
        matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
    }) {
        Some(c) => Err(OmexError::Refused(format!(
            "'{}' holds the character U+{:04X} which XML forbids",
            value.escape_debug(),
            u32::from(c)
        ))),
        None => Ok(()),
    }
}

/// Read the `master` attribute the way the python reference (pydantic) reads a bool.
fn parse_master(value: &str) -> Result<bool, OmexError> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Ok(false),
        _ => Err(OmexError::Malformed(format!(
            "master must be a boolean, but is '{value}'"
        ))),
    }
}

/// Escape a value for a double quoted XML attribute.
fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // an XML parser normalizes raw whitespace in an attribute to a space
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

impl Manifest {
    /// Parse the content of a `manifest.xml`.
    ///
    /// The entries keep the document order. The two required entries come first when the
    /// document does not hold them. The locations are normalized, so several spellings of a
    /// location are one entry, the last one wins.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the text is not XML, the root element is not `omexManifest`,
    /// a `content` element lacks its `location` or `format` or has a `master` which is not a
    /// boolean; [`OmexError::Refused`] for a location outside of the archive.
    pub fn parse(xml: &str) -> Result<Self, OmexError> {
        Ok(Self::from_declared(Self::parse_declared(xml)?))
    }

    /// The entries of a `manifest.xml` as the document declares them, one per `content`
    /// element in document order, with the errors of [`Manifest::parse`].
    pub(crate) fn parse_declared(xml: &str) -> Result<Vec<Entry>, OmexError> {
        let document = roxmltree::Document::parse(xml)
            .map_err(|e| OmexError::Malformed(format!("manifest is not valid XML: {e}")))?;
        let root = document.root_element();
        if root.tag_name().name() != "omexManifest" {
            return Err(OmexError::Malformed(format!(
                "root element of the manifest must be omexManifest, but is '{}'",
                root.tag_name().name()
            )));
        }
        let mut parsed = Vec::new();
        for content in root
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "content")
        {
            let attribute = |name: &str| {
                content.attribute(name).ok_or_else(|| {
                    OmexError::Malformed(format!("content element without {name} attribute"))
                })
            };
            parsed.push(Entry::new(
                attribute("location")?,
                attribute("format")?,
                content
                    .attribute("master")
                    .map_or(Ok(false), parse_master)?,
            )?);
        }
        Ok(parsed)
    }

    /// The manifest of declared entries, with the two required ones first when they are missing.
    pub(crate) fn from_declared(parsed: Vec<Entry>) -> Self {
        let mut manifest = Manifest {
            entries: Vec::new(),
            index: HashMap::new(),
        };
        for (location, format_key) in [
            (ARCHIVE_LOCATION, "omex"),
            (MANIFEST_LOCATION, "omex_manifest"),
        ] {
            if !parsed.iter().any(|e| e.location == location) {
                manifest.insert(required_entry(location, format_key));
            }
        }
        for entry in parsed {
            manifest.insert(entry);
        }
        manifest
    }

    /// Serialize the manifest to the content of a `manifest.xml`.
    pub fn to_xml(&self) -> String {
        let mut xml = String::from("<?xml version='1.0' encoding='utf-8'?>\n");
        // writing to a String cannot fail
        let _ = writeln!(xml, "<omexManifest xmlns=\"{NAMESPACE}\">");
        for entry in &self.entries {
            let master = if entry.master { " master=\"true\"" } else { "" };
            let _ = writeln!(
                xml,
                "  <content location=\"{}\" format=\"{}\"{master}/>",
                escape_attribute(&entry.location),
                escape_attribute(&entry.format),
            );
        }
        xml.push_str("</omexManifest>\n");
        xml
    }

    /// The entries in order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry for a location, in any spelling `normalize_location` accepts.
    pub fn get(&self, location: &str) -> Option<&Entry> {
        let location = normalize_location(location).ok()?;
        self.index.get(&location).and_then(|&i| self.entries.get(i))
    }

    /// Add an entry; the entry of the same location is replaced in place.
    ///
    /// The location of the entry is normalized first.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] as for [`Entry::new`].
    pub fn add(&mut self, mut entry: Entry) -> Result<(), OmexError> {
        entry.location = normalize_location(&entry.location)?;
        check_xml_chars(&entry.location)?;
        check_xml_chars(&entry.format)?;
        self.insert(entry);
        Ok(())
    }

    /// Insert an entry with a normalized location, replacing the one of the same location.
    fn insert(&mut self, entry: Entry) {
        match self.index.get(&entry.location) {
            Some(&i) => {
                if let Some(slot) = self.entries.get_mut(i) {
                    *slot = entry;
                }
            }
            None => {
                self.index
                    .insert(entry.location.clone(), self.entries.len());
                self.entries.push(entry);
            }
        }
    }

    /// Remove the entry for a location and return it.
    ///
    /// `.` and `./manifest.xml` are required and never removed, nor is an unknown location, all
    /// of which give `None`. A location which cannot be normalized is no location of a manifest
    /// and gives `None` too; it never reaches here through [`crate::Archive::remove_entry`],
    /// which refuses it with the error every other call of an archive refuses it with.
    pub fn remove(&mut self, location: &str) -> Option<Entry> {
        let location = normalize_location(location).ok()?;
        if location == ARCHIVE_LOCATION || location == MANIFEST_LOCATION {
            return None;
        }
        let i = self.index.remove(&location)?;
        let entry = self.entries.remove(i);
        for slot in self.index.values_mut() {
            if *slot > i {
                *slot -= 1;
            }
        }
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::OmexError;

    fn doc(body: &str) -> String {
        format!("<omexManifest xmlns=\"{NAMESPACE}\">{body}</omexManifest>")
    }

    /// A new manifest holds the two entries the specification requires.
    #[test]
    fn default_holds_the_two_required_entries() {
        let m = Manifest::default();
        assert_eq!(m.entries().len(), 2);
        assert_eq!(
            m.get(".").unwrap().format,
            "http://identifiers.org/combine.specifications/omex"
        );
        assert_eq!(
            m.get("./manifest.xml").unwrap().format,
            "http://identifiers.org/combine.specifications/omex-manifest"
        );
    }

    /// Every spelling of a location is the same entry.
    #[test]
    fn parses_locations_in_every_spelling_to_one_entry() {
        let xml = doc(r#"<content location=".\models\model.xml" format="a"/>
               <content location="models/model.xml" format="b"/>
               <content location="./models/model.xml" format="c"/>"#);
        let m = Manifest::parse(&xml).unwrap();
        assert_eq!(m.entries().len(), 3);
        assert_eq!(m.get("models/model.xml").unwrap().format, "c");
        assert_eq!(m.get(".\\models\\model.xml").unwrap().format, "c");
    }

    /// The master flag reads the spellings pydantic reads, absent is false.
    #[test]
    fn parses_master() {
        let xml = doc(r#"<content location="a" format="f" master="true"/>
               <content location="b" format="f" master="1"/>
               <content location="c" format="f"/>
               <content location="d" format="f" master="false"/>"#);
        let m = Manifest::parse(&xml).unwrap();
        let master = |l: &str| m.get(l).unwrap().master;
        assert!(master("a") && master("b"));
        assert!(!master("c") && !master("d"));
        let bad = doc(r#"<content location="a" format="f" master="maybe"/>"#);
        assert!(matches!(
            Manifest::parse(&bad),
            Err(OmexError::Malformed(_))
        ));
    }

    /// Only an `omexManifest` document is a manifest.
    #[test]
    fn refuses_a_wrong_root_element() {
        assert!(matches!(
            Manifest::parse("<other/>"),
            Err(OmexError::Malformed(_))
        ));
        assert!(matches!(
            Manifest::parse("not xml"),
            Err(OmexError::Malformed(_))
        ));
    }

    /// A content element needs both attributes.
    #[test]
    fn refuses_content_without_location_or_format() {
        let no_location = doc(r#"<content format="f"/>"#);
        let no_format = doc(r#"<content location="a"/>"#);
        assert!(matches!(
            Manifest::parse(&no_location),
            Err(OmexError::Malformed(_))
        ));
        assert!(matches!(
            Manifest::parse(&no_format),
            Err(OmexError::Malformed(_))
        ));
    }

    /// An escaping location is refused, not stored.
    #[test]
    fn refuses_a_location_outside_of_the_archive() {
        let xml = doc(r#"<content location="../x" format="f"/>"#);
        assert!(matches!(Manifest::parse(&xml), Err(OmexError::Refused(_))));
    }

    /// Document order is kept and absent required entries come first.
    #[test]
    fn keeps_the_document_order_and_prepends_the_required_entries() {
        let xml = doc(r#"<content location="z" format="f"/><content location="a" format="f"/>"#);
        let m = Manifest::parse(&xml).unwrap();
        let locations: Vec<&str> = m.entries().iter().map(|e| e.location.as_str()).collect();
        assert_eq!(locations, [".", "./manifest.xml", "./z", "./a"]);
        let xml = doc(
            r#"<content location="./manifest.xml" format="m"/><content location="a" format="f"/>"#,
        );
        let m = Manifest::parse(&xml).unwrap();
        let locations: Vec<&str> = m.entries().iter().map(|e| e.location.as_str()).collect();
        assert_eq!(locations, [".", "./manifest.xml", "./a"]);
        assert_eq!(m.get("./manifest.xml").unwrap().format, "m");
    }

    /// Serializing and parsing again gives the same manifest.
    #[test]
    fn roundtrips_through_xml() {
        let mut m = Manifest::default();
        m.add(Entry {
            location: "./a.xml".into(),
            format: "f".into(),
            master: true,
        })
        .unwrap();
        m.add(Entry {
            location: "./b.xml".into(),
            format: "g".into(),
            master: false,
        })
        .unwrap();
        let xml = m.to_xml();
        assert!(xml.starts_with("<?xml version='1.0' encoding='utf-8'?>"));
        assert!(xml.contains("master=\"true\""));
        assert_eq!(xml.matches("master=").count(), 1);
        assert_eq!(Manifest::parse(&xml).unwrap(), m);
    }

    /// Special characters of an attribute survive byte for byte.
    #[test]
    fn escapes_the_attributes() {
        let mut m = Manifest::default();
        let location = "./a&b \"c\" <d> \t\n.xml";
        m.add(Entry {
            location: location.into(),
            format: "x&\"<>".into(),
            master: false,
        })
        .unwrap();
        let xml = m.to_xml();
        assert!(xml.contains("&amp;") && xml.contains("&quot;"));
        let back = Manifest::parse(&xml).unwrap();
        assert_eq!(back.get(location).unwrap().format, "x&\"<>");
        assert_eq!(back, m);
    }

    /// Adding a known location replaces the entry where it is.
    #[test]
    fn add_replaces_the_entry_for_a_location() {
        let mut m = Manifest::default();
        let entry = |format: &str| Entry {
            location: "./a".into(),
            format: format.into(),
            master: false,
        };
        m.add(entry("first")).unwrap();
        m.add(Entry {
            location: "./b".into(),
            format: "b".into(),
            master: false,
        })
        .unwrap();
        m.add(entry("second")).unwrap();
        assert_eq!(m.entries().len(), 4);
        assert_eq!(m.get("./a").unwrap().format, "second");
        assert_eq!(m.entries()[2].location, "./a");
    }

    /// Remove returns the entry, but never a required one.
    #[test]
    fn remove_keeps_the_required_entries() {
        let mut m = Manifest::default();
        m.add(Entry {
            location: "./a".into(),
            format: "f".into(),
            master: false,
        })
        .unwrap();
        m.add(Entry {
            location: "./b".into(),
            format: "f".into(),
            master: false,
        })
        .unwrap();
        assert_eq!(m.remove("a").unwrap().location, "./a");
        assert!(m.get("./a").is_none());
        assert_eq!(m.get("./b").unwrap().location, "./b");
        assert!(m.remove("a").is_none());
        assert!(m.remove(".").is_none());
        assert!(m.remove("./manifest.xml").is_none());
        assert!(m.remove("../x").is_none());
        assert_eq!(m.entries().len(), 3);
    }

    /// A raw location is normalized by `add`.
    #[test]
    fn add_normalizes_the_location() {
        let mut m = Manifest::default();
        m.add(Entry {
            location: "models\\m.xml".into(),
            format: "f".into(),
            master: false,
        })
        .unwrap();
        assert_eq!(m.entries()[2].location, "./models/m.xml");
        assert!(m.get("./models/m.xml").is_some());
        let again = Entry::new("models/m.xml", "g", true).unwrap();
        m.add(again).unwrap();
        assert_eq!(m.entries().len(), 3);
    }

    /// `add` refuses a location outside of the archive and changes nothing.
    #[test]
    fn add_refuses_a_location_outside_of_the_archive() {
        let mut m = Manifest::default();
        let entry = Entry {
            location: "../m.xml".into(),
            format: "f".into(),
            master: false,
        };
        assert!(matches!(m.add(entry), Err(OmexError::Refused(_))));
        assert_eq!(m.entries().len(), 2);
        assert!(Entry::new("/abs", "f", false).is_err());
    }

    /// A character XML forbids is refused, so `to_xml` always reads back.
    #[test]
    fn refuses_a_character_xml_forbids() {
        let Err(OmexError::Refused(message)) = Entry::new("./a\u{1}.xml", "f", false) else {
            panic!("location with U+0001 must be refused");
        };
        assert!(message.contains("U+0001"), "{message}");
        assert!(matches!(
            Entry::new("a", "f\u{B}", false),
            Err(OmexError::Refused(_))
        ));
        let mut m = Manifest::default();
        let raw = Entry {
            location: "./a".into(),
            format: "f\u{1F}".into(),
            master: false,
        };
        assert!(matches!(m.add(raw), Err(OmexError::Refused(_))));
        assert_eq!(m.entries().len(), 2);
        // tab, line feed and carriage return are legal and escaped
        assert!(Entry::new("a", "f\t\n\r", false).is_ok());
    }
}
