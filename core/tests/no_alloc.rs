//! 8.10: the audio path must not touch the allocator. MIDI input, hardware and
//! any strict real-time use depend on it, and a block that allocates will
//! eventually block on a lock inside the allocator and drop out.
//!
//! A counting allocator is the only way to assert this rather than hope: the
//! per-block allocations that used to be here were invisible in every test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// Frees, counted separately: a free takes the allocator's lock as much as an
/// allocation does, but only the fast-edit test asks about them so far.
static FREES: AtomicUsize = AtomicUsize::new(0);
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
        if COUNTING.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
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

use tatum_core::song_engine::SongEngine;
use tatum_core::BLOCK_SIZE;

/// Drums with a nudge, an arpeggiated chord track, two buses, a capture window,
/// both sends and three scenes with automation: every path that used to
/// allocate per block, plus the one node that owns a large buffer.
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

bus ghost
ghost { in > capture(1, speed=0.5, reverse=1) > master }

track beat { play beat using kit out > drums }
track bass { play line using low delay_send 0.2 out > ghost }
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

/// The live swap is the other path that runs on the audio thread: the split
/// block, the handover (`start_from_bar`, inheritance by `mem::swap`, the
/// coasting old engine under its fade) and the retirement of the old engine.
/// Every engine is built outside the count, as the control thread does.
#[test]
fn live_swap_never_allocates() {
    use tatum_core::live::{Applied, LivePlanner, LivePlayer};

    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(SONG, player.generation()).expect("compiles");
    assert_eq!(player.apply(plan), Applied::Loaded);
    player.start();
    let mut l = [0.0f32; BLOCK_SIZE];
    let mut r = [0.0f32; BLOCK_SIZE];
    for _ in 0..64 {
        player.process(&mut l, &mut r);
    }
    // Two edits: one that keeps every track (an unused pattern) and one that
    // changes the pad into another kind of module, so both the inherited and
    // the released paths run. Both are planned, and their engines built,
    // outside the count.
    let edit_a = format!("{}\npattern unused {{ 1.1 - - - }}\n", SONG);
    let edit_b = edit_a.replace("module keys pad { cutoff 0.5 }", "module bass pad { cutoff 0.5 }");
    assert_ne!(edit_a, edit_b);
    let plan_a = planner.plan(&edit_a, player.generation()).expect("compiles");

    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    assert_eq!(player.apply(plan_a), Applied::Queued);
    let mut swaps = 0;
    let mut blocks = 0;
    while swaps < 1 && blocks < 4000 {
        if player.process(&mut l, &mut r).is_some() { swaps += 1; }
        blocks += 1;
    }
    COUNTING.store(false, Ordering::Relaxed);
    let plan_b = planner.plan(&edit_b, player.generation()).expect("compiles");
    COUNTING.store(true, Ordering::Relaxed);
    assert_eq!(player.apply(plan_b), Applied::Queued);
    while swaps < 2 && blocks < 8000 {
        if player.process(&mut l, &mut r).is_some() { swaps += 1; }
        blocks += 1;
    }
    // Past the fade-out, so the coasting engine retires inside the count.
    for _ in 0..8 {
        player.process(&mut l, &mut r);
    }
    COUNTING.store(false, Ordering::Relaxed);

    assert_eq!(swaps, 2, "both swaps must land");
    let n = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(n, 0, "the live swap path allocated {} times", n);
    let mut retired = 0;
    while player.take_retired().is_some() { retired += 1; }
    assert_eq!(retired, 2, "both old engines must come back for dropping");
}

/// A value edit arrives as a list of ops the planner allocated. Applying it
/// on the audio thread must not free the list there: it comes back through
/// `take_retired` like an engine, to be dropped off-thread.
#[test]
fn a_fast_edit_frees_nothing_on_the_audio_thread() {
    use tatum_core::live::{Applied, LivePlanner, LivePlayer};

    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    assert_eq!(player.apply(planner.plan(SONG, player.generation()).expect("compiles")), Applied::Loaded);
    player.start();
    let mut l = [0.0f32; BLOCK_SIZE];
    let mut r = [0.0f32; BLOCK_SIZE];
    for _ in 0..16 {
        player.process(&mut l, &mut r);
    }
    let edit = SONG.replace("module bass low { cutoff 0.4 }", "module bass low { cutoff 0.6 }");
    let plan = planner.plan(&edit, player.generation()).expect("compiles");

    ALLOCS.store(0, Ordering::Relaxed);
    FREES.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let applied = player.apply(plan);
    for _ in 0..16 {
        player.process(&mut l, &mut r);
    }
    COUNTING.store(false, Ordering::Relaxed);

    assert_eq!(applied, Applied::Fast);
    let (a, f) = (ALLOCS.load(Ordering::Relaxed), FREES.load(Ordering::Relaxed));
    assert_eq!((a, f), (0, 0), "a fast edit allocated {} and freed {} times on the audio thread", a, f);
    let mut retired = 0;
    while player.take_retired().is_some() { retired += 1; }
    assert_eq!(retired, 1, "the op list must come back for dropping");
}
