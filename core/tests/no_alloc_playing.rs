//! Playing a song never allocates. The audio callback asks for a block every
//! 2.9 ms, and an allocation is a call into the system allocator that takes
//! as long as it likes: a click on a laptop, and on a microcontroller a heap
//! that fragments until a note cannot be played. Every song in the corpus is
//! built, then played under a counting allocator, and nothing may be counted.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

struct Counting;

// Only this thread's allocations count: the test harness allocates on its
// own threads (its "running for over 60 seconds" notice, for one).
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}
static COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.with(Cell::get) {
            COUNT.fetch_add(1, Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if ARMED.with(Cell::get) {
            COUNT.fetch_add(1, Relaxed);
        }
        System.realloc(p, layout, new)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Release only, like the realtime budget: in debug, two minutes of every
/// song is a long wait for a check that does not depend on the build.
#[test]
#[cfg_attr(debug_assertions, ignore = "release only: it plays two minutes of every song")]
fn no_song_allocates_while_it_plays() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples");
    let mut songs: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "synth"))
        .collect();
    songs.sort();
    let mut offenders = Vec::new();
    for path in songs {
        let src = std::fs::read_to_string(&path).unwrap();
        let mut engine = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        engine.start();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        COUNT.store(0, Relaxed);
        ARMED.with(|a| a.set(true));
        for _ in 0..(SAMPLE_RATE as usize * 120 / BLOCK_SIZE) {
            engine.process_block_stereo(&mut l, &mut r);
        }
        ARMED.with(|a| a.set(false));
        let n = COUNT.load(Relaxed);
        if n > 0 {
            offenders.push(format!("{}: {n}", path.file_name().unwrap().to_string_lossy()));
        }
    }
    assert!(offenders.is_empty(), "allocations while playing two minutes: {}", offenders.join(", "));
}
