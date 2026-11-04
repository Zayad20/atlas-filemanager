#[path = "../crates/atlas-ui/src/ui/cleanup/packing.rs"]
mod packing;
fn main() {
    let weights: Vec<u64> = (1..=32).rev().map(|i| i * i * 1000).collect();
    let mut samples = Vec::new();
    for _ in 0..300 {
        let started = std::time::Instant::now();
        std::hint::black_box(packing::pack(std::hint::black_box(&weights), 730., 520.));
        samples.push(started.elapsed().as_secs_f64()*1000.);
    }
    samples.sort_by(f64::total_cmp);
    let output = std::env::args_os().nth(1).expect("report path");
    std::fs::write(output, format!("{{\"median_ms\":{},\"p95_ms\":{},\"nodes\":32,\"iterations\":300}}", samples[150],samples[284])).unwrap();
}
