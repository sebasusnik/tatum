//! `tatum set play <dir> --record <name>`: the take.
//!
//! A take is the set as it was played, in two files: `<name>.wav`, what came
//! out, and `<name>.txt`, every knob, pad, key, strip, step move and scene
//! the performer touched, each on the bar and the sample it landed on, in
//! the script `set render --perform` reads. Rendered again from the script,
//! the take comes back sample for sample.
//!
//! That cannot be had from the plain live session, where the main thread
//! plans what the controller does whenever it wakes and the audio callback
//! applies it at the top of whichever callback comes next: when a knob
//! lands, how a tempo ramp steps and when a step held for its line goes out
//! all follow the wall clock and the device's buffer. So a take is played
//! the way `set render` plays: a [`Stage`] moved on block by block on the
//! main thread, with what arrived from the controller handed in between two
//! blocks, a little ahead of the device. The audio callback only copies
//! samples out of a lock-free ring ([`Ring`]); the blocks also go, with the
//! script's lines, down a channel to a thread that writes the two files.
//! Nothing in the callback allocates, locks or touches a file, and a
//! dropout heard live (the main thread late, a step slow to build) is not
//! in the take: the take is the blocks, not what the device managed to play.
//!
//! The price is latency: what the controller does is heard after the
//! samples already waiting in the ring, about [`LOOKAHEAD`] frames.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use tatum_core::dsl::ast::KeyAction;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::include::Source;
use crate::live::{Status, Ui};
use crate::midi::{Event as Midi, DRUM_CHANNEL};
use crate::resample::Resampler;
use crate::set::{Act, Cue, TakeHeader};
use crate::stage::Stage;

/// Frames the session keeps rendered ahead of the device, at the least. A
/// callback bigger than half of it raises it to two callbacks and a block.
pub const LOOKAHEAD: usize = 1024;

/// Frames to keep in the ring for a device that asks for `widest` frames a
/// callback at `rate` Hz: two callbacks' worth of the engine's frames,
/// rounded up, and a block, or [`LOOKAHEAD`] if that is more.
pub fn lookahead_for(widest: usize, rate: u32) -> usize {
    let engine = (widest as u64 * SAMPLE_RATE as u64).div_ceil(rate.max(1) as u64) as usize;
    LOOKAHEAD.max(engine * 2 + BLOCK_SIZE)
}

/// The most frames `--buffer` lets a callback ask for.
const WIDEST_BUFFER: usize = 8192;

/// Frames the ring holds for a device at `rate` Hz: what [`lookahead_for`]
/// asks for the biggest buffer `--buffer` allows, counted like it in the
/// engine's frames, and a block of room to reach it.
pub fn ring_frames(rate: u32) -> usize {
    lookahead_for(WIDEST_BUFFER, rate) + BLOCK_SIZE
}

/// Seconds of silence in `short` frames the ring did not have. They are the
/// engine's frames, whatever the device's rate (see [`Output::short`]).
pub fn silence_secs(short: u64) -> f64 {
    short as f64 / SAMPLE_RATE as f64
}

// ── The ring the callback reads ─────────────────────────────────────────────

/// Stereo samples from one thread to one other, without a lock: the main
/// thread pushes rendered blocks, the audio callback pulls what it needs.
pub struct Ring {
    l: Box<[AtomicU32]>,
    r: Box<[AtomicU32]>,
    /// Frames ever pushed, and ever pulled. Only the pusher stores the first
    /// and only the puller the second.
    written: AtomicUsize,
    read: AtomicUsize,
}

