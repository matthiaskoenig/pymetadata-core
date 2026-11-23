# pymetadata-core

Lazy reading and writing of COMBINE archives (OMEX, a zip with a `manifest.xml`), as a Rust core with a python binding. Opening an archive reads the manifest and nothing else, an entry is inflated when it is asked for, and writing copies every untouched entry as the compressed blob it already is.

```python
from pymetadata_core import Archive, lookup_format

with open("model.omex", "rb") as f:
    archive = Archive.from_bytes(f.read())

print(archive.entries)  # [(location, format, master), ...]
sbml = archive.entry_bytes("./model.xml")

archive.set_entry("data/notes.txt", lookup_format("txt"), False, b"hello")
data = archive.to_bytes()  # only the new entry is deflated
```

Errors are raised as `pymetadata_core.OmexError`, a subclass of `ValueError`.

Inflating an entry, `set_entry` and writing the archive run without the GIL, so another thread can work meanwhile. An `Archive` is in use by one call at a time: a second thread which uses it meanwhile gets `pymetadata_core.ArchiveBusyError`, a `RuntimeError`. Use one `Archive` per thread.

The design is in [`docs/design/omex-core.md`](https://github.com/matthiaskoenig/pymetadata-core/blob/develop/docs/design/omex-core.md).
