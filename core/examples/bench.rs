//! How much machine a song needs: run on the machine you want to play from.
//!
//! ```text
//! cargo run --release -p tatum-core --example bench -- examples/dub_techno.synth [more.synth ...] [--seconds 60]
//! ```
//!
//! For each song it builds the engine, plays `--seconds` of it one block at a
//! time the way an audio callback asks for it, and prints:
//!
//! - **heap**: the bytes the engine holds once built, and the most it held
//!   while building (parse and compile included). On a microcontroller this
//!   is the RAM the song needs besides the program itself.
//! - **allocs while playing**: should be 0; an allocation on the audio
//!   thread is a click waiting to happen.
//! - **x real time**: seconds of audio per second of CPU, on one core. Under
//!   1.0 the song cannot be played on this machine at all.
//! - **x real, heaviest 4 s**: the same over the four seconds of the song that
//!   cost the most, and where they start. A song is as playable as its
//!   heaviest moment, not its average.
//! - **median / p99 / worst block** against the budget of one block
//!   (128 samples at 44.1 kHz, 2.9 ms). The worst block is the one that
//!   clicks; on a busy machine it describes the machine more than the engine.
//!
//! `--from S` plays the first S seconds unmeasured and measures from there.
//! `--seconds 0` builds the engine (and plays `--from`) and stops: run it under
//! an instruction counter (valgrind --tool=cachegrind) with and without
//! `--seconds 4`, and the difference is what those four seconds cost,
//! independent of the machine's clock.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let now = NOW.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        NOW.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, layout, new);
        if !q.is_null() {
            let now = if new >= layout.size() {
                NOW.fetch_add(new - layout.size(), Relaxed) + (new - layout.size())
            } else {
                NOW.fetch_sub(layout.size() - new, Relaxed) - (layout.size() - new)
            };
            PEAK.fetch_max(now, Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn kib(b: usize) -> String {
    format!("{:.0} KiB", b as f64 / 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seconds = 60.0f64;
    let mut from = 0.0f64;
    let mut songs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--seconds" {
            seconds = args.get(i + 1).and_then(|v| v.parse().ok()).expect("--seconds needs a number");
            i += 1;
        } else if args[i] == "--from" {
            from = args.get(i + 1).and_then(|v| v.parse().ok()).expect("--from needs a number");
            i += 1;
        } else {
            songs.push(args[i].clone());
        }
        i += 1;
    }
    if songs.is_empty() {
        eprintln!("usage: bench <song.synth> [more.synth ...] [--seconds 60] [--from 0]");
        std::process::exit(1);
    }
    let budget_us = BLOCK_SIZE as f64 / SAMPLE_RATE as f64 * 1e6;
    println!(
        "{:<22} {:>10} {:>10} {:>7} {:>8} {:>17} {:>15} {:>15} {:>15}",
        "song", "heap", "peak", "allocs", "x real", "heaviest 4 s", "median", "p99", "worst"
    );
    for path in songs {
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let name = std::path::Path::new(&path).file_stem().unwrap().to_string_lossy().into_owned();
        let base = NOW.load(Relaxed);
        PEAK.store(base, Relaxed);
        let mut engine = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{path}: {e}"));
        engine.start();
        let held = NOW.load(Relaxed) - base;
        let peak = PEAK.load(Relaxed) - base;
        let mut l = [0.0f32; BLOCK_SIZE];
        let mut r = [0.0f32; BLOCK_SIZE];
        for _ in 0..(SAMPLE_RATE as f64 * from / BLOCK_SIZE as f64) as usize {
            engine.process_block_stereo(&mut l, &mut r);
        }
        let blocks = (SAMPLE_RATE as f64 * seconds / BLOCK_SIZE as f64) as usize;
        let mut times = Vec::with_capacity(blocks);
        let before = ALLOCS.load(Relaxed);
        let start = Instant::now();
        for _ in 0..blocks {
            let t = Instant::now();
            engine.process_block_stereo(&mut l, &mut r);
            times.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let total = start.elapsed().as_secs_f64();
        // `times` was sized before the loop, so these are the engine's own.
        let allocs = ALLOCS.load(Relaxed) - before;
        if blocks == 0 {
            println!("{name:<22} {:>10} {:>10}", kib(held), kib(peak));
            continue;
        }
        // The four seconds that cost the most, sliding a second at a time. A
        // block more than four times the 99th percentile is the machine
        // pausing, not the engine, and counts as that percentile: one such
        // pause would otherwise make its four seconds the heaviest.
        let cap = {
            let mut sorted = times.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            sorted[((sorted.len() - 1) as f64 * 0.99) as usize] * 4.0
        };
        let per_second = (SAMPLE_RATE as usize / BLOCK_SIZE).max(1);
        let seconds_cpu: Vec<f64> = times.chunks(per_second).map(|c| c.iter().map(|&t| t.min(cap)).sum()).collect();
        let (at, heaviest) = seconds_cpu
            .windows(4.min(seconds_cpu.len()))
            .enumerate()
            .map(|(k, w)| (k, w.iter().sum::<f64>()))
            .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        let window = 4.min(seconds_cpu.len()) as f64;
        let at = from + at as f64;
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        let pct = |us: f64| format!("{us:6.0} us {:3.0}%", us / budget_us * 100.0);
        println!(
            "{name:<22} {:>10} {:>10} {allocs:>7} {:>7.1}x {:>17} {:>15} {:>15} {:>15}",
            kib(held),
            kib(peak),
            seconds / total,
            format!("{:.1}x at {}:{:02}", window * 1e6 / heaviest, at as u32 / 60, at as u32 % 60),
            pct(pick(0.5)),
            pct(pick(0.99)),
            pct(*times.last().unwrap()),
        );
    }
}
