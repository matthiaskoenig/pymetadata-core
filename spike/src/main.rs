//! Spike timings: lazy in-memory OMEX core, native.
//!
//! Protocol of the sibling repos: 3 warm-up calls, then 3 rounds of 20 timed
//! calls; the number is the median of the three round medians.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use spike_wasm::{extract_all, manifest_only, modify_rawcopy, read_all, write_recompress};

const WARMUP: usize = 3;
const ROUNDS: usize = 3;
const CALLS: usize = 20;

fn median(v: &mut Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn time<F: FnMut() -> u64>(mut f: F) -> (f64, u64) {
    let mut checksum = 0u64;
    for _ in 0..WARMUP {
        checksum = f();
    }
    let mut round_medians = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let mut calls = Vec::with_capacity(CALLS);
        for _ in 0..CALLS {
            let t0 = Instant::now();
            checksum = f();
            calls.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        round_medians.push(median(&mut calls));
    }
    (median(&mut round_medians), checksum)
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/mkoenig/git/pymetadata/tests/data/omex".to_string());
    let mut paths: Vec<PathBuf> = fs::read_dir(Path::new(&dir))
        .expect("data dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "omex"))
        .collect();
    paths.sort();

    println!("archive,size_bytes,entries,op,ms,checksum");
    for path in &paths {
        let bytes = fs::read(path).expect("read archive");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let entries = manifest_only(&bytes).map(|e| e.len()).unwrap_or(0);

        let (ms, cs) = time(|| manifest_only(&bytes).map(|e| e.len() as u64).unwrap_or(0));
        println!("{name},{},{entries},manifest_only,{ms:.4},{cs}", bytes.len());

        let (ms, cs) = time(|| read_all(&bytes).unwrap_or(0) as u64);
        println!("{name},{},{entries},read_all,{ms:.4},{cs}", bytes.len());

        let (ms, cs) = time(|| {
            let tmp = std::env::temp_dir().join(format!("spike_x_{}", std::process::id()));
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp).unwrap();
            let n = extract_all(&bytes, &tmp).unwrap_or(0) as u64;
            let _ = fs::remove_dir_all(&tmp);
            n
        });
        println!("{name},{},{entries},extract_all,{ms:.4},{cs}", bytes.len());

        // replace the first non-directory entry with 64 kB of new content
        let target = {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let mut t = String::new();
            for i in 0..za.len() {
                let f = za.by_index(i).unwrap();
                let fname = f.name().unwrap().to_string();
                if !f.is_dir() && fname != "manifest.xml" {
                    t = fname;
                    break;
                }
            }
            t
        };
        let payload = vec![b'x'; 64 * 1024];
        let (ms, cs) = time(|| {
            modify_rawcopy(&bytes, &target, &payload)
                .map(|v| v.len() as u64)
                .unwrap_or(0)
        });
        println!("{name},{},{entries},modify_rawcopy,{ms:.4},{cs}", bytes.len());

        let (ms, cs) = time(|| write_recompress(&bytes).map(|v| v.len() as u64).unwrap_or(0));
        println!("{name},{},{entries},write_recompress,{ms:.4},{cs}", bytes.len());
    }
}