impl Ring {
    pub fn new(frames: usize) -> Self {
        let n = frames.next_power_of_two();
        let slots = || (0..n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice();
        Self { l: slots(), r: slots(), written: AtomicUsize::new(0), read: AtomicUsize::new(0) }
    }

    /// Frames pushed and not yet pulled.
    pub fn waiting(&self) -> usize {
        self.written.load(Ordering::Acquire).wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// Frames there is room to push.
    pub fn room(&self) -> usize {
        self.l.len() - self.waiting()
    }

    /// Push a block. The caller has checked [`room`](Self::room); what does
    /// not fit is dropped rather than written over what waits.
    pub fn push(&self, l: &[f32], r: &[f32]) {
        let w = self.written.load(Ordering::Relaxed);
        let n = l.len().min(r.len()).min(self.room());
        let mask = self.l.len() - 1;
        for i in 0..n {
            let at = w.wrapping_add(i) & mask;
            self.l[at].store(l[i].to_bits(), Ordering::Relaxed);
            self.r[at].store(r[i].to_bits(), Ordering::Relaxed);
        }
        self.written.store(w.wrapping_add(n), Ordering::Release);
    }

    /// Pull up to `l.len()` frames. Returns how many there were; the rest of
    /// the buffers is left alone.
    pub fn pull(&self, l: &mut [f32], r: &mut [f32]) -> usize {
        let at = self.read.load(Ordering::Relaxed);
        let n = l.len().min(r.len()).min(self.written.load(Ordering::Acquire).wrapping_sub(at));
        let mask = self.l.len() - 1;
        for i in 0..n {
            let k = at.wrapping_add(i) & mask;
            l[i] = f32::from_bits(self.l[k].load(Ordering::Relaxed));
            r[i] = f32::from_bits(self.r[k].load(Ordering::Relaxed));
        }
        self.read.store(at.wrapping_add(n), Ordering::Release);
        n
    }
}

/// The audio callback's side: the ring, and the converter when the device
/// is not at the engine's rate.
pub struct Output {
    ring: Arc<Ring>,
    resampler: Option<Resampler>,
    /// Frames the ring did not have when the device asked: silence played.
    /// Counted in the engine's frames, as they are pulled before the
    /// converter.
    pub short: u64,
}

impl Output {
    pub fn new(ring: Arc<Ring>, resampler: Option<Resampler>) -> Self {
        Self { ring, resampler, short: 0 }
    }

    /// Fill one callback's interleaved stereo buffer. Allocates nothing.
    pub fn fill(&mut self, data: &mut [f32]) {
        let mut l = [0.0f32; BLOCK_SIZE];
        let mut r = [0.0f32; BLOCK_SIZE];
        for chunk in data.chunks_mut(BLOCK_SIZE * 2) {
            let frames = chunk.len() / 2;
            match self.resampler.as_mut() {
                None => {
                    let got = self.ring.pull(&mut l[..frames], &mut r[..frames]);
                    if got < frames {
                        l[got..frames].fill(0.0);
                        r[got..frames].fill(0.0);
                        self.short += (frames - got) as u64;
                    }
                }
                Some(rs) => {
                    while rs.needed(frames) > 0 && rs.can_push() {
                        let mut bl = [0.0f32; BLOCK_SIZE];
                        let mut br = [0.0f32; BLOCK_SIZE];
                        let got = self.ring.pull(&mut bl, &mut br);
                        self.short += (BLOCK_SIZE - got) as u64;
                        rs.push(&bl, &br);
                    }
                    rs.pull(&mut l[..frames], &mut r[..frames]);
                }
            }
            for (i, frame) in chunk.chunks_mut(2).enumerate() {
                frame[0] = l[i];
                if frame.len() > 1 {
                    frame[1] = r[i];
                }
            }
        }
    }
}

// ── What the controller sent, waiting for the next block ────────────────────

/// What arrived since the last block. Notes, pads and the computer's keys
/// keep their order; a knob turned fast is only its last value, and the
/// strip the same, as in the plain session.
pub struct Inbox {
    ordered: Vec<Act>,
    bend: Option<u16>,
    ccs: [Option<u8>; 128],
}

impl Default for Inbox {
    fn default() -> Self {
        Self { ordered: Vec::new(), bend: None, ccs: [None; 128] }
    }
}

impl Inbox {
    /// A message from the controller: the same path for a real one and for
    /// a test.
    pub fn midi(&mut self, event: Midi) {
        match event {
            Midi::Cc(cc, v) => self.ccs[(cc & 0x7f) as usize] = Some(v),
            Midi::Bend(v) => self.bend = Some(v),
            Midi::Note { channel, note, velocity } => self.ordered.push(if channel == DRUM_CHANNEL {
                Act::Pad(note, velocity)
            } else {
                Act::Key(note, velocity)
            }),
            Midi::Pressure { channel, note, value } => {
                self.ordered.push(Act::Pressure(channel == DRUM_CHANNEL, note, value))
            }
            Midi::Other { .. } => {}
        }
    }

    /// Something from the computer's keys or the screen.
    pub fn act(&mut self, act: Act) {
        self.ordered.push(act);
    }

    /// Everything waiting, in the order it acts: notes and keys as they
    /// came, then the strip, then the knobs.
    fn take(&mut self) -> Vec<Act> {
        let mut acts = std::mem::take(&mut self.ordered);
        acts.extend(self.bend.take().map(Act::Bend));
        for (cc, v) in self.ccs.iter_mut().enumerate() {
            if let Some(v) = v.take() {
                acts.push(Act::Cc(cc as u8, v));
            }
        }
        acts
    }
}

// ── The two files ───────────────────────────────────────────────────────────

/// What goes to the writer: interleaved frames, and the script's lines
/// written since the last batch.
struct Batch {
    audio: Vec<f32>,
    lines: String,
}

/// Writes the take on a thread of its own: the WAV as it plays (its header
/// rewritten every few seconds, so a take cut short still opens), and the
/// script. What is written goes in batches of [`CHUNK`] frames, a write
/// call or two each, and the buffers come back to be filled again.
///
/// A file that cannot be written (the disk full, the drive gone) is said
/// once, through [`trouble`](Self::trouble), while the take plays, and is
/// written no more; the other file goes on to the end. The script alone
/// renders the take again, so a WAV lost is not the take lost.
pub struct Recorder {
    tx: Option<Sender<Batch>>,
    /// Buffers the writer is done with.
    spare: Receiver<Batch>,
    /// What the writer could not write, to be said to the performer.
    trouble: Receiver<String>,
    /// The writer is gone and that has not been said yet.
    gone: bool,
    writer: Option<JoinHandle<Result<usize, String>>>,
    /// Frames and lines not yet sent.
    pending: Batch,
    pub wav: PathBuf,
    pub txt: PathBuf,
}

/// Frames sent to the writer at a time, about 0.19 s.
const CHUNK: usize = 8192;

/// `<name>.wav` and `<name>.txt`. A name given with either extension is the
/// same take; any other extension is part of the name (`take.v1` is
/// `take.v1.wav`).
pub fn take_files(name: &Path) -> (PathBuf, PathBuf) {
    let base = match name.extension() {
        Some(e) if e == "wav" || e == "txt" => name.with_extension(""),
        _ => name.to_path_buf(),
    };
    let with = |ext: &str| {
        let mut s = base.clone().into_os_string();
        s.push(".");
        s.push(ext);
        PathBuf::from(s)
    };
    (with("wav"), with("txt"))
}

fn taken(p: &Path) -> String {
    format!("{} exists; a take does not write over another, name this one otherwise", p.display())
}

/// Whether the take's files are free, said before anything slow starts.
/// Creating them is what decides (see [`Recorder::create`]).
pub fn check_free(name: &Path) -> Result<(), String> {
    let (wav, txt) = take_files(name);
    if let Some(dir) = wav.parent().filter(|d| !d.as_os_str().is_empty() && !d.is_dir()) {
        return Err(format!("{} is not a directory: a take is written into one that is there", dir.display()));
    }
    let busy = [&wav, &txt].into_iter().find(|p| p.exists()).map(|p| taken(p));
    busy.map_or(Ok(()), Err)
}

/// Create a file that must not exist yet.
fn create_new(p: &Path) -> Result<File, String> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(p).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            taken(p)
        } else {
            format!("{}: {}", p.display(), e)
        }
    })
}

