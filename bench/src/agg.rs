//! Aggregates a bench CSV per class and codec: sum(compressed)/sum(raw), mean µs per frame.
use std::collections::BTreeMap;

#[derive(Default)]
struct Acc {
    n: usize,
    raw: u64,
    comp: u64,
    cus: f64,
    dus: f64,
    bad: usize,
}

pub fn run(path: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut m: BTreeMap<(String, String), Acc> = BTreeMap::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 9 {
            continue;
        }
        let a = m.entry((f[0].to_string(), f[2].to_string())).or_default();
        a.n += 1;
        a.raw += f[3].parse::<u64>().unwrap();
        a.comp += f[4].parse::<u64>().unwrap();
        a.cus += f[6].parse::<f64>().unwrap();
        a.dus += f[7].parse::<f64>().unwrap();
        a.bad += (f[8] != "true") as usize;
    }
    println!(
        "{:<18} {:<22} {:>3} {:>9} {:>9} {:>6} {:>9} {:>9} {}",
        "class", "codec", "n", "raw", "comp", "ratio", "c_us/f", "d_us/f", "bad"
    );
    for ((class, codec), a) in &m {
        println!(
            "{:<18} {:<22} {:>3} {:>9} {:>9} {:>6.3} {:>9.2} {:>9.2} {}",
            class,
            codec,
            a.n,
            a.raw,
            a.comp,
            a.comp as f64 / a.raw as f64,
            a.cus / a.n as f64,
            a.dus / a.n as f64,
            if a.bad > 0 { "ROUNDTRIP-FAIL" } else { "" }
        );
    }
}
