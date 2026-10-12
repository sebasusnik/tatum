//! A loop of whole bars, caught by hand: the bars between `loop_in` and
//! `loop_out` play again and again, the way a looper or a DJ player's loop
//! does, but on the sequence rather than on the audio, so the knobs still
//! move what is looped. At every bar line the engine notes where each track
//! is in its pattern and how far the `auto` lanes have gone; looping is
//! putting that note back at the bar line the loop ends on. The bar count
//! keeps going: only what plays goes back.

use super::SongEngine;

/// Where a track was at a bar line.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TrackMark {
    current_step: usize,
    loop_start: usize,
    loop_index: u64,
    pattern_idx: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Off,
    /// From the bar `start` began on, waiting for the end.
    Open,
    /// Every `len` steps from `start`, back to it; the next time on `wrap`.
    Looping {
        len: usize,
        wrap: usize,
    },
}

pub(super) struct Looper {
    /// The bar line just crossed: its first step and what was there.
    bar_step: usize,
    bar_marks: alloc::vec::Vec<TrackMark>,
    bar_lane_origin: usize,
    bar_scene_step: usize,
    /// The loop's start, as `bar_*` were then.
    start: usize,
    marks: alloc::vec::Vec<TrackMark>,
    lane_origin: usize,
    scene_step: usize,
    state: State,
}

impl Looper {
    /// Sized once for the tracks, so nothing grows on the audio thread.
    pub(super) fn new(tracks: usize) -> Self {
        Self {
            bar_step: 0,
            bar_marks: alloc::vec![TrackMark::default(); tracks],
            bar_lane_origin: 0,
            bar_scene_step: 0,
            start: 0,
            marks: alloc::vec![TrackMark::default(); tracks],
            lane_origin: 0,
            scene_step: 0,
            state: State::Off,
        }
    }
}

impl SongEngine {
    /// At a bar line, before its first step fires: back to the loop's start
    /// if this is where it ends, then note where everything is.
    pub(super) fn loop_bar_line(&mut self) {
        if let State::Looping { len, wrap } = self.looper.state {
            if self.global_step == wrap {
                let l = &mut self.looper;
                for (t, m) in self.tracks.iter_mut().zip(l.marks.iter()) {
                    t.current_step = m.current_step;
                    t.loop_start = m.loop_start;
                    t.loop_index = m.loop_index;
                    t.pattern_idx = m.pattern_idx;
                }
                // The lanes read as many steps since their origin as they
                // had at the start.
                self.lane_origin = l.lane_origin + (self.global_step - l.start);
                self.scene_step = l.scene_step;
                l.state = State::Looping { len, wrap: wrap + len };
            }
        }
        let l = &mut self.looper;
        l.bar_step = self.global_step;
        for (m, t) in l.bar_marks.iter_mut().zip(self.tracks.iter()) {
            *m = TrackMark {
                current_step: t.current_step,
                loop_start: t.loop_start,
                loop_index: t.loop_index,
                pattern_idx: t.pattern_idx,
            };
        }
        l.bar_lane_origin = self.lane_origin;
        l.bar_scene_step = self.scene_step;
    }

    /// The loop starts on the bar playing now.
    pub fn loop_in(&mut self) {
        let l = &mut self.looper;
        l.marks.copy_from_slice(&l.bar_marks);
        l.start = l.bar_step;
        l.lane_origin = l.bar_lane_origin;
        l.scene_step = l.bar_scene_step;
        l.state = State::Open;
    }

    /// The loop is `bars` long from its start: it goes back on the bar line
    /// that ends them, or the next one a whole loop later if that has passed.
    pub fn loop_out(&mut self, bars: usize) {
        if self.looper.state != State::Open {
            return;
        }
        let len = bars.max(1) * self.steps_per_bar.max(1);
        let mut wrap = self.looper.start + len;
        while wrap < self.global_step {
            wrap += len;
        }
        self.looper.state = State::Looping { len, wrap };
    }

    /// No more going back: what plays goes on from where it is.
    pub fn loop_exit(&mut self) {
        self.looper.state = State::Off;
    }
}
