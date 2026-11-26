//! A set as it plays, one block at a time, on whichever clock calls it.
//!
//! `set render` and `set play --record` walk a set through this one thing:
//! the planner, the player and the walk between the steps, moved on block by
//! block, with what a performer does (an [`Act`]) handed in between two
//! blocks. Nothing in here reads a wall clock or waits for a thread, so the
//! same acts on the same blocks make the same samples, which is what lets a
//! take recorded live be rendered again sample for sample.

use tatum_core::live::{Applied, LivePlanner, LivePlayer, Plan};
use tatum_core::midi::PadAction;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::set::{describe, Act, Step, Walk};
use crate::setnav::{Move, SetNav};

pub struct Stage {
    pub planner: LivePlanner,
    pub player: LivePlayer,
    pub nav: SetNav,
    /// A scene called, and the bar it takes the keyboard on.
    pending_scene: Option<(String, usize)>,
    /// Samples rendered so far.
    pub frames: usize,
    /// Say where a move lands in a script's count of bars, from 0, rather
    /// than a performer's, from 1.
    pub script_count: bool,
}

/// What one block did besides render.
#[derive(Default)]
pub struct Turn {
    /// What the walk said: a step built, a step that does not compile.
    pub tick: Vec<String>,
    /// What landed in this block, already in the script's words: `bar 12:
    /// now 3/9 03.synth`, `bar 16: scene drop`.
    pub log: Vec<String>,
    /// A step landed: the bar it landed on.
    pub landed: Option<usize>,
}

impl Stage {
    /// The set loaded and its first step playing, with the scene its header
    /// names. `walk.live` walks it as `set play` does: the headers move a
    /// step that moves by itself and nothing else asks for one.
    pub fn new(steps: Vec<Step>, walk: &Walk) -> Result<Self, String> {
        let mut planner = LivePlanner::new();
        planner.set_output_gain(crate::set::set_gain(&steps)?);
        // Each step's `auto ... over N` starts on the step's first bar.
        planner.restart_lanes();
        let mut player = LivePlayer::new();
        let first = planner
            .plan(&steps[0].src, player.generation())
            .map_err(|e| format!("{}: {}", steps[0].name(), describe(&e)))?;
        player.apply(first);
        player.start();
        let mut nav = SetNav::new(steps, walk.phrase, walk.ramp_bars);
        nav.blend_bars = walk.blend_bars;
        // A render walks the steps itself, `auto=` passes included; a take
        // walks them as `set play` did.
        nav.headers = walk.live;
        nav.auto = walk.live && walk.auto;
        // Every step as it was loaded: what the set's gain was measured on,
        // and, for a take, what was played.
        nav.frozen = true;
        let mut stage = Self { planner, player, nav, pending_scene: None, frames: 0, script_count: true };
        if let Some(scene) = stage.nav.steps[0].perform.clone() {
            let g = stage.player.generation();
            let plans: Vec<_> = stage
                .planner
                .enter_scene(&scene, g)
                .into_iter()
                .flatten()
                .chain(stage.planner.scene_values(&scene, None, g).into_iter().flatten())
                .collect();
            stage.send(plans);
        }
        Ok(stage)
    }

    /// Hand plans to the player, as the audio callback does: a song loaded
    /// over one that had ended starts from the top.
    fn send(&mut self, plans: Vec<Plan>) {
        for p in plans {
            if self.player.apply(p) == Applied::Loaded && !self.player.running() {
                self.player.start();
            }
        }
    }

    /// The bar playing, counted from 0.
    pub fn bar(&self) -> usize {
        self.player.engine().map_or(0, |e| e.current_bar())
    }

    /// Where the set is, in bars from its first downbeat, fractional.
    pub fn position(&self) -> f64 {
        self.player.engine().map_or(0.0, |e| e.position_bars())
    }

    fn ask(&mut self, m: Move, bar: usize) -> String {
        let said = self.nav.ask(m, bar);
        if !self.script_count {
            return said;
        }
        // `SetNav::ask` speaks to a performer, counting from 1, which read a
        // bar late next to a script's timings.
        match self.nav.waiting(bar) {
            Some((step, wait)) => format!("asked for {}, lands on bar {}", self.nav.describe(step), bar + wait),
            None => said,
        }
    }

