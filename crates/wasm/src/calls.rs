//! The calls behind the exports: one pure function per export.
//!
//! Every function here takes owned or borrowed data, answers with owned data and touches no
//! module state, so the whole of this host but the wrappers of `crates/wasm/src/lib.rs` is
//! tested natively by `crates/wasm/tests/calls.rs`. The answer of a call is the bytes the
//! export publishes: the JSON of a wire type, the bytes of an entry or of a written archive, or
//! nothing at all for a call which only changes the archive.

use pymetadata_core::location::suffix;
use pymetadata_core::wire::{ErrorResponse, ManifestResponse, SetEntryRequest, kind};
use pymetadata_core::{Archive, Entry, OmexError, guess_format};

/// The kind of a failure of this host which no [`OmexError`] describes.
///
/// The core produces four kinds, the ones of [`kind`], and these two are the host's: no call of
/// the core answers with either of them. `internal` is a request this host cannot carry out at
/// all, which is a request before `omex_open` and a pointer and length pair which is no range
/// of the memory of the instance.
pub const INTERNAL: &str = "internal";

/// The kind the panic hook reports, which `panic.rs` installs.
pub const PANIC: &str = "panic";

/// What `omex_about` answers: the version of this build and its deflate backend.
///
/// Written with `concat!` of two literals rather than through a `serde_json::Value`, which is
/// the only use this crate would otherwise have for `Value`, its map and its `Display`: a page
/// reads two fields here and a module which travels to a browser does not carry a JSON writer
/// for them. Neither value holds a character JSON escapes, so the string is JSON by
/// construction, and `about_names_the_version_and_the_backend` of `crates/wasm/tests/calls.rs`
/// parses it to keep it that way.
///
/// The backend is `flate2` with `zlib-rs`, the one combination the spike measured at zero
/// imports and 71 ms on the path a writer takes, against 141 ms for `miniz_oxide`
/// (`spike/RESULTS.md`).
const ABOUT: &str = concat!(
    r#"{"version":""#,
    env!("CARGO_PKG_VERSION"),
    r#"","backend":"zlib-rs"}"#
);

/// The level `omex_write` deflates at when it is asked to recompress.
///
/// 9 is what `pymetadata_core.Archive.to_bytes` defaults to in python, so an archive written by
/// the two hosts of this workspace comes out the same way. The export carries no level of its
/// own: a page which wants another one is a signature to change, not a parameter to guess.
const DEFLATE_LEVEL: u32 = 9;

/// The JSON a failure answers with when its own serialization failed.
///
/// Unreachable: [`ErrorResponse`] is a `&'static str` and a `String`, and `serde_json` fails on
/// neither. It is here because the alternative is an `unwrap` on the one path which runs when
/// something already went wrong.
const UNREPORTABLE: &[u8] = br#"{"kind":"internal","message":"the failure could not be reported"}"#;

/// Why a call could not be answered.
#[derive(Debug)]
pub enum CallError {
    /// A failure of the core, reported by the kind string of [`kind`].
    Core(OmexError),
    /// A failure of this host, reported as [`INTERNAL`].
    Internal(String),
}

impl From<OmexError> for CallError {
    /// Every error of the core travels as it is, so that its kind stays the core's.
    fn from(error: OmexError) -> Self {
        Self::Core(error)
    }
}

impl CallError {
    /// The kind string a page routes on: one of the four of [`kind`] or [`INTERNAL`].
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Core(error) => kind(error),
            Self::Internal(_) => INTERNAL,
        }
    }

    /// What went wrong, in the sentence the core or this host wrote.
    pub fn message(&self) -> String {
        match self {
            Self::Core(error) => error.to_string(),
            Self::Internal(message) => message.clone(),
        }
    }

    /// The JSON of [`ErrorResponse`], which is what a failed export publishes.
    pub fn response(&self) -> Vec<u8> {
        response(self.kind(), self.message())
    }
}

/// The JSON of [`ErrorResponse`] for a kind and a message.
///
/// The one place which writes a failure, so the panic hook reports it the way an export does.
pub(crate) fn response(kind: &'static str, message: String) -> Vec<u8> {
    let response = ErrorResponse { kind, message };
    serde_json::to_vec(&response).unwrap_or_else(|_| UNREPORTABLE.to_vec())
}