impl Recorder {
    /// `<name>.wav` and `<name>.txt`, neither of which may exist: a take is
    /// not written over another.
    pub fn create(name: &Path, header: &[String]) -> Result<Self, String> {
        Self::create_limited(name, header, tatum_core::wav::MAX_STEREO_16_FRAMES)
    }

    /// With the WAV stopped after `limit` frames: the format's own limit, or
    /// a small one in a test.
    fn create_limited(name: &Path, header: &[String], limit: usize) -> Result<Self, String> {
        let (wav, txt) = take_files(name);
        let audio_file = create_new(&wav)?;
        let script_file = match create_new(&txt) {
            Ok(f) => f,
            Err(e) => {
                // Not half a take: the WAV just made goes again.
                drop(audio_file);
                let _ = std::fs::remove_file(&wav);
                return Err(e);
            }
        };
        Ok(Self::start(audio_file, script_file, wav, txt, header, limit))
    }

    /// Start the writer on the two files, already made: the take's own, or
    /// ones that fail in a test.
    fn start<A, S>(audio_file: A, script_file: S, wav: PathBuf, txt: PathBuf, header: &[String], limit: usize) -> Self
    where
        A: Write + Seek + Send + 'static,
        S: Write + Send + 'static,
    {
        let (mut audio, mut script) = (BufWriter::new(audio_file), BufWriter::new(script_file));
        let sr = SAMPLE_RATE as u32;
        let wav_name = wav.display().to_string();
        let txt_name = txt.display().to_string();
        let lines = header.to_vec();
        let (tx, rx) = channel::<Batch>();
        let (back, spare) = channel::<Batch>();
        let (tell, trouble) = channel::<String>();
        let writer = std::thread::spawn(move || -> Result<usize, String> {
            // The first error of each file: said once, now, and kept for the
            // end. That file is then left alone and the other goes on.
            let (mut wav_lost, mut txt_lost) = (None::<String>, None::<String>);
            let lose_wav = |lost: &mut Option<String>, e: std::io::Error| {
                let e = format!("{}: {}", wav_name, e);
                let _ = tell.send(format!("{}; the WAV stops here, the script goes on and renders the take again", e));
                *lost = Some(e);
            };
            let lose_txt = |lost: &mut Option<String>, e: std::io::Error| {
                let e = format!("{}: {}", txt_name, e);
                let _ = tell.send(format!("{}; the script stops here, the WAV goes on", e));
                *lost = Some(e);
            };
            if let Err(e) = audio.write_all(&tatum_core::wav::header_stereo_16(0, sr)) {
                lose_wav(&mut wav_lost, e);
            }
            let begun = lines.iter().try_for_each(|l| writeln!(script, "{}", l)).and_then(|_| script.flush());
            if let Err(e) = begun {
                lose_txt(&mut txt_lost, e);
            }
            let mut frames = 0usize;
            let mut since_header = 0usize;
            let mut bytes = Vec::with_capacity(CHUNK * 4);
            for mut batch in rx {
                // Past the format's limit the WAV stops; the script goes on.
                let n = if wav_lost.is_some() { 0 } else { (batch.audio.len() / 2).min(limit - frames) };
                if n > 0 {
                    bytes.clear();
                    for s in &batch.audio[..n * 2] {
                        bytes.extend_from_slice(&tatum_core::wav::sample_16(*s).to_le_bytes());
                    }
                    let wrote = audio.write_all(&bytes).and_then(|_| {
                        frames += n;
                        since_header += n;
                        if since_header < 5 * SAMPLE_RATE as usize {
                            return Ok(());
                        }
                        since_header = 0;
                        rewrite_header(&mut audio, frames, sr)
                    });
                    if let Err(e) = wrote {
                        lose_wav(&mut wav_lost, e);
                    }
                }
                if txt_lost.is_none() && !batch.lines.is_empty() {
                    if let Err(e) = script.write_all(batch.lines.as_bytes()).and_then(|_| script.flush()) {
                        lose_txt(&mut txt_lost, e);
                    }
                }
                batch.audio.clear();
                batch.lines.clear();
                let _ = back.send(batch);
            }
            if wav_lost.is_none() {
                if let Err(e) = rewrite_header(&mut audio, frames, sr) {
                    lose_wav(&mut wav_lost, e);
                }
            }
            if wav_lost.is_some() {
                // What did reach the file, under a header that says so, as
                // far as that can still be written. What the buffer holds
                // is not tried again.
                let (mut file, _) = audio.into_parts();
                let _ = close_short(&mut file, sr);
            }
            if txt_lost.is_none() {
                if let Err(e) = script.flush() {
                    lose_txt(&mut txt_lost, e);
                }
            }
            match (wav_lost, txt_lost) {
                (None, None) => Ok(frames),
                (Some(w), None) => Err(format!(
                    "{}; the WAV is cut short, the script {} is whole and renders the take again",
                    w, txt_name
                )),
                (None, Some(t)) => Err(format!("{}; the script is cut short, the WAV {} is whole", t, wav_name)),
                (Some(w), Some(t)) => Err(format!("{}; {}", w, t)),
            }
        });
        Self {
            tx: Some(tx),
            spare,
            trouble,
            gone: false,
            writer: Some(writer),
            pending: Batch { audio: Vec::with_capacity(CHUNK * 2), lines: String::new() },
            wav,
            txt,
        }
    }