    /// One thing the performer does, between two blocks. Returns what it
    /// did worth reading, or nothing.
    pub fn act(&mut self, act: &Act) -> String {
        let bar = self.bar();
        let g = self.player.generation();
        let mut plans = Vec::new();
        let mut said = String::new();
        match act.clone() {
            Act::Cc(cc, v) => {
                let turn = self.planner.knob(cc, v, g);
                said = turn.readings.join(", ");
                if said.is_empty() {
                    said = format!("cc {} (not mapped)", cc);
                }
                plans = turn.plans;
            }
            Act::Pad(note, v) => {
                // A `loop` pad: the bars it is held over loop.
                if self.planner.pad_navigation(note) == Some(PadAction::Loop) {
                    let done = if v > 0 { Some(self.nav.loop_press(bar)) } else { self.nav.loop_release(bar) };
                    if let Some((op, s)) = done {
                        plans.push(Plan::Control { base: g, op });
                        said = s;
                    }
                } else if v > 0 {
                    if let Some(action) = self.planner.pad_navigation(note) {
                        let m = match action {
                            PadAction::Prev => Move::Prev,
                            PadAction::Step(n) => Move::To(n),
                            _ => Move::Next,
                        };
                        said = self.ask(m, bar);
                    }
                }
                // A set move is only that, as in `set play`.
                if said.is_empty() && plans.is_empty() && self.planner.pad_navigation(note).is_none() {
                    match self.planner.pad(note, v, g) {
                        Some(p) => plans = p,
                        None if v > 0 => said = format!("pad {} (not mapped)", note),
                        None => {}
                    }
                }
            }
            Act::Key(note, v) => {
                // A `loop` key, held, as a `loop` pad.
                if self.planner.key_loops(note) {
                    let done = if v > 0 { Some(self.nav.loop_press(bar)) } else { self.nav.loop_release(bar) };
                    if let Some((op, s)) = done {
                        plans.push(Plan::Control { base: g, op });
                        said = s;
                    }
                } else {
                    match self.planner.key(note, v, g) {
                        Some(p) => plans = p,
                        None if v > 0 => said = format!("key {} (not mapped)", note),
                        None => {}
                    }
                }
            }
            Act::Bend(v) => {
                plans = self.planner.bend(v, g);
                said = self.planner.bend_reading();
            }
            Act::Next => said = self.ask(Move::Next, bar),
            Act::Prev => said = self.ask(Move::Prev, bar),
            Act::Step(n) => said = self.ask(Move::To(n), bar),
            Act::Perform(scene) => {
                // A phrase counts from the step's first bar.
                let q = (self.planner.keyboard().1 as usize).max(1);
                let origin = bar + 1 - self.nav.bar_in_step(bar);
                let target = origin + ((bar - origin) / q + 1) * q;
                match self.planner.scene_values(&scene, Some(target), g) {
                    Some(p) => {
                        plans = p;
                        said = format!("scene {}, on bar {}", scene, target);
                        self.pending_scene = Some((scene, target));
                    }
                    None => said = format!("no scene named {}", scene),
                }
            }
            Act::Pressure(pad, note, v) => plans = self.planner.pressure(pad, note, v),
            Act::Reset => {
                plans = self.planner.reset_knobs(g);
                said = String::from("reset: every knob back to the text");
            }
            Act::Voice(mv) => {
                said = self.planner.voice(&mv, g).unwrap_or_else(|| String::from("no knob pages"));
            }
            Act::Loop(down) => {
                let done = if down { Some(self.nav.loop_press(bar)) } else { self.nav.loop_release(bar) };
                if let Some((op, s)) = done {
                    plans.push(Plan::Control { base: g, op });
                    said = s;
                }
            }
            Act::Keep => said = self.nav.toggle_loop(bar),
            Act::Mute(track) => {
                plans = self.planner.toggle_mute(&track, g);
                said = format!("mute {}", track);
            }
            Act::Solo(tracks) => {
                plans = self.planner.toggle_solo_group(&tracks, g);
                said = format!("solo {}", tracks.join(" "));
            }
            Act::End => {}
        }
        self.send(plans);
        said
    }

    /// Render the next block, `l.len()` frames (at most `BLOCK_SIZE`): a
    /// scene whose bar has come takes the keyboard, the walk moves on, the
    /// player plays.
    pub fn block(&mut self, l: &mut [f32], r: &mut [f32]) -> Result<Turn, String> {
        let len = l.len().min(r.len()).min(BLOCK_SIZE);
        let mut turn = Turn::default();
        let e = self.player.engine().ok_or("the set did not load")?;
        let (bar, tempo, g) = (e.current_bar(), e.tempo(), self.player.generation());
        if let Some((scene, target)) = self.pending_scene.clone() {
            if bar >= target {
                self.pending_scene = None;
                let plans = self.planner.enter_scene(&scene, g).unwrap_or_default();
                self.send(plans);
                turn.log.push(format!("bar {}: scene {}", bar, self.planner.describe_scene(&scene)));
            }
        }
        let now = self.frames as f32 / SAMPLE_RATE;
        let before = self.nav.current;
        let (plans, said) = self.nav.tick(bar, tempo, g, now, &mut self.planner);
        self.send(plans);
        turn.tick = said;
        if self.player.process(&mut l[..len], &mut r[..len]).is_some() {
            self.nav.landed(self.player.generation(), now);
        }
        if self.nav.current != before {
            let landed_at = self.bar();
            turn.landed = Some(landed_at);
            if let Some(scene) =
                self.nav.steps[self.nav.current].perform.clone().filter(|s| self.planner.scene() != Some(s))
            {
                let g = self.player.generation();
                let plans: Vec<_> = self
                    .planner
                    .enter_scene(&scene, g)
                    .into_iter()
                    .flatten()
                    .chain(self.planner.scene_values(&scene, None, g).into_iter().flatten())
                    .collect();
                self.send(plans);
                self.pending_scene = None;
                turn.log.push(format!("bar {}: scene {}", landed_at, self.planner.describe_scene(&scene)));
            }
            turn.log.push(format!("bar {}: now {}", landed_at, self.nav.describe(self.nav.current)));
        }
        self.frames += len;
        Ok(turn)
    }
}