/// The failure of a call which needs an archive before one was opened.
pub(crate) fn no_archive() -> CallError {
    CallError::Internal(
        "no archive is open: call omex_new for an empty one, or write the bytes of one into the \
         allocation of omex_alloc and call omex_open, before this"
            .to_string(),
    )
}

/// A request this host cannot read as text.
fn malformed(message: String) -> CallError {
    CallError::Core(OmexError::Malformed(message))
}

/// The location a request names, which the ABI carries as UTF-8 bytes.
fn location_of(bytes: &[u8]) -> Result<&str, CallError> {
    std::str::from_utf8(bytes)
        .map_err(|error| malformed(format!("the location is no UTF-8: {error}")))
}

/// Open the archive in `bytes`, which is the request of `omex_open`.
///
/// The archive keeps the budget of the core for how far one member may inflate
/// ([`pymetadata_core::zip::MAX_INFLATED_SIZE`]), the same as the python host: a smaller one
/// here would refuse an entry a page has the memory for, and the export carries no way for a
/// page to raise it again.
///
/// # Errors
///
/// Those of [`Archive::from_bytes`]: bytes which are no zip container, no archive or no
/// manifest are [`OmexError::Malformed`].
pub fn open(bytes: &[u8]) -> Result<Archive, CallError> {
    Ok(Archive::from_bytes(bytes)?)
}

/// The entries of the archive, as the JSON of [`ManifestResponse`].
///
/// # Errors
///
/// [`CallError::Internal`] if the entries cannot be written as JSON, which no entry of an
/// archive the core opened is.
pub fn manifest(archive: &Archive) -> Result<Vec<u8>, CallError> {
    let response = ManifestResponse {
        entries: archive.manifest().entries().to_vec(),
    };
    serde_json::to_vec(&response).map_err(|error| {
        CallError::Internal(format!(
            "the manifest could not be written as JSON: {error}"
        ))
    })
}

/// The inflated bytes of the entry at `location`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the location is no UTF-8, and the errors of
/// [`Archive::entry_bytes`]: `./manifest.xml` is [`OmexError::Refused`], since it is written
/// from the entries and `omex_manifest` is how a page reads it, and a location the archive
/// holds no bytes for is [`OmexError::NotFound`].
pub fn entry(archive: &mut Archive, location: &[u8]) -> Result<Vec<u8>, CallError> {
    let location = location_of(location)?;
    Ok(archive.entry_bytes(location)?.to_vec())
}

/// Add an entry with its bytes, or replace the entry of the same location.
///
/// The request is the JSON of [`SetEntryRequest`] and the `len` content bytes behind it, with
/// nothing in between: the header ends where its closing brace is, which is where the bytes
/// start. An absent `format` is guessed from the suffix of the location and the first bytes of
/// the content, as [`guess_format`] does it for the python host.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the header is no [`SetEntryRequest`], e.g. because it carries a
/// key the type does not know, or if the request does not carry the `len` bytes it declares, and
/// the errors of [`Entry::new`] and [`Archive::set_entry`]: a location outside of the archive,
/// `.` and `./manifest.xml` are [`OmexError::Refused`]. The archive is unchanged unless the
/// whole of it succeeded.
pub fn set_entry(archive: &mut Archive, request: &[u8]) -> Result<Vec<u8>, CallError> {
    let (header, content) = split(request)?;
    let format = match &header.format {
        Some(format) => format.clone(),
        None => guess_format(suffix(&header.location), content).to_string(),
    };
    let entry = Entry::new(&header.location, &format, header.master)?;
    archive.set_entry(entry, content.to_vec())?;
    Ok(Vec::new())
}

/// Remove the entry at `location` and its bytes.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the location is no UTF-8, the errors of
/// [`Archive::remove_entry`], which is [`OmexError::Refused`] for a location no archive can
/// hold, and [`OmexError::NotFound`] if the archive has no such entry, which is also what `.`
/// and `./manifest.xml` are: the two entries the specification requires are not removed. The
/// archive answers `None` for an entry which was not there and the failure is this call's, so
/// that a page which does not care whether there was one ignores the return code of the export.
pub fn remove_entry(archive: &mut Archive, location: &[u8]) -> Result<Vec<u8>, CallError> {
    let location = location_of(location)?;
    if archive.remove_entry(location)?.is_none() {
        return Err(CallError::Core(OmexError::NotFound(format!(
            "no entry at '{location}' to remove"
        ))));
    }
    Ok(Vec::new())
}

