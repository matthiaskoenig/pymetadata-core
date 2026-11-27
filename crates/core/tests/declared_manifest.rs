//! Hosts can preserve application manifest ordering without reparsing XML.
use pymetadata_core::manifest::Manifest;
#[test]
fn declared_entries_preserve_duplicate_locations_and_per_declaration_metadata() {
    let xml = r#"<omexManifest><content location="a.xml" format="sbml" master="false"/><content location="./a.xml" format="csv" master="YES"/></omexManifest>"#;
    let entries = Manifest::parse_declared(xml).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].location, "./a.xml");
    assert_eq!(entries[1].location, "./a.xml");
    assert_eq!(entries[0].format, "sbml");
    assert_eq!(entries[1].format, "csv");
    assert!(!entries[0].master);
    assert!(entries[1].master);
    assert_eq!(Manifest::parse(xml).unwrap().entries().len(), 3);
    assert!(
        Manifest::parse_declared(
            r#"<omexManifest><content location="../a" format="sbml"/></omexManifest>"#
        )
        .is_err()
    );
}
