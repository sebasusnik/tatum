use crate::harmony::HarmonyContext;
use crate::math;
use crate::SAMPLE_RATE;

/// Arp pattern direction.
#[derive(Clone, Copy, PartialEq)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
}

/// Events emitted by the arp processor each sample tick.
pub enum ArpEvent {
    NoteOn(u8, f32),
    NoteOff(u8),
}

/// Pure note-scheduling arpeggiator — no audio, no oscillator, no filter.
/// Attach to any melodic track to arpeggiate its instrument.
pub struct ArpProcessor {
    pattern: ArpPattern,
    current_step: usize,
    direction: i8,
    arp_notes: [u8; 16],
    num_notes: usize,
    samples_per_step: f32,
    sample_counter: f32,
    gate_length: f32,
    octave_range: u8,
    gate_open: bool,
    current_note: u8,
    velocity: f32,
    active: bool,
}

impl Default for ArpProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl ArpProcessor {
    pub fn new() -> Self {
        let samples_per_step = SAMPLE_RATE * 60.0 / 120.0 / 4.0;
        Self {
            pattern: ArpPattern::Up,
            current_step: 0,
            direction: 1,
            arp_notes: [0; 16],
            num_notes: 0,
            samples_per_step,
            sample_counter: 0.0,
            gate_length: 0.7,
            octave_range: 2,
            gate_open: false,
            current_note: 0,
            velocity: 1.0,
            active: false,
        }
    }

    /// Tick once per sample. Returns an event if the arp triggers a note-on or note-off.
    pub fn tick(&mut self) -> Option<ArpEvent> {
        if !self.active || self.num_notes == 0 {
            return None;
        }

        self.sample_counter += 1.0;

        // Gate off at gate_length fraction of step
        let gate_off_point = self.samples_per_step * self.gate_length;
        if self.gate_open
            && self.sample_counter >= gate_off_point
            && self.sample_counter < gate_off_point + 1.0
        {
            self.gate_open = false;
            return Some(ArpEvent::NoteOff(self.current_note));
        }

        // Step boundary — advance and trigger new note
        if self.sample_counter >= self.samples_per_step {
            self.sample_counter = 0.0;
            self.advance_step();
            let note = self.arp_notes[self.current_step % self.num_notes];
            self.current_note = note;
            self.gate_open = true;
            return Some(ArpEvent::NoteOn(note, self.velocity));
        }

        None
    }

    /// Rebuild note list from harmony context.
    pub fn rebuild_notes_from_harmony(&mut self, harmony: &HarmonyContext) {
        let chord = harmony.chord_notes();
        let mut idx = 0;
        for octave in 0..self.octave_range {
            for note in chord.iter().flatten() {
                if idx < 16 {
                    self.arp_notes[idx] = note + octave * 12;
                    idx += 1;
                }
            }
        }
        self.num_notes = idx;
    }

    /// Set notes directly (for non-harmony use cases, e.g. held notes).
    pub fn set_notes(&mut self, notes: &[u8]) {
        let count = notes.len().min(16);
        self.arp_notes[..count].copy_from_slice(&notes[..count]);
        self.num_notes = count;
    }

    /// Start the arpeggiator — triggers immediately on next tick.
    pub fn start(&mut self, velocity: f32) {
        self.active = true;
        self.velocity = velocity;
        self.current_step = 0;
        self.direction = 1;
        self.sample_counter = self.samples_per_step; // trigger on next tick
    }

    /// Stop the arpeggiator and emit a final note-off if needed.
    pub fn stop(&mut self) -> Option<ArpEvent> {
        self.active = false;
        if self.gate_open {
            self.gate_open = false;
            Some(ArpEvent::NoteOff(self.current_note))
        } else {
            None
        }
    }

    /// Change the velocity used for subsequent note-ons without restarting.
    pub fn set_velocity(&mut self, velocity: f32) {
        self.velocity = velocity;
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.set_bpm(60.0 + rate * 180.0);
    }

    pub fn set_gate(&mut self, gate: f32) {
        self.gate_length = math::clamp(gate, 0.1, 0.95);
    }

    pub fn set_pattern(&mut self, value: f32) {
        self.pattern = match (value * 2.0) as u8 {
            0 => ArpPattern::Up,
            1 => ArpPattern::Down,
            _ => ArpPattern::UpDown,
        };
    }

    pub fn set_octave_range(&mut self, value: f32) {
        self.octave_range = 1 + (value * 3.0) as u8; // 1-4
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.current_step = 0;
        self.sample_counter = 0.0;
        self.direction = 1;
        self.gate_open = false;
        self.active = false;
    }

