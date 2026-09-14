//! 8.10: the audio path must not touch the allocator. MIDI input, hardware and
//! any strict real-time use depend on it, and a block that allocates will
//! eventually block on a lock inside the allocator and drop out.
//!
//! A counting allocator is the only way to assert this rather than hope: the
//! per-block allocations that used to be here were invisible in every test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

use synth_core::song_engine::SongEngine;
use synth_core::BLOCK_SIZE;

/// Drums with a nudge, an arpeggiated chord track, a bus, both sends and three
/// scenes with automation: every path that used to allocate per block.
const SONG: &str = r#"
tempo 128
scale A minor
sidechain 0.4

module beats kit { kick_level 1.0 }
module bass low { cutoff 0.4 }
module keys pad { cutoff 0.5 }

pattern beat  {
    kick:  X - - x  - - X -
    snare: - - - -  X - - -
}
pattern line  { 1.1 - 1.3 -  1.5 - 1.3 - }
pattern hold  { [1.3 3.3 5.3] .. .. .. .. .. .. .. }

bus drums
drums { in > compressor(-8, ratio=4) > master }

track beat { play beat using kit out > drums }
track bass { play line using low delay_send 0.2 out > master }
track pad  { play hold using pad arp up rate=16 octaves=2 reverb_send 0.4 out > master }

master { in > eq(low=2) > gain(0.8) > limiter > out }

scene a { track beat { play beat using kit } track bass { play line using low } }
scene b {
    auto low cutoff 0.2 > 0.8
    auto master gain 0.6 > 0.9
    track beat { play beat using kit }
    track pad  { play hold using pad }
}
scene c { track pad { play hold using pad } }
arrange { a x2 b x2 c x2 a x2 }
"#;

#[test]
fn rendering_blocks_never_allocates() {
    let mut engine = SongEngine::from_source(SONG).expect("compiles");
    engine.start();
    let mut l = [0.0f32; BLOCK_SIZE];
    let mut r = [0.0f32; BLOCK_SIZE];

    // Warm up outside the count: the first blocks cross the first scene change
    // and settle any lazily built state.
    for _ in 0..64 {
        engine.process_block_stereo(&mut l, &mut r);
    }

    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    // Long enough to cross every scene boundary in the arrangement, which is
    // where automation lanes are rebuilt and tracks are reassigned.
    let mut heard = false;
    for _ in 0..8000 {
        engine.process_block_stereo(&mut l, &mut r);
        heard |= l.iter().any(|v| *v != 0.0);
    }
    COUNTING.store(false, Ordering::Relaxed);

    let n = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(n, 0, "the audio path allocated {} times over 8000 blocks", n);
    assert!(heard, "rendered silence, so the test proved nothing");
}
