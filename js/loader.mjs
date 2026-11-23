/**
 * The loader of the pymetadata-core WebAssembly module: a COMBINE archive opened, changed and
 * written in a browser, a Web Worker or any other JavaScript engine, with no server.
 *
 * This file uses no API of any one host and has no dependency. The module it loads imports
 * nothing, so it is instantiated with an empty import object.
 *
 * Two rules of the module shape everything below.
 *
 * - The linear memory grows, and when it does `memory.buffer` is replaced and every typed array
 *   over the old buffer is detached. So no view is ever kept: `view()` makes one over the
 *   current buffer, and every use of it comes right after the call it belongs to and before the
 *   next call into the module.
 * - A trap leaves the instance spent. The module has no unwinding: a panic runs its hook, which
 *   writes a message into the output buffer, and then traps; an allocation it cannot serve
 *   traps without a message. After either the module's state is unknown, so the runtime
 *   refuses every later call with the kind `spent` and a page creates a new runtime.
 *
 * @module
 */

/** The kind of the error of a call which trapped without leaving a message of the module. */
const TRAP = "trap";

/** The kind of the error of every call after a trap, which the runtime refuses itself. */
const SPENT = "spent";

/** The kind the module's panic hook writes. */
const PANIC = "panic";

/** The kind of a failure the module cannot carry out, which the loader also uses for its own. */
const INTERNAL = "internal";

/** The exports the loader calls, as the module declares them. */
const FUNCTIONS = [
  "omex_alloc",
  "omex_output_ptr",
  "omex_output_len",
  "omex_new",
  "omex_open",
  "omex_manifest",
  "omex_entry",
  "omex_set_entry",
  "omex_remove_entry",
  "omex_write",
  "omex_about",
];

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * The error of a failed call.
 *
 * `kind` is what a page routes on. The module reports `malformed`, `refused`, `unsupported`,
 * `not_found` and `password` for the archive, `internal` for a request it cannot carry out and
 * `panic` for a bug of the build. A `password` is a refusal whose message carries the `refused`
 * prefix, so the kind is what separates a member a caller may not read from every other refusal. The loader adds `trap`, a trap which left no message, and `spent`, a call
 * on an instance which trapped before.
 */
export class OmexError extends Error {
  /**
   * @param {string} kind The kind of the failure.
   * @param {string} message What went wrong, for a person.
   */
  constructor(kind, message) {
    super(message);
    this.name = "OmexError";
    this.kind = kind;
  }
}

/**
 * Bytes of a page, as a `Uint8Array` over the same memory.
 *
 * @param {unknown} value A `Uint8Array`, another typed array or `DataView`, or an `ArrayBuffer`.
 * @param {string} what The name of the argument, for the message.
 * @returns {Uint8Array}
 */
function toBytes(value, what) {
  if (value instanceof Uint8Array) return value;
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  throw new TypeError(`${what} must be a Uint8Array or an ArrayBuffer`);
}

/**
 * Check that an argument is a string.
 *
 * @param {unknown} value
 * @param {string} what
 * @returns {string}
 */
function toText(value, what) {
  if (typeof value !== "string") throw new TypeError(`${what} must be a string`);
  return value;
}

/**
 * Instantiate the module and wrap its exports.
 *
 * Asynchronous because instantiating is: the runtime it resolves to is synchronous, one call
 * after the other. An archive is one object, so a runtime holds one archive at a time, and a
 * page which wants two instantiates two runtimes, one per Web Worker if it wants them in
 * parallel. A call must not be made from inside another call.
 *
 * @param {{module: WebAssembly.Module | BufferSource}} options The compiled module, or the bytes
 *   of `runtime/pymetadata-core.wasm`.
 * @returns {Promise<Runtime>}
 */
export async function createRuntime({ module }) {
  const result = await WebAssembly.instantiate(module, {});
  return new Runtime((result.instance ?? result).exports);
}

/**
 * One instance of the module and the archive it holds.
 *
 * Make one with {@link createRuntime}. The constructor checks the exports itself, so there is
 * one way in whichever way a runtime is made.
 */
export class Runtime {
  #exports;
  #spent = false;

  /** @param {WebAssembly.Exports} exports The exports of an instance of the module. */
  constructor(exports) {
    for (const name of ["memory", ...FUNCTIONS]) {
      if (exports?.[name] === undefined) {
        throw new TypeError(`the module does not export ${name}: it is not pymetadata-core`);
      }
    }
    this.#exports = exports;
  }