    fn advance_step(&mut self) {
        if self.num_notes == 0 {
            return;
        }

        match self.pattern {
            ArpPattern::Up => {
                self.current_step = (self.current_step + 1) % self.num_notes;
            }
            ArpPattern::Down => {
                if self.current_step == 0 {
                    self.current_step = self.num_notes - 1;
                } else {
                    self.current_step -= 1;
                }
            }
            ArpPattern::UpDown => {
                let next = self.current_step as i8 + self.direction;
                if next >= self.num_notes as i8 {
                    self.direction = -1;
                    self.current_step = if self.num_notes > 1 {
                        self.num_notes - 2
                    } else {
                        0
                    };
                } else if next < 0 {
                    self.direction = 1;
                    self.current_step = if self.num_notes > 1 { 1 } else { 0 };
                } else {
                    self.current_step = next as usize;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arp_up_pattern() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60, 64, 67]); // C major triad
        arp.set_bpm(120.0);
        arp.start(1.0);

        let mut notes_triggered = std::vec::Vec::new();
        let samples_per_step = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;

        // Run for 4 steps worth of samples
        for _ in 0..(samples_per_step * 4 + 1) {
            if let Some(ArpEvent::NoteOn(note, _vel)) = arp.tick() {
                notes_triggered.push(note);
            }
        }

        // Up pattern: should cycle through 60, 64, 67, 60
        assert_eq!(notes_triggered.len(), 4);
        assert_eq!(notes_triggered[0], 64); // step 1 (start() sets counter to trigger, advance_step first)
        assert_eq!(notes_triggered[1], 67); // step 2
        assert_eq!(notes_triggered[2], 60); // step 3 (wraps)
        assert_eq!(notes_triggered[3], 64); // step 4
    }

    #[test]
    fn test_arp_down_pattern() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60, 64, 67]);
        arp.set_pattern(0.5); // Down
        arp.set_bpm(120.0);
        arp.start(1.0);

        let mut notes_triggered = std::vec::Vec::new();
        let samples_per_step = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;

        for _ in 0..(samples_per_step * 3 + 1) {
            if let Some(ArpEvent::NoteOn(note, _vel)) = arp.tick() {
                notes_triggered.push(note);
            }
        }

        // Down: starts at 0, advance_step goes to num_notes-1, then decrements
        assert_eq!(notes_triggered.len(), 3);
        assert_eq!(notes_triggered[0], 67); // wraps to last
        assert_eq!(notes_triggered[1], 64);
        assert_eq!(notes_triggered[2], 60);
    }

    #[test]
    fn test_arp_gate_off() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60, 64, 67]);
        arp.set_gate(0.5); // 50% gate
        arp.set_bpm(120.0);
        arp.start(1.0);

        let mut got_note_on = false;
        let mut got_note_off = false;
        let samples_per_step = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;

        for _ in 0..(samples_per_step + 1) {
            match arp.tick() {
                Some(ArpEvent::NoteOn(_, _)) => got_note_on = true,
                Some(ArpEvent::NoteOff(_)) => got_note_off = true,
                None => {}
            }
        }

        assert!(got_note_on, "should have triggered a note on");
        assert!(got_note_off, "should have triggered a gate off at 50%");
    }

    #[test]
    fn test_arp_stop_emits_note_off() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60]);
        arp.set_bpm(120.0);
        arp.start(1.0);

        // Tick until we get a note on
        for _ in 0..2 {
            arp.tick();
        }

        // Stop while gate is open should emit note-off
        let evt = arp.stop();
        assert!(matches!(evt, Some(ArpEvent::NoteOff(60))));
        assert!(!arp.is_active());
    }

    #[test]
    fn test_arp_inactive_emits_nothing() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60, 64, 67]);
        // Don't call start()

        for _ in 0..10000 {
            assert!(arp.tick().is_none());
        }
    }

    #[test]
    fn test_arp_updown_pattern() {
        let mut arp = ArpProcessor::new();
        arp.set_notes(&[60, 64, 67, 72]); // 4 notes
        arp.set_pattern(1.0); // UpDown
        arp.set_bpm(120.0);
        arp.start(1.0);

        let mut notes_triggered = std::vec::Vec::new();
        let samples_per_step = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;

        // Run for 7 steps to see full up-down cycle
        for _ in 0..(samples_per_step * 7 + 1) {
            if let Some(ArpEvent::NoteOn(note, _vel)) = arp.tick() {
                notes_triggered.push(note);
            }
        }

        // UpDown with 4 notes: starts at 0, advances up then bounces down
        // advance_step from 0 → 1(64), 2(67), 3(72), bounce→2(67), 1(64), 0(60), bounce→1(64)
        assert!(notes_triggered.len() >= 6);
        assert_eq!(notes_triggered[0], 64); // up
        assert_eq!(notes_triggered[1], 67); // up
        assert_eq!(notes_triggered[2], 72); // peak
        assert_eq!(notes_triggered[3], 67); // down
        assert_eq!(notes_triggered[4], 64); // down
        assert_eq!(notes_triggered[5], 60); // bottom
    }
}
