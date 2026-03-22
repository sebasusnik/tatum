import {
  playing, recording, bpm, setRecording,
  startPlayback, stopPlayback, adjustBpm, persistBpm,
} from "../stores/synth";

export default function Transport() {
  let bpmDragging = false;
  let startY = 0;
  let startBpm = 0;

  function onBpmPointerDown(e: PointerEvent) {
    bpmDragging = true;
    startY = e.clientY;
    startBpm = bpm();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onBpmPointerMove(e: PointerEvent) {
    if (!bpmDragging) return;
    const delta = (startY - e.clientY) * 0.5;
    adjustBpm(Math.round(startBpm + delta) - bpm());
  }

  function onBpmPointerUp() {
    if (bpmDragging) {
      bpmDragging = false;
      persistBpm(); // update DSL text with final BPM value
    }
  }

  return (
    <div class="top-row">
      <div class="logo">synth</div>
      <div class="transport">
        <button
          class="t-btn"
          classList={{ "rec-active": recording() }}
          title="Motion Record"
          onClick={() => setRecording((r) => !r)}
        >
          &#9679;
        </button>
        <button class="t-btn" title="Stop" onClick={stopPlayback}>
          &#9632;
        </button>
        <button
          class="t-btn"
          classList={{ playing: playing() }}
          title="Play"
          onClick={startPlayback}
        >
          &#9654;
        </button>
        <div
          class="bpm-pill"
          onPointerDown={onBpmPointerDown}
          onPointerMove={onBpmPointerMove}
          onPointerUp={onBpmPointerUp}
        >
          <span>{bpm()}</span>
          <small>BPM</small>
        </div>
      </div>
    </div>
  );
}
