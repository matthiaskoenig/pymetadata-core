import { readFile } from 'node:fs/promises';

const path = process.argv[2];
const bytes = await readFile(path);
const { instance } = await WebAssembly.instantiate(bytes, {});
const e = instance.exports;
const mem = () => new Uint8Array(e.memory.buffer);

// payload: a real SBML file, which is what archives hold
const xml = await readFile(process.argv[3]);
const payload = new Uint8Array(xml);

function put(data) {
  const ptr = e.input_alloc(data.length);
  mem().set(data, ptr);
  return ptr;
}

const ptr = put(payload);
const crc = e.crc32(ptr, payload.length);
const dlen = Number(e.deflate_len(ptr, payload.length));
const deflated = mem().slice(e.out_ptr(), e.out_ptr() + dlen);
const dptr = put(deflated);
const ilen = Number(e.inflate_len(dptr, deflated.length));

const rounds = 20;
const t0 = performance.now();
for (let i = 0; i < rounds; i++) { const p = put(payload); e.deflate_len(p, payload.length); }
const tDeflate = (performance.now() - t0) / rounds;
const t1 = performance.now();
for (let i = 0; i < rounds; i++) { const p = put(deflated); e.inflate_len(p, deflated.length); }
const tInflate = (performance.now() - t1) / rounds;

console.log(JSON.stringify({
  wasm: path.split('/').pop(), imports: WebAssembly.Module.imports(new WebAssembly.Module(bytes)).length,
  input: payload.length, deflated: dlen, inflated_back: ilen, roundtrip_ok: ilen === payload.length,
  crc32: crc >>> 0, deflate_ms: +tDeflate.toFixed(3), inflate_ms: +tInflate.toFixed(3),
}));
