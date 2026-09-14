import { createMemo } from "solid-js";
import ActivityLed from "./ActivityLed";
import PatternVis from "./PatternVis";
import {
  trackInfos, getPatternForTrack, getModuleNameForTrack,
  setTrackLevelRT, persistTrackProp,
} from "../stores/synth";

const KIND_COLORS: Record<string, string> = {
  bass: "#ff6b35", keys: "#00c9b1", fm: "#ffd23f", beats: "#ff5ea0",
};

interface Props {
  trackName: string;
  trackIdx: number;
  selected: boolean;
  onSelect: () => void;
}

export default function TrackCard(props: Props) {
  const track = createMemo(() => trackInfos().find(t => t.name === props.trackName));
  const pattern = createMemo(() => getPatternForTrack(props.trackName));
  const moduleName = createMemo(() => getModuleNameForTrack(props.trackName));
  const color = createMemo(() => KIND_COLORS[track()?.kind ?? ""] ?? "#888");

  let dragging = false;

  function onVolDown(e: PointerEvent) {
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    updateVol(e);
  }
  function onVolMove(e: PointerEvent) {
    if (!dragging) return;
    updateVol(e);
  }
  function onVolUp() {
    dragging = false;
    const t = track();
    if (t) persistTrackProp(t.name, "level", t.level);
  }
  function updateVol(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    setTrackLevelRT(props.trackIdx, pct);
  }

  return (
    <div
      classList={{ "track-card": true, selected: props.selected }}
      style={{ "--c": color() }}
      onClick={props.onSelect}
    >
      <div class="track-card-top">
        <ActivityLed pattern={pattern()} color={color()} />
        <span class="track-card-name" style={{ color: color() }}>{props.trackName}</span>
        <span class="track-card-module">{moduleName()}</span>
        <span class="track-card-vol">{Math.round((track()?.level ?? 0) * 100)}</span>
      </div>
      <div
        class="track-card-fader"
        onPointerDown={onVolDown}
        onPointerMove={onVolMove}
        onPointerUp={onVolUp}
      >
        <div class="track-card-fader-fill" style={{ width: `${(track()?.level ?? 0) * 100}%` }} />
        <div class="track-card-fader-thumb" style={{ left: `${(track()?.level ?? 0) * 100}%` }} />
      </div>
      <PatternVis pattern={pattern()} color={color()} />
    </div>
  );
}
