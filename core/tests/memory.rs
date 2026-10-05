//! How much memory a song holds once its engine is built. A microcontroller
//! has a few hundred kilobytes of fast RAM; a simple song should fit in 512 KB
//! of it, and it did not while every delay line held two seconds whatever the
//! echo asked for.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

use tatum_core::song_engine::SongEngine;

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}
static HELD: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.with(Cell::get) {
            HELD.fetch_add(layout.size() as isize, Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if ARMED.with(Cell::get) {
            HELD.fetch_sub(layout.size() as isize, Relaxed);
        }
        System.dealloc(p, layout)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn held_by(path: &str) -> usize {
    let src = std::fs::read_to_string(path).unwrap();
    HELD.store(0, Relaxed);
    ARMED.with(|a| a.set(true));
    let engine = SongEngine::from_source(&src).unwrap();
    let held = HELD.load(Relaxed);
    ARMED.with(|a| a.set(false));
    drop(engine);
    held as usize
}

#[test]
fn a_simple_song_fits_in_512_kb() {
    for song in ["arp_keys", "live_set", "acid_arp"] {
        let path = format!("{}/../examples/{song}.synth", env!("CARGO_MANIFEST_DIR"));
        let kb = held_by(&path) / 1024;
        assert!(kb <= 512, "{song} holds {kb} KB once built");
    }
}

#[test]
fn the_slowest_tempo_counts_the_tempo_knob() {
    let song = |midi: &str| {
        let src = format!(
            "tempo 130\nmodule beats kit {{ kick_level 1.0 }}\npattern b {{ kick: X - - - }}\ntrack d {{ play b using kit }}\n{midi}\n"
        );
        let parsed = tatum_core::dsl::parse(&src).unwrap();
        tatum_core::dsl::compiler::compile(&parsed).unwrap().slowest_tempo
    };
    assert_eq!(song(""), 130.0);
    assert_eq!(song("midi { cc 27 > tempo 118..132 }"), 118.0);
    // A knob with no range spans 60 to 180 BPM.
    assert_eq!(song("midi { cc 27 > tempo }"), 60.0);
}
