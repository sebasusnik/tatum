import { createMemo, For, Show } from "solid-js";
import Knob from "./Knob";
import {
  trackInfos,
  getModulePagesForTrack, getModuleNameForTrack, setModuleParamForTrack,
} from "../stores/tatum";

const KIND_COLORS: Record<string, string> = {
  bass: "#ff6b35", keys: "#00c9b1", fm: "#ffd23f", beats: "#ff5ea0",
};

interface Props {
  trackName: string | null;
}

export default function DetailDrawer(props: Props) {
  const track = createMemo(() => {
    if (!props.trackName) return null;
    return trackInfos().find(t => t.name === props.trackName);
  });

  const pages = createMemo(() => {
    if (!props.trackName) return [];
    return getModulePagesForTrack(props.trackName);
  });

  const moduleName = createMemo(() => {
    if (!props.trackName) return "";
    return getModuleNameForTrack(props.trackName);
  });

  const color = createMemo(() => KIND_COLORS[track()?.kind ?? ""] ?? "#888");

  return (
    <Show when={props.trackName && track()}>
      <div class="detail-drawer">
        <div class="detail-drawer-params">
          <div class="detail-drawer-head">
            <div class="detail-drawer-dot" style={{ background: color() }} />
            <span class="detail-drawer-name">{props.trackName}</span>
            <span class="detail-drawer-sub">{moduleName()} · {track()?.kind}</span>
          </div>
          <div class="detail-drawer-knobs">
            <For each={pages()}>
              {(page, pi) => (
                <For each={page.values}>
                  {(val, ki) => (
                    <Knob
                      value={val}
                      label={page.keys[ki()] ?? `P${pi()}.${ki()}`}
                      color={color()}
                      size={32}
                      onChange={(v) => setModuleParamForTrack(props.trackName!, pi(), ki(), v)}
                    />
                  )}
                </For>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
}