  /**
   * Whether the instance trapped, after which every call but this getter throws `spent`.
   *
   * @returns {boolean}
   */
  get spent() {
    return this.#spent;
  }

  /**
   * Create an empty archive, dropping the one open before.
   *
   * The archive holds the two entries a COMBINE archive must have, `.` and `./manifest.xml`,
   * and no member: `setEntry` adds the members and `write` gives the bytes of the `.omex` file.
   * This is how a page builds an archive without opening one first, and it is `Archive()` of the
   * python package.
   */
  create() {
    this.#call(() => {
      this.#finish(this.#exports.omex_new());
    });
  }

  /**
   * Open the archive in `bytes`, dropping the one open before.
   *
   * A failed open leaves no archive open. The module keeps its own copy of the bytes, so the
   * array stays the page's.
   *
   * @param {Uint8Array | ArrayBuffer} bytes The archive.
   */
  open(bytes) {
    const archive = toBytes(bytes, "the archive");
    this.#call(() => {
      this.#stage(archive);
      this.#finish(this.#exports.omex_open());
    });
  }

  /**
   * The entries of the open archive, in the order of its manifest.
   *
   * @returns {{location: string, format: string, master: boolean}[]}
   */
  manifest() {
    return this.#call(() => JSON.parse(decoder.decode(this.#answer(this.#exports.omex_manifest()))))
      .entries;
  }

  /**
   * The inflated bytes of one member, a copy which the page owns.
   *
   * @param {string} location The location in the manifest, `./model.xml` or `model.xml`.
   * @returns {Uint8Array}
   */
  entry(location) {
    const request = encoder.encode(toText(location, "the location"));
    return this.#call(() => {
      const ptr = this.#stage(request);
      return this.#answer(this.#exports.omex_entry(ptr, request.length));
    });
  }

  /**
   * Add a member, or replace the member of the same location.
   *
   * @param {string} location Where the member goes in the archive.
   * @param {string | null} format The format URI, or `null` to guess it from the location.
   * @param {boolean} master Whether the entry is the master file.
   * @param {Uint8Array | ArrayBuffer} bytes The content.
   */
  setEntry(location, format, master, bytes) {
    const content = toBytes(bytes, "the content");
    // exactly the four keys of the request: the module refuses any other
    const header = encoder.encode(
      JSON.stringify({
        location: toText(location, "the location"),
        format: format === null || format === undefined ? null : toText(format, "the format"),
        master: Boolean(master),
        len: content.length,
      }),
    );
    const request = new Uint8Array(header.length + content.length);
    request.set(header);
    request.set(content, header.length);
    this.#call(() => {
      const ptr = this.#stage(request);
      this.#finish(this.#exports.omex_set_entry(ptr, request.length));
    });
  }

  /**
   * Remove a member and its entry.
   *
   * A location the archive does not hold throws `not_found`, which is also what `.` and
   * `./manifest.xml` are, since a COMBINE archive keeps those two; the python package answers
   * `False` for the same calls. A page which does not care whether there was an entry catches
   * that kind.
   *
   * @param {string} location The location in the manifest.
   */
  removeEntry(location) {
    const request = encoder.encode(toText(location, "the location"));
    this.#call(() => {
      const ptr = this.#stage(request);
      this.#finish(this.#exports.omex_remove_entry(ptr, request.length));
    });
  }

  /**
   * Write the archive.
   *
   * @param {{recompress?: boolean}} [options] `recompress` deflates every member again; by
   *   default the members which were not replaced are copied as the compressed blobs they are.
   * @returns {Uint8Array} The archive, a copy which the page owns.
   */
  write(options = {}) {
    if (options === null || typeof options !== "object") {
      throw new TypeError("write takes an options object such as { recompress: true }");
    }
    const recompress = Boolean(options.recompress);
    return this.#call(() => this.#answer(this.#exports.omex_write(recompress ? 1 : 0)));
  }

  /**
   * The version and the deflate backend of the module.
   *
   * @returns {{version: string, backend: string}}
   */
  about() {
    return this.#call(() => JSON.parse(decoder.decode(this.#answer(this.#exports.omex_about()))));
  }

  /**
   * Run one operation on the instance, and turn a trap into a spent runtime.
   *
   * The operation starts with `omex_about`, which cannot fail and replaces the output of the
   * call before it: a trap leaves the output buffer as it was, so without this a trap which
   * says nothing would be read as whatever the call before it answered, which can be bytes a
   * member of an archive chose. The one window is a trap of that `omex_about` itself, which the
   * ABI gives no way to clear the buffer before: the output is then whatever the call before
   * left, so it is not read, and the error is a plain `trap` even when the module panicked.
   *
   * Any exception of an operation spends the instance, which is the conservative reading: a
   * `WebAssembly.RuntimeError` is a trap, anything else is a failure of the loader around the
   * module and says so, since the module's state is unknown either way.
   *
   * @template T
   * @param {() => T} operation
   * @returns {T}
   */
  #call(operation) {
    if (this.#spent) {
      throw new OmexError(
        SPENT,
        "the instance trapped on an earlier call and is spent; create a new runtime",
      );
    }
    let reset = false;
    try {
      this.#exports.omex_about();
      reset = true;
      return operation();
    } catch (cause) {
      if (cause instanceof OmexError) throw cause;
      throw this.#failed(cause, reset);
    }
  }

  /**
   * The error of a call which threw, and the end of this instance.
   *
   * @param {unknown} cause What was thrown.
   * @param {boolean} reset Whether the output buffer was replaced by `omex_about` before it.
   * @returns {OmexError}
   */
  #failed(cause, reset) {
    this.#spent = true;
    const reason = cause instanceof Error ? `${cause.name}: ${cause.message}` : String(cause);
    if (!(cause instanceof WebAssembly.RuntimeError)) {
      return new OmexError(
        INTERNAL,
        `the call failed with ${reason}, which is no trap of the module with a message: for instance a ` +
          "stack overflow or a memory which could not grow, or a failure of the loader; the " +
          "module's state is unknown, so " +
          "the instance is spent, create a new runtime",
      );
    }
    // the output buffer is only trusted when this call replaced it first
    if (reset) {
      try {
        const output = JSON.parse(decoder.decode(this.#output()));
        // the panic hook wrote this just before the trap; `#call` made the output the answer of
        // `omex_about` first, so nothing else can be here
        if (output.kind === PANIC) return new OmexError(PANIC, String(output.message));
      } catch {
        // no readable message of the module: the trap was an abort, or the instance is unusable
      }
    }
    return new OmexError(
      TRAP,
      `the instance trapped (${reason}) and left no message, for instance an allocation it ` +
        "could not serve; it is spent, create a new runtime",
    );
  }

  /**
   * A view of the linear memory, over the buffer it has right now.
   *
   * The only way the loader reads or writes the memory, and never kept: any call into the module
   * can grow the memory and detach it.
   *
   * @param {number} ptr
   * @param {number} length
   * @returns {Uint8Array}
   */
  #view(ptr, length) {
    return new Uint8Array(this.#exports.memory.buffer, ptr, length);
  }

  /**
   * Write a request into the input buffer of the module and give its address.
   *
   * @param {Uint8Array} bytes
   * @returns {number} The address, 0 for no bytes.
   */
  #stage(bytes) {
    const ptr = this.#exports.omex_alloc(bytes.length);
    if (bytes.length === 0) return 0;
    if (ptr === 0) {
      throw new OmexError(
        INTERNAL,
        `the instance cannot reserve ${bytes.length} bytes for the request`,
      );
    }
    // the allocation may have grown the memory: the view comes after it
    this.#view(ptr, bytes.length).set(bytes);
    return ptr;
  }

  /**
   * A copy of the output of the last call.
   *
   * @returns {Uint8Array}
   */
  #output() {
    const length = this.#exports.omex_output_len();
    if (length === 0) return new Uint8Array(0);
    return this.#view(this.#exports.omex_output_ptr(), length).slice();
  }

  /**
   * The output of a call which answered, or its error.
   *
   * @param {number} code The return code of the export.
   * @returns {Uint8Array}
   */
  #answer(code) {
    if (code === 0) return this.#output();
    throw this.#failure();
  }

  /**
   * Check the code of a call which has no answer but success.
   *
   * @param {number} code The return code of the export.
   */
  #finish(code) {
    if (code !== 0) throw this.#failure();
  }

  /**
   * The error the module put in the output buffer.
   *
   * @returns {OmexError}
   */
  #failure() {
    let response;
    try {
      response = JSON.parse(decoder.decode(this.#output()));
    } catch {
      response = undefined;
    }
    if (typeof response?.kind !== "string") {
      return new OmexError(INTERNAL, "the module failed and its error output is unreadable");
    }
    // a panic always traps after its hook, so an answer of this kind is not expected; if one
    // arrives the instance is not trusted either
    if (response.kind === PANIC) this.#spent = true;
    return new OmexError(response.kind, String(response.message));
  }
}
