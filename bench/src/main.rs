//! Per-file ratio and timing on the ILM corpus for zstd-rs and baselines.
//! Usage: zl-bench run <corpus_dir> <zdict> <rill_raw_dict> [budget_ms] > out.csv
//!        zl-bench agg <out.csv>
mod agg;
mod codecs;

use std::time::Instant;

fn time_it<F: FnMut() -> Vec<u8>>(mut f: F, budget_ms: u128) -> (Vec<u8>, f64) {
    let first = f();
    let start = Instant::now();
    let mut iters = 0u32;
    while iters < 2000 && (iters < 3 || start.elapsed().as_millis() < budget_ms) {
        std::hint::black_box(f());
        iters += 1;
    }
    (first, start.elapsed().as_nanos() as f64 / iters as f64 / 1000.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("agg") {
        agg::run(&args[2]);
        return;
    }
    if args.get(1).map(String::as_str) == Some("prof") {
        // zl-bench prof <codec> <c|d> <seconds> <zdict> <files...>: hot loop for profilers.
        let zdict = std::fs::read(&args[5]).expect("zdict");
        let mut codecs = codecs::all(&zdict, &[]);
        let codec = codecs.iter_mut().find(|c| c.name == args[2]).expect("codec");
        let files: Vec<Vec<u8>> = args[6..].iter().map(|f| std::fs::read(f).unwrap()).collect();
        let comp: Vec<Vec<u8>> = files.iter().map(|d| (codec.c)(d)).collect();
        let secs: f64 = args[4].parse().unwrap();
        let start = Instant::now();
        let mut n = 0u64;
        while start.elapsed().as_secs_f64() < secs {
            for (d, c) in files.iter().zip(&comp) {
                if args[3] == "c" {
                    std::hint::black_box((codec.c)(d));
                } else {
                    std::hint::black_box((codec.d)(c, d.len()));
                }
                n += 1;
            }
        }
        eprintln!("{n} ops, {:.3} us/op", start.elapsed().as_secs_f64() * 1e6 / n as f64);
        return;
    }
    let dir = &args[2];
    let zdict = std::fs::read(&args[3]).expect("zdict");
    let rdict = std::fs::read(&args[4]).expect("rill dict");
    let budget: u128 = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(100);
    let only = std::env::var("CODECS").ok();
    let mut codecs = codecs::all(&zdict, &rdict);
    if let Some(o) = &only {
        codecs.retain(|c| o.split(',').any(|n| c.name == n || (n.ends_with('*') && c.name.starts_with(&n[..n.len() - 1]))));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
    files.sort();
    println!("class,file,codec,raw,compressed,ratio,comp_us,decomp_us,roundtrip_ok");
    for path in files {
        let data = std::fs::read(&path).unwrap();
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let class = fname.split("__").next().unwrap().to_string();
        for codec in codecs.iter_mut() {
            let (comp, cus) = time_it(|| (codec.c)(&data), budget);
            let (dec, dus) = time_it(|| (codec.d)(&comp, data.len()), budget);
            let ok = dec == data;
            println!(
                "{},{},{},{},{},{:.4},{:.3},{:.3},{}",
                class,
                fname,
                codec.name,
                data.len(),
                comp.len(),
                comp.len() as f64 / data.len() as f64,
                cus,
                dus,
                ok
            );
        }
    }
}