    /// What could not be written since this was last asked, if anything:
    /// the main thread asks between blocks and tells the performer.
    pub fn trouble(&mut self) -> Option<String> {
        if std::mem::take(&mut self.gone) {
            return Some(String::from("the thread writing the take stopped: nothing more of it is written"));
        }
        self.trouble.try_recv().ok()
    }

    pub fn audio(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.pending.audio.push(*a);
            self.pending.audio.push(*b);
        }
        if self.pending.audio.len() >= CHUNK * 2 {
            self.send();
        }
    }

    /// Hand what waits to the writer, and take a buffer it is done with
    /// (or a new one, while none has come back yet).
    fn send(&mut self) {
        if self.pending.audio.is_empty() && self.pending.lines.is_empty() {
            return;
        }
        let Some(tx) = &self.tx else {
            // No writer to hand it to: the buffer is filled again.
            self.pending.audio.clear();
            self.pending.lines.clear();
            return;
        };
        let next = self
            .spare
            .try_recv()
            .unwrap_or_else(|_| Batch { audio: Vec::with_capacity(CHUNK * 2), lines: String::new() });
        let full = std::mem::replace(&mut self.pending, next);
        // The writer only goes before its channel closes by panicking.
        if tx.send(full).is_err() {
            self.tx = None;
            self.gone = true;
        }
    }

    /// A line of the script. It goes with the next batch of audio.
    pub fn line(&mut self, line: &str) {
        self.pending.lines.push_str(line);
        self.pending.lines.push('\n');
    }

    /// Send what is left, close both files and wait for them. Returns the
    /// frames the WAV holds.
    pub fn finish(mut self) -> Result<usize, String> {
        self.send();
        drop(self.tx.take());
        match self.writer.take().map(|w| w.join()) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err(String::from("the thread writing the take panicked")),
            None => Ok(0),
        }
    }
}