/// The bytes of the written archive.
///
/// Without `recompress` every entry which was not replaced is copied as the compressed blob it
/// is, which is the write a page makes after it changed one file; with it every entry is
/// deflated again at level 9, which is what the python host defaults to.
///
/// # Errors
///
/// Those of [`Archive::to_bytes`] and [`Archive::to_bytes_recompressed`].
pub fn write(archive: &mut Archive, recompress: bool) -> Result<Vec<u8>, CallError> {
    let bytes = if recompress {
        archive.to_bytes_recompressed(DEFLATE_LEVEL)?
    } else {
        archive.to_bytes()?
    };
    Ok(bytes)
}

/// What this build is: its version and its deflate backend, the `ABOUT` constant.
///
/// A page reads this once, before it sends anything, and refuses a module whose version it does
/// not know rather than guessing at its answers. The answer is a constant, so it cannot fail and
/// the export always returns 0.
pub fn about() -> Vec<u8> {
    ABOUT.as_bytes().to_vec()
}

/// The header of a set-entry request and the content bytes behind it.
///
/// The length is checked against what the request carries, so a page which wrote a shorter
/// request than its header declares is told so instead of storing the bytes of whatever follows
/// in the input buffer.
fn split(request: &[u8]) -> Result<(SetEntryRequest, &[u8]), CallError> {
    let mut headers = serde_json::Deserializer::from_slice(request).into_iter::<SetEntryRequest>();
    let header = match headers.next() {
        Some(Ok(header)) => header,
        Some(Err(error)) => {
            return Err(malformed(format!(
                "the request does not start with the JSON header of an entry: {error}"
            )));
        }
        None => return Err(malformed("the request carries no header".to_string())),
    };
    // where the header ended is where its bytes start, which is the one thing the stream
    // deserializer is here for: a JSON object is self delimiting and the bytes behind it are no
    // JSON at all
    let at = headers.byte_offset();
    let content = request.get(at..).unwrap_or(&[]);
    if content.len() != header.len {
        return Err(malformed(format!(
            "the header declares {} content bytes and the request carries {}",
            header.len,
            content.len()
        )));
    }
    Ok((header, content))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // Every test of this binary takes the gate of `crate::tests`, these two included, although
    // they drive no export: the panic hook is the process's once any export installed it, so a
    // failing assertion here would publish its panic into the output buffer of a test which
    // reads it. The gate is what keeps the module state of one call at a time.

    /// The header of a request ends at its closing brace and the content follows it.
    #[test]
    fn a_request_is_the_header_and_the_bytes_behind_it() {
        let _gate = crate::tests::gate();
        let mut request = br#"{"location":"./a.bin","len":3}"#.to_vec();
        request.extend_from_slice(&[0x00, 0xff, 0x7b]);
        let (header, content) = split(&request).expect("the header");
        assert_eq!(header.location, "./a.bin");
        assert_eq!(header.format, None);
        assert!(!header.master);
        assert_eq!(content, &[0x00, 0xff, 0x7b]);
        // whitespace in front of the header and no content at all
        let (header, content) = split(br#"  {"location":"./a.bin","len":0}"#).expect("the header");
        assert_eq!(header.len, 0);
        assert!(content.is_empty());
    }

    /// The two kinds of this host are not the four of the core.
    #[test]
    fn the_host_kinds_are_its_own() {
        let _gate = crate::tests::gate();
        assert_eq!(no_archive().kind(), INTERNAL);
        assert_eq!(
            CallError::from(OmexError::NotFound("x".into())).kind(),
            "not_found"
        );
        assert_eq!(malformed("x".to_string()).kind(), "malformed");
        let reported = response(PANIC, "panicked at 'x'".to_string());
        let answer: serde_json::Value = serde_json::from_slice(&reported).expect("the JSON");
        assert_eq!(answer["kind"], json!("panic"));
        assert_eq!(answer["message"], json!("panicked at 'x'"));
        // a message with a quote or a newline arrives as the sentence it is
        let reported = response(INTERNAL, "a \"quoted\" word\nand a line".to_string());
        let answer: serde_json::Value = serde_json::from_slice(&reported).expect("the JSON");
        assert_eq!(answer["message"], json!("a \"quoted\" word\nand a line"));
    }
}
