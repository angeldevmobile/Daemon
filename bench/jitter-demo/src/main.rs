use std::thread;
use std::time::{Duration, Instant};

const PERIOD: Duration = Duration::from_millis(1);
const REPORT_EVERY: Duration = Duration::from_secs(1);

fn main() {
    let noise = std::env::args().any(|a| a == "--noise");
    if noise {
        let n = thread::available_parallelism().map_or(2, |n| n.get());
        for _ in 0..n {
            thread::spawn(noise_loop);
        }
        println!("noise: {n} busy threads");
    }

    let mut late_us = Vec::with_capacity(1024);
    let mut next = Instant::now() + PERIOD;
    let mut report_at = Instant::now() + REPORT_EVERY;
    loop {
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        let now = Instant::now();
        late_us.push(now.saturating_duration_since(next).as_micros() as u64);
        next += PERIOD;
        if next < now {
            next = now + PERIOD;
        }

        if now >= report_at {
            late_us.sort_unstable();
            println!(
                "wake-up lateness  p50 {:>6}µs  p99 {:>6}µs  max {:>6}µs  ({} samples)",
                percentile(&late_us, 50.0),
                percentile(&late_us, 99.0),
                late_us.last().copied().unwrap_or(0),
                late_us.len()
            );
            late_us.clear();
            report_at = now + REPORT_EVERY;
        }
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx]
}

fn noise_loop() {
    loop {
        let until = Instant::now() + Duration::from_millis(5);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
        thread::sleep(Duration::from_millis(1));
    }
}