/// After a write failed: the header of what the file does hold, which is
/// not what was counted when a write went in part or stayed in the buffer.
fn close_short<W: Write + Seek>(file: &mut W, sr: u32) -> std::io::Result<()> {
    let end = file.seek(SeekFrom::End(0))?;
    let frames = (end.saturating_sub(44) / 4) as usize;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tatum_core::wav::header_stereo_16(frames, sr))?;
    file.flush()
}

fn rewrite_header<W: Write + Seek>(audio: &mut BufWriter<W>, frames: usize, sr: u32) -> std::io::Result<()> {
    audio.flush()?;
    let file = audio.get_mut();
    let end = file.stream_position()?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tatum_core::wav::header_stereo_16(frames, sr))?;
    file.seek(SeekFrom::Start(end))?;
    Ok(())
}

/// The line a take writes for an act: `12.503906@529152  cc 74 64`, and
/// what it did, after a `#`, for the reader.
fn script_line(bar: f64, sample: usize, act: &Act, said: &str) -> String {
    let line = Cue { bar, sample: Some(sample), act: act.clone() }.line();
    if said.is_empty() {
        line
    } else {
        format!("{:<36}# {}", line, said.replace(['\n', '\r'], " "))
    }
}

// ── The session, without the device ─────────────────────────────────────────

/// What a block said, for the screen or the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// What a knob or the strip reads now.
    Reading(String),
    Info(String),
    Good(String),
    /// Something went wrong that the performer has to know now.
    Bad(String),
}

/// A take being played: the stage, what waits for it, the ring and the
/// files. The device's callback and the controller are outside; a test
/// drives this the same way the session does.
pub struct Session {
    pub stage: Stage,
    pub inbox: Inbox,
    ring: Arc<Ring>,
    recorder: Option<Recorder>,
    /// Frames to keep in the ring.
    pub lookahead: usize,
}

impl Session {
    pub fn new(stage: Stage, ring: Arc<Ring>, recorder: Option<Recorder>) -> Self {
        Self { stage, inbox: Inbox::default(), ring, recorder, lookahead: LOOKAHEAD }
    }

    /// Render blocks until `lookahead` frames wait for the device. `each` is
    /// handed every block and the stage after it, for the screen's meters.
    pub fn render_ahead(&mut self, mut each: impl FnMut(&[f32], &[f32], &mut Stage)) -> Result<Vec<Said>, String> {
        let mut said = Vec::new();
        while self.ring.waiting() < self.lookahead && self.ring.room() >= BLOCK_SIZE {
            let (l, r) = self.block(&mut said)?;
            each(&l, &r, &mut self.stage);
        }
        // A file that stopped being written: said while the take plays.
        if let Some(rec) = self.recorder.as_mut() {
            said.extend(std::iter::from_fn(|| rec.trouble()).map(Said::Bad));
        }
        Ok(said)
    }

    /// One block: what waits acts on its first sample, then it renders.
    fn block(&mut self, said: &mut Vec<Said>) -> Result<([f32; BLOCK_SIZE], [f32; BLOCK_SIZE]), String> {
        let acts = self.inbox.take();
        let (bar, sample) = (self.stage.position(), self.stage.frames);
        for act in &acts {
            let s = self.stage.act(act);
            if let Some(rec) = self.recorder.as_mut() {
                rec.line(&script_line(bar, sample, act, &s));
            }
            if !s.is_empty() {
                said.push(match act {
                    Act::Cc(..) | Act::Bend(_) => Said::Reading(s),
                    _ => Said::Info(s),
                });
            }
        }
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let turn = self.stage.block(&mut l, &mut r)?;
        said.extend(turn.tick.into_iter().filter(|s| !s.starts_with("now ")).map(Said::Info));
        said.extend(turn.log.into_iter().map(Said::Good));
        self.ring.push(&l, &r);
        if let Some(rec) = self.recorder.as_mut() {
            rec.audio(&l, &r);
        }
        // Engines the player is done with are freed here, off the device's
        // thread, as the plain session frees them on its main thread.
        while self.stage.player.take_retired().is_some() {}
        Ok((l, r))
    }

    /// The take is over: its `end` on the sample it stopped on, and both
    /// files closed. Returns the files, the frames the WAV holds and the
    /// frames played, which are more only past the WAV format's limit.
    pub fn finish(mut self) -> Result<Option<(PathBuf, PathBuf, usize, usize)>, String> {
        let Some(mut rec) = self.recorder.take() else { return Ok(None) };
        rec.line(&script_line(self.stage.position(), self.stage.frames, &Act::End, ""));
        let (wav, txt) = (rec.wav.clone(), rec.txt.clone());
        let frames = rec.finish()?;
        Ok(Some((wav, txt, frames, self.stage.frames)))
    }

    /// The take never started: its two files, empty still, go again, so
    /// the same name can be asked for once more.
    pub fn discard(mut self) {
        let Some(rec) = self.recorder.take() else { return };
        let (wav, txt) = (rec.wav.clone(), rec.txt.clone());
        let _ = rec.finish();
        let _ = std::fs::remove_file(wav);
        let _ = std::fs::remove_file(txt);
    }
}

// ── The live session ────────────────────────────────────────────────────────

/// What the callback counts for the summary.
#[derive(Default)]
struct Counters {
    callbacks: AtomicU64,
    frames: AtomicU64,
    /// The most frames one callback asked for.
    widest: AtomicU64,
    worst_ns: AtomicU64,
    late: AtomicU64,
    short: AtomicU64,
}

/// Play the set and record it. `walk` and `header` are how it is played.
#[allow(clippy::too_many_arguments)]
pub fn run(
    steps: Vec<crate::set::Step>,
    walk: &crate::set::Walk,
    header: &TakeHeader,
    name: &Path,
    device_name: Option<&str>,
    forced_rate: Option<u32>,
    midi_name: Option<&str>,
    tui: bool,
    glass: bool,
) -> Result<(), String> {
    let set_name = steps[0].path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    // Said before the set is measured, which takes a while.
    check_free(name)?;
    let mut stage = Stage::new(steps, walk)?;
    // A performer counts bars from 1.
    stage.script_count = false;
    let device = crate::live::open_device(device_name)?;
    let (mut config, rate) = crate::live::pick_config(&device, forced_rate)?;
    if let Some(&frames) = crate::live::BUFFER.get() {
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    let resampler = if rate == SAMPLE_RATE as u32 { None } else { Some(Resampler::new(SAMPLE_RATE as u32, rate)) };
    let device_desc = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "?".into());
    // Room for the biggest buffer `--buffer` allows, twice, at this rate.
    let ring = Arc::new(Ring::new(ring_frames(rate)));
    let counters = Arc::new(Counters::default());
    let counters_cb = Arc::clone(&counters);
    let mut output = Output::new(Arc::clone(&ring), resampler);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let t0 = Instant::now();
                let before = output.short;
                output.fill(data);
                let frames = (data.len() / 2) as u64;
                counters_cb.callbacks.fetch_add(1, Ordering::Relaxed);
                counters_cb.frames.fetch_add(frames, Ordering::Relaxed);
                counters_cb.widest.fetch_max(frames, Ordering::Relaxed);
                counters_cb.short.fetch_add(output.short - before, Ordering::Relaxed);
                let ns = t0.elapsed().as_nanos() as u64;
                counters_cb.worst_ns.fetch_max(ns, Ordering::Relaxed);
                if ns > frames * 1_000_000_000 / rate as u64 {
                    counters_cb.late.fetch_add(1, Ordering::Relaxed);
                }
            },
            |_err| {
                crate::live::STREAM_ERRORS.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("cannot open output stream: {}", e))?;
    let (midi_tx, midi_rx) = channel::<Midi>();
    let inputs = crate::midi::open(midi_name, &midi_tx)?;
    // The files last. What can still fail before the take starts (the first
    // blocks, the device, the screen) discards them again, so no empty take
    // is left behind to refuse the name next time.
    // The `# take:` line first: only there is it read as one.
    let lines = vec![
        header.line(),
        format!("# a take of {}, from `tatum set play --record`", set_name),
        String::from("# set render <dir> --perform <this file> plays it back, sample for sample"),
    ];
    let recorder = Recorder::create(name, &lines)?;
    let (wav, txt) = (recorder.wav.clone(), recorder.txt.clone());
    let mut session = Session::new(stage, Arc::clone(&ring), Some(recorder));
    let telemetry = tui.then(|| Arc::new(crate::tui::Telemetry::new()));
    let meter = |l: &[f32], r: &[f32], stage: &mut Stage| {
        if let Some(t) = &telemetry {
            t.push(l, r);
            if let Some(e) = stage.player.engine_mut() {
                e.set_band_metering(true);
                t.meter(e);
            }
        }
    };
    // Sound waiting before the device asks for any: enough for the buffer
    // asked for, which the device has not shown yet.
    session.lookahead = lookahead_for(crate::live::BUFFER.get().map_or(0, |&n| n as usize), rate);
    let begun = session
        .render_ahead(meter)
        .and_then(|_| stream.play().map_err(|e| format!("cannot start output stream: {}", e)))
        .and_then(|_| match &telemetry {
            Some(t) => {
                let mut screen = crate::tui::Screen::new(Arc::clone(t), device_desc.clone(), inputs.names.clone())
                    .map_err(|e| format!("cannot open the screen: {}", e))?;
                screen.glass = glass;
                screen.say(format!("recording a take to {}", wav.display()), crate::tui::Tone::Info);
                Ok(Ui::Screen(Box::new(screen)))
            }
            None => Ok(Ui::Plain(Status::new())),
        });
    let mut ui = match begun {
        Ok(ui) => ui,
        Err(e) => {
            session.discard();
            return Err(e);
        }
    };
    let (key_tx, key_rx) = channel::<crate::keys::Key>();
    let _keys = if let Ui::Plain(_) = ui {
        let nav = &session.stage.nav;
        eprintln!("playing {} on {}, recording to {} and {}", set_name, device_desc, wav.display(), txt.display());
        if !inputs.names.is_empty() {
            eprintln!("  midi: {}", inputs.names.join(", "));
        }
        eprintln!("  steps, {} bars to a phrase; a move lands on the next phrase line:", nav.phrase);
        for i in 0..nav.steps.len() {
            eprintln!("    {}", nav.describe(i));
        }
        eprintln!("  space or → next, ← back, 1-9 a step, q quits and closes the take; the files are not watched");
        crate::live::print_keyboard(&session.stage.planner);
        Some(crate::keys::spawn(key_tx))
    } else {
        None
    };
    let song = |nav: &crate::setnav::SetNav, ui: &mut Ui| {
        if let Ok(s) = Source::load(&nav.path()) {
            ui.song(&nav.describe(nav.current), &s);
        }
    };
    song(&session.stage.nav, &mut ui);

    let started = Instant::now();
    let poll_every = Duration::from_millis(if ui.is_screen() { 33 } else { 50 });
    let mut next_poll = Instant::now() + poll_every;
    let mut shown_step = session.stage.nav.current;
    let mut knob_moves = 0u32;
    let result = loop {
        // A short wait: the ring is refilled every time round.
        let first = match midi_rx.recv_timeout(Duration::from_millis(2)) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        };
        let now_s = started.elapsed().as_secs_f32();
        for event in first.into_iter().chain(std::iter::from_fn(|| midi_rx.try_recv().ok())) {
            if let Ui::Screen(screen) = &mut ui {
                let planner = &session.stage.planner;
                let what = planner.song().and_then(|s| crate::midi::meaning(s, Some(planner), &event));
                screen.midi_in(now_s, crate::midi::describe(&event), what);
            }
            session.inbox.midi(event);
        }
        // Two callbacks and a block ahead of the widest the device asked for.
        session.lookahead = lookahead_for(counters.widest.load(Ordering::Relaxed) as usize, rate);
        let said = match session.render_ahead(meter) {
            Ok(s) => s,
            Err(e) => break Err(e),
        };
        let mut readings = Vec::new();
        for s in said {
            match s {
                Said::Reading(r) => {
                    knob_moves += 1;
                    readings.push(r);
                }
                Said::Info(line) => ui.say(now_s, &line, crate::tui::Tone::Info),
                Said::Good(line) => ui.say(now_s, &line, crate::tui::Tone::Good),
                Said::Bad(line) => ui.say(now_s, &line, crate::tui::Tone::Bad),
            }
        }
        if !readings.is_empty() {
            ui.readings(readings);
        }
        if Instant::now() < next_poll {
            continue;
        }
        next_poll = Instant::now() + poll_every;

        let mut quit = false;
        let mut keys: Vec<crate::keys::Key> = key_rx.try_iter().collect();
        if let Ui::Screen(screen) = &mut ui {
            for k in screen.keys() {
                match k {
                    crate::tui::Key::Quit => keys.push(crate::keys::Key::Quit),
                    crate::tui::Key::Next => keys.push(crate::keys::Key::Next),
                    crate::tui::Key::Prev => keys.push(crate::keys::Key::Prev),
                    crate::tui::Key::Step(n) => keys.push(crate::keys::Key::Step(n)),
                    crate::tui::Key::Loop => keys.push(crate::keys::Key::Loop),
                    crate::tui::Key::LoopUp => keys.push(crate::keys::Key::LoopUp),
                    crate::tui::Key::Keep => keys.push(crate::keys::Key::Keep),
                    crate::tui::Key::Named(n) => keys.push(crate::keys::Key::Named(n)),
                    crate::tui::Key::Mute => {
                        for (name, _) in screen.selected_tracks() {
                            session.inbox.act(Act::Mute(name));
                        }
                    }
                    crate::tui::Key::Solo => {
                        let names: Vec<String> = screen.selected_tracks().into_iter().map(|(n, _)| n).collect();
                        if !names.is_empty() {
                            session.inbox.act(Act::Solo(names));
                        }
                    }
                    // The take plays the files as they are: an edit would
                    // be heard and not be in it.
                    crate::tui::Key::Edit(_) | crate::tui::Key::Knob(_) | crate::tui::Key::Undo => {
                        screen.say("not while recording: a take plays the files as they are", crate::tui::Tone::Bad)
                    }
                }
            }
            screen.soloed = session.stage.planner.soloed().to_vec();
        }
        let (bindings, _) = session.stage.planner.keyboard();
        for key in keys {
            let key = match key {
                crate::keys::Key::Named(name) => {
                    match bindings.iter().find(|b| b.key == name.word()).map(|b| &b.action) {
                        Some(KeyAction::Reset) => {
                            session.inbox.act(Act::Reset);
                            continue;
                        }
                        Some(KeyAction::Voice(mv)) => {
                            session.inbox.act(Act::Voice(mv.clone()));
                            continue;
                        }
                        Some(KeyAction::Perform(scene)) => {
                            session.inbox.act(Act::Perform(scene.clone()));
                            continue;
                        }
                        Some(KeyAction::Next) => crate::keys::Key::Next,
                        Some(KeyAction::Prev) => crate::keys::Key::Prev,
                        Some(KeyAction::Step(n)) => crate::keys::Key::Step(*n),
                        Some(KeyAction::Loop) => crate::keys::Key::Loop,
                        None => match name.default_move() {
                            Some(k) => k,
                            None => continue,
                        },
                    }
                }
                k => k,
            };
            match key {
                crate::keys::Key::Quit => quit = true,
                crate::keys::Key::Next => session.inbox.act(Act::Next),
                crate::keys::Key::Prev => session.inbox.act(Act::Prev),
                crate::keys::Key::Step(n) => session.inbox.act(Act::Step(n)),
                crate::keys::Key::Loop => session.inbox.act(Act::Loop(true)),
                crate::keys::Key::LoopUp => session.inbox.act(Act::Loop(false)),
                crate::keys::Key::Keep => session.inbox.act(Act::Keep),
                crate::keys::Key::Named(_) => {}
            }
        }
        if quit {
            break Ok(());
        }
        let stage = &session.stage;
        let nav = &stage.nav;
        if nav.current != shown_step {
            shown_step = nav.current;
            song(nav, &mut ui);
        }
        let bar = stage.bar();
        if let Some((step, bars)) = nav.waiting(bar) {
            let line = format!(
                "bar {} · next {} in {} bar{}",
                bar + 1,
                nav.describe(step),
                bars,
                if bars == 1 { "" } else { "s" }
            );
            ui.waiting(&line);
        }
        if let Ui::Screen(screen) = &mut ui {
            let planner = &stage.planner;
            screen.bound = bindings.iter().map(|b| b.key.clone()).collect();
            screen.voice = planner.voice_name();
            screen.pages = planner.pages();
            screen.pad_voices = planner.waiting_for_pads();
            screen.perform = planner.scene().map(String::from);
            let step = &nav.steps[nav.current];
            screen.set = Some(crate::tui::SetView {
                steps: nav.steps.iter().map(|s| s.name()).collect(),
                current: nav.current,
                next: nav.waiting(bar),
                phrase: nav.phrase,
                bar: nav.bar_in_step(bar),
                bars: step.length(),
                cues: step.cues.clone(),
                moves_in: nav.moves_in(bar),
                capped: step.max.filter(|_| step.auto.is_none()),
                looping: nav.looping,
                pass: nav.pass(bar),
                bar_loop: nav.loop_view(),
            });
            if let Some(e) = stage.player.engine() {
                screen.position(e.current_bar(), e.global_step(), e.tempo());
            }
            if let Err(e) = screen.draw() {
                break Err(format!("cannot draw the screen: {}", e));
            }
        }
    };
    drop(inputs);
    drop(stream);
    drop(ui);
    let closed = session.finish();
    result?;
    let (wav, txt, frames, played_frames) = closed?.expect("a live take records");
    if frames < played_frames {
        let secs = frames / SAMPLE_RATE as usize;
        eprintln!(
            "the WAV stops at {}:{:02}:{:02}, as long as a WAV file can be; the script goes on to the end",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        );
    }

    let callbacks = counters.callbacks.load(Ordering::Relaxed);
    let played = counters.frames.load(Ordering::Relaxed);
    let worst = counters.worst_ns.load(Ordering::Relaxed) as f64 / 1e6;
    let per_cb = if callbacks > 0 { played as f64 / callbacks as f64 } else { 0.0 };
    eprintln!(
        "played {:.1} s in {} callbacks of ~{:.0} frames, worst {:.3} ms, {} late",
        played as f64 / rate as f64,
        callbacks,
        per_cb,
        worst,
        counters.late.load(Ordering::Relaxed)
    );
    let short = counters.short.load(Ordering::Relaxed);
    if short > 0 {
        eprintln!(
            "{:.2} s of silence played where the set was not rendered in time (the take has no gap: it is the set, not the device)",
            silence_secs(short)
        );
    }
    if knob_moves > 0 {
        eprintln!("{} knob moves", knob_moves);
    }
    let secs = frames as f32 / SAMPLE_RATE;
    eprintln!("wrote {} ({}:{:02}) and {}", wav.display(), (secs / 60.0) as u32, (secs % 60.0) as u32, txt.display());
    eprintln!("  tatum set render {} --perform {} -o again.wav  renders it again", set_name, txt.display());
    Ok(())
}

#[cfg(test)]
mod tests;
