import { createStore } from "solid-js/store";
import { createEffect, createSignal, createMemo, For, Show, onMount, onCleanup, type JSX } from "solid-js";
import BassBlock from "./BassBlock";
import KeysBlock from "./KeysBlock";
import FmBlock from "./FmBlock";
import BeatsBlock from "./BeatsBlock";
import { DelayBlock, ReverbBlock } from "./EffectsRack";
import MasterStrip from "./MasterStrip";
import FxBlock from "./FxBlock";
import FxPalette from "./FxPalette";
import {
  useGraph, setFxNodePosition,
  edgeInsertInfo,
  type GraphNode as GNode, type GraphEdge,
} from "../stores/graph";
import {
  tracksFx, masterFx,
  addTrackInsertFx, clearTrackInsertFx, toggleTrackInsertFx,
  addMasterInsertFx, clearMasterInsertFx, toggleMasterInsertFx,
  setTrackInsertFxParam, setMasterInsertFxParam,
  setTracksFx, setMasterFx,
  FX_LABELS,
  trackInfos,
  type FxTypeId,
  type TrackInfo,
} from "../stores/synth";

// ── Types ────────────────────────────────────────────

type BlockId = string;

// ── Persistence ──────────────────────────────────────

function load<T>(key: string, fb: T): T {
  try {
    const s = localStorage.getItem(key);
    if (s) return JSON.parse(s);
  } catch {}
  return fb;
}

// ── Canvas state ─────────────────────────────────────

const SNAP = 20;
const snap = (v: number) => Math.round(v / SNAP) * SNAP;

/** Generate default positions for dynamic tracks + fixed nodes */
function defaultPositions(tracks: TrackInfo[]): Record<string, { x: number; y: number }> {
  const positions: Record<string, { x: number; y: number }> = {};
  // Instrument tracks: stack vertically on the left
  tracks.forEach((t, i) => {
    positions[t.name] = { x: 40, y: 20 + i * 320 };
  });
  // Fixed effect/master nodes on the right
  const lastY = tracks.length > 0 ? 20 + (tracks.length - 1) * 320 : 0;
  positions["delay"] = { x: 700, y: Math.max(800, lastY - 160) };
  positions["reverb"] = { x: 700, y: Math.max(960, lastY) };
  positions["master"] = { x: 700, y: 20 };
  return positions;
}

// ── Block sizes (approximate, for connection routing) ─

const KIND_SIZES: Record<string, { w: number; h: number }> = {
  bass:   { w: 210, h: 340 },
  keys:   { w: 210, h: 290 },
  fm:     { w: 210, h: 440 },
  beats:  { w: 250, h: 340 },
  graph:  { w: 210, h: 200 },
  unknown: { w: 210, h: 200 },
};
const FIXED_SIZES: Record<string, { w: number; h: number }> = {
  delay:  { w: 150, h: 155 },
  reverb: { w: 150, h: 140 },
  master: { w: 300, h: 130 },
};

function getSize(id: string, tracks: TrackInfo[]): { w: number; h: number } {
  if (FIXED_getSize(id, trackInfos())) return FIXED_getSize(id, trackInfos());
  const track = tracks.find((t) => t.name === id);
  if (track) return KIND_SIZES[track.kind] || KIND_SIZES.unknown;
  return KIND_SIZES.unknown;
}

const FX_NODE_SIZE = { w: 150, h: 140 };

// ── Canvas state ─────────────────────────────────────

function loadPositions(tracks: TrackInfo[]): Record<string, { x: number; y: number }> {
  const saved = load<Record<string, { x: number; y: number }>>("synth-canvas-pos", {});
  const defaults = defaultPositions(tracks);
  const merged = { ...defaults };
  for (const id of Object.keys(defaults)) {
    if (saved[id]) merged[id] = saved[id];
  }
  return merged;
}

const [pos, setPos] = createStore<Record<string, { x: number; y: number }>>(
  loadPositions([])
);

// Re-layout when tracks change
createEffect(() => {
  const tracks = trackInfos();
  if (tracks.length > 0) {
    const newPos = loadPositions(tracks);
    for (const [id, p] of Object.entries(newPos)) {
      if (!pos[id]) setPos(id, p);
    }
  }
});

const KIND_COLORS: Record<string, string> = {
  bass: "#ff6b35",
  keys: "#00c9b1",
  fm: "#ffd23f",
  beats: "#ff5ea0",
  graph: "#88aaff",
  unknown: "#888888",
};

function BlockForKind(props: { kind: string }) {
  switch (props.kind) {
    case "bass": return <BassBlock />;
    case "keys": return <KeysBlock />;
    case "fm": return <FmBlock />;
    case "beats": return <BeatsBlock />;
    default: return <div class="block-placeholder">{props.kind}</div>;
  }
}

const [cam, setCam] = createStore(
  load("synth-canvas-cam", { tx: 0, ty: 0, zoom: 1 })
);

// Persist changes
createEffect(() => {
  const snapshot: Record<string, { x: number; y: number }> = {};
  for (const id of Object.keys(pos)) {
    if (pos[id]) snapshot[id] = { x: pos[id].x, y: pos[id].y };
  }
  localStorage.setItem("synth-canvas-pos", JSON.stringify(snapshot));
});

createEffect(() => {
  localStorage.setItem("synth-canvas-cam",
    JSON.stringify({ tx: cam.tx, ty: cam.ty, zoom: cam.zoom }));
});

const allIds = createMemo(() => {
  const tracks = trackInfos();
  return [...tracks.map((t) => t.name), "delay", "reverb", "master"];
});

function resetPositions(): void {
  const defaults = defaultPositions(trackInfos());
  for (const id of Object.keys(defaults)) {
    setPos(id, defaults[id]);
  }
  setCam({ tx: 0, ty: 0, zoom: 1 });
}

function fitAll(vpWidth: number, vpHeight: number): void {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const id of allIds()) {
    const p = pos[id];
    const s = getSize(id, trackInfos());
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x + s.w > maxX) maxX = p.x + s.w;
    if (p.y + s.h > maxY) maxY = p.y + s.h;
  }
  const cw = maxX - minX;
  const ch = maxY - minY;
  if (cw <= 0 || ch <= 0) return;
  const pad = 40;
  const z = Math.min(2, Math.max(0.25, Math.min(
    (vpWidth - pad * 2) / cw,
    (vpHeight - pad * 2) / ch,
  )));
  const cx = minX + cw / 2;
  const cy = minY + ch / 2;
  setCam({
    zoom: z,
    tx: vpWidth / 2 - cx * z,
    ty: vpHeight / 2 - cy * z,
  });
}

// ── Node position helpers ────────────────────────────

function getBlockPos(id: string) {
  const blockId = id as BlockId;
  if (pos[blockId]) {
    const p = pos[blockId];
    const s = getSize(blockId, trackInfos()) || FX_NODE_SIZE;
    return { x: p.x, y: p.y, w: s.w, h: s.h };
  }
  return { x: 0, y: 0, w: 120, h: 40 };
}

// ── Wire path computation ────────────────────────────

function getNodeCenter(
  nodeId: string,
  side: "out" | "in",
  fxPositions: Record<string, { x: number; y: number }>,
): { x: number; y: number } {
  // Check if it's a block
  const blockId = nodeId as BlockId;
  if (pos[blockId]) {
    const p = pos[blockId];
    const s = getSize(blockId, trackInfos());
    if (side === "out") {
      return { x: p.x + s.w, y: p.y + 16 + (s.h - 16) / 2 };
    }
    return { x: p.x, y: p.y + 16 + (s.h - 16) / 2 };
  }

  // Master internal ports
  if (nodeId === "master-in") {
    const p = pos.master;
    return { x: p.x, y: p.y + 16 + (SIZES.master.h - 16) / 2 };
  }
  if (nodeId === "master-out") {
    const p = pos.master;
    return { x: p.x + SIZES.master.w, y: p.y + 16 + (SIZES.master.h - 16) / 2 };
  }

  // FX node
  const fxPos = fxPositions[nodeId];
  if (fxPos) {
    if (side === "out") {
      return { x: fxPos.x + FX_NODE_SIZE.w, y: fxPos.y + FX_NODE_SIZE.h / 2 };
    }
    return { x: fxPos.x, y: fxPos.y + FX_NODE_SIZE.h / 2 };
  }

  return { x: 0, y: 0 };
}

function wirePath(
  from: { x: number; y: number },
  to: { x: number; y: number },
): string {
  const dx = Math.max(40, Math.abs(to.x - from.x) * 0.4);
  return `M ${from.x} ${from.y} C ${from.x + dx} ${from.y}, ${to.x - dx} ${to.y}, ${to.x} ${to.y}`;
}

function wireMidpoint(
  from: { x: number; y: number },
  to: { x: number; y: number },
): { x: number; y: number } {
  // Bezier midpoint approximation
  return { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2 };
}

// ── Canvas Node ──────────────────────────────────────

function Node(props: {
  id: BlockId;
  label: string;
  color: string;
  showOutputPort?: boolean;
  showInputPort?: boolean;
  children: JSX.Element;
}) {
  const p = () => pos[props.id];

  function onGrab(e: PointerEvent) {
    e.stopPropagation();
    e.preventDefault();

    const sx = e.clientX;
    const sy = e.clientY;
    const ox = p().x;
    const oy = p().y;

    function onMove(me: PointerEvent) {
      setPos(props.id, {
        x: snap(ox + (me.clientX - sx) / cam.zoom),
        y: snap(oy + (me.clientY - sy) / cam.zoom),
      });
    }

    function onUp() {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    }

    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  }

  return (
    <div
      class="canvas-node"
      style={{ transform: `translate(${p().x}px, ${p().y}px)` }}
    >
      <Show when={props.showInputPort}>
        <div class="graph-port graph-port-in" style={{ "--port-color": props.color }} />
      </Show>
      <div
        class="canvas-node-handle"
        style={{ "--node-color": props.color }}
        onPointerDown={onGrab}
      >
        <span class="canvas-node-label">{props.label}</span>
      </div>
      {props.children}
      <Show when={props.showOutputPort}>
        <div class="graph-port graph-port-out" style={{ "--port-color": props.color }} />
      </Show>
    </div>
  );
}

// ── FX Canvas Node (small, draggable) ────────────────

function FxCanvasNode(props: {
  node: GNode;
  x: number;
  y: number;
  params?: Record<string, number>;
  onToggle: () => void;
  onRemove: () => void;
  onParamChange?: (key: string, paramIdx: number, value: number) => void;
}) {
  const enabled = () => props.node.enabled ?? true;
  const label = () => FX_LABELS[props.node.fxType!] ?? "FX";

  function onGrab(e: PointerEvent) {
    if ((e.target as HTMLElement).closest(".enc-wheel, .graph-fx-btn")) return;
    e.stopPropagation();
    e.preventDefault();

    const sx = e.clientX;
    const sy = e.clientY;
    const ox = props.x;
    const oy = props.y;

    function onMove(me: PointerEvent) {
      setFxNodePosition(
        props.node.id,
        snap(ox + (me.clientX - sx) / cam.zoom),
        snap(oy + (me.clientY - sy) / cam.zoom),
      );
    }

    function onUp() {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    }

    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  }

  return (
    <div
      class="canvas-node"
      classList={{ bypassed: !enabled() }}
      style={{ transform: `translate(${props.x}px, ${props.y}px)` }}
    >
      <div class="graph-port graph-port-in" style={{ "--port-color": props.node.color }} />
      <div
        class="canvas-node-handle"
        style={{ "--node-color": props.node.color }}
        onPointerDown={onGrab}
      >
        <span class="canvas-node-label">{label()}</span>
        <div class="canvas-node-actions">
          <button
            class="graph-fx-btn"
            classList={{ off: !enabled() }}
            onClick={(e) => { e.stopPropagation(); props.onToggle(); }}
          >
            {enabled() ? "ON" : "OFF"}
          </button>
          <button
            class="graph-fx-btn graph-fx-remove"
            onClick={(e) => { e.stopPropagation(); props.onRemove(); }}
          >
            x
          </button>
        </div>
      </div>
      <FxBlock
        fxType={props.node.fxType!}
        color={props.node.color}
        params={props.params}
        onParamChange={props.onParamChange}
      />
      <div class="graph-port graph-port-out" style={{ "--port-color": props.node.color }} />
    </div>
  );
}

// ── Main Canvas ──────────────────────────────────────

export default function Canvas() {
  let vp!: HTMLDivElement;
  let panning = false;
  let psx = 0;
  let psy = 0;

  // Palette state
  const [palettePos, setPalettePos] = createSignal<{ x: number; y: number } | null>(null);
  const [paletteEdgeId, setPaletteEdgeId] = createSignal<string | null>(null);

  // Graph
  const { fxNodes, edges, fxNodePositions } = useGraph(
    (id) => getBlockPos(id),
    () => getBlockPos("master"),
  );

  // Debug: log edges on mount
  createEffect(() => {
    const e = edges();
    const p = fxNodePositions();
    console.log("[graph] edges:", e.length, e.map(x => x.id));
    console.log("[graph] fxNodes:", fxNodes().length);
    // Log first edge path coords
    if (e.length > 0) {
      const from = getNodeCenter(e[0].from, "out", p);
      const to = getNodeCenter(e[0].to, "in", p);
      console.log("[graph] first wire:", e[0].id, "from:", from, "to:", to);
    }
  });

  function onDown(e: PointerEvent) {
    if ((e.target as HTMLElement).closest(".canvas-node")) return;
    if ((e.target as HTMLElement).closest(".fx-palette")) return;
    panning = true;
    psx = e.clientX - cam.tx;
    psy = e.clientY - cam.ty;
    // Close palette on background click
    setPalettePos(null);
    setPaletteEdgeId(null);
  }

  function onMove(e: PointerEvent) {
    if (!panning) return;
    setCam({ tx: e.clientX - psx, ty: e.clientY - psy });
  }

  function onUp() {
    panning = false;
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.92 : 1.08;
    const z = Math.min(2, Math.max(0.25, cam.zoom * factor));
    const r = vp.getBoundingClientRect();
    const cx = e.clientX - r.left;
    const cy = e.clientY - r.top;
    const scale = z / cam.zoom;
    setCam({
      zoom: z,
      tx: cx - (cx - cam.tx) * scale,
      ty: cy - (cy - cam.ty) * scale,
    });
  }

  function onDblClick(e: MouseEvent) {
    if ((e.target as HTMLElement).closest(".canvas-node")) return;
    setCam({ tx: 0, ty: 0, zoom: 1 });
  }

  // ── Wire "+" click → open palette ──
  function handleWireAddClick(edgeId: string, worldX: number, worldY: number) {
    setPalettePos({ x: worldX, y: worldY });
    setPaletteEdgeId(edgeId);
  }

  function handlePaletteSelect(fxType: FxTypeId) {
    const edgeId = paletteEdgeId();
    if (!edgeId) return;

    const info = edgeInsertInfo(edgeId, tracksFx);
    if (!info) return;

    if (info.isMaster) {
      addMasterInsertFx(info.slotIdx, fxType);
    } else {
      addTrackInsertFx(info.trackIdx, info.slotIdx, fxType);
    }

    setPalettePos(null);
    setPaletteEdgeId(null);
  }

  // ── FX node actions ──
  function handleFxToggle(node: GNode) {
    if (node.trackIdx === -1 && node.slotIdx !== undefined) {
      toggleMasterInsertFx(node.slotIdx);
    } else if (node.trackIdx !== undefined && node.trackIdx >= 0 && node.slotIdx !== undefined) {
      toggleTrackInsertFx(node.trackIdx, node.slotIdx);
    }
  }

  function handleFxRemove(node: GNode) {
    if (node.trackIdx === -1 && node.slotIdx !== undefined) {
      clearMasterInsertFx(node.slotIdx);
    } else if (node.trackIdx !== undefined && node.trackIdx >= 0 && node.slotIdx !== undefined) {
      clearTrackInsertFx(node.trackIdx, node.slotIdx);
    }
  }

  function handleFxParamChange(node: GNode, key: string, paramIdx: number, value: number) {
    const normalized = Math.round(Math.min(100, Math.max(0, value))) / 100;
    if (node.trackIdx === -1 && node.slotIdx !== undefined) {
      // Update store
      setMasterFx("insertFx", node.slotIdx, "params", key, value);
      // Send to engine
      setMasterInsertFxParam(node.slotIdx, paramIdx, normalized);
    } else if (node.trackIdx !== undefined && node.trackIdx >= 0 && node.slotIdx !== undefined) {
      // Update store
      setTracksFx(node.trackIdx, "insertFx", node.slotIdx, "params", key, value);
      // Send to engine
      setTrackInsertFxParam(node.trackIdx, node.slotIdx, paramIdx, normalized);
    }
  }

  // ── Keyboard shortcuts ──
  function onKeyDown(e: KeyboardEvent) {
    if ((e.target as HTMLElement)?.closest?.(".cm-editor")) return;
    if ((e.target as HTMLElement)?.tagName === "INPUT") return;

    switch (e.key.toLowerCase()) {
      case "f":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          const r = vp.getBoundingClientRect();
          fitAll(r.width, r.height);
        }
        break;
      case "r":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          resetPositions();
        }
        break;
      case "=":
      case "+":
        e.preventDefault();
        setCam("zoom", (z) => Math.min(2, z * 1.15));
        break;
      case "-":
        e.preventDefault();
        setCam("zoom", (z) => Math.max(0.25, z * 0.87));
        break;
      case "escape":
        setPalettePos(null);
        setPaletteEdgeId(null);
        break;
    }
  }

  onMount(() => document.addEventListener("keydown", onKeyDown));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));

  // ── Minimap ──
  const MINIMAP_W = 140;
  const MINIMAP_H = 90;

  function minimapScale() {
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const id of allIds()) {
      const p = pos[id];
      const s = getSize(id, trackInfos());
      if (p.x < minX) minX = p.x;
      if (p.y < minY) minY = p.y;
      if (p.x + s.w > maxX) maxX = p.x + s.w;
      if (p.y + s.h > maxY) maxY = p.y + s.h;
    }
    const cw = maxX - minX || 1;
    const ch = maxY - minY || 1;
    const pad = 10;
    const s = Math.min((MINIMAP_W - pad * 2) / cw, (MINIMAP_H - pad * 2) / ch);
    return { s, ox: minX, oy: minY, cw, ch };
  }

  const BLOCK_COLORS: Record<BlockId, string> = {
    bass: "#ff6b35", keys: "#00c9b1", fm: "#ffd23f", beats: "#ff5ea0",
    delay: "#5b8cff", reverb: "#b07aff", master: "#ffffff",
  };

  function onMinimapClick(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const { s, ox, oy, cw, ch } = minimapScale();
    const padX = (MINIMAP_W - cw * s) / 2;
    const padY = (MINIMAP_H - ch * s) / 2;
    const wx = (mx - padX) / s + ox;
    const wy = (my - padY) / s + oy;
    const r = vp.getBoundingClientRect();
    setCam({
      tx: r.width / 2 - wx * cam.zoom,
      ty: r.height / 2 - wy * cam.zoom,
    });
  }

  return (
    <div
      ref={vp}
      class="canvas-viewport"
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onWheel={onWheel}
      onDblClick={onDblClick}
    >
      <div
        class="canvas-world"
        style={{
          transform: `translate(${cam.tx}px, ${cam.ty}px) scale(${cam.zoom})`,
        }}
      >
        {/* Dynamic signal flow wires */}
        <svg class="canvas-connections" style={{ overflow: "visible" }}>
          <For each={edges()}>
            {(edge) => {
              // Reactive accessors — recompute when positions change
              const from = () => getNodeCenter(edge.from, "out", fxNodePositions());
              const to = () => getNodeCenter(edge.to, "in", fxNodePositions());
              const mid = () => wireMidpoint(from(), to());
              const isSend = edge.dash;
              const isTrackEdge = edge.trackIdx >= 0;

              return (
                <>
                  <path
                    d={wirePath(from(), to())}
                    stroke={edge.color}
                    stroke-width={2}
                    stroke-dasharray={edge.dash ? "6 4" : undefined}
                    fill="none"
                  />
                  {/* "+" button on wire (only for track and master chain edges) */}
                  <Show when={!isSend}>
                    {(() => {
                      const info = () => edgeInsertInfo(edge.id, tracksFx);
                      return (
                        <Show when={info()}>
                          <circle
                            cx={mid().x} cy={mid().y} r={11}
                            fill="rgba(26,26,34,0.85)"
                            stroke={isTrackEdge ? edge.color : "rgba(255,255,255,0.15)"}
                            stroke-width={1.5}
                            style={{ cursor: "pointer", "pointer-events": "all" }}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleWireAddClick(edge.id, mid().x, mid().y);
                            }}
                          />
                          <text
                            x={mid().x} y={mid().y + 1}
                            text-anchor="middle"
                            dominant-baseline="middle"
                            fill={isTrackEdge ? edge.color : "rgba(255,255,255,0.4)"}
                            font-size="14"
                            font-weight="bold"
                            style={{ "pointer-events": "none", "user-select": "none" }}
                          >+</text>
                        </Show>
                      );
                    })()}
                  </Show>
                </>
              );
            }}
          </For>
        </svg>

        {/* Instrument nodes (dynamic from DSL) */}
        <For each={trackInfos()}>
          {(track) => (
            <Node
              id={track.name}
              label={track.name.toUpperCase()}
              color={KIND_COLORS[track.kind] || KIND_COLORS.unknown}
              showOutputPort
            >
              <BlockForKind kind={track.kind} />
            </Node>
          )}
        </For>

        {/* FX nodes (standalone, draggable) */}
        <For each={fxNodes()}>
          {(node) => {
            const fxPos = () => fxNodePositions()[node.id];
            const params = () => {
              if (node.trackIdx === -1 && node.slotIdx !== undefined) {
                return masterFx.insertFx[node.slotIdx]?.params;
              }
              if (node.trackIdx !== undefined && node.trackIdx >= 0 && node.slotIdx !== undefined) {
                return tracksFx[node.trackIdx]?.insertFx[node.slotIdx]?.params;
              }
              return undefined;
            };
            return (
              <Show when={fxPos()}>
                <FxCanvasNode
                  node={node}
                  x={fxPos()!.x}
                  y={fxPos()!.y}
                  params={params()}
                  onToggle={() => handleFxToggle(node)}
                  onRemove={() => handleFxRemove(node)}
                  onParamChange={(key, paramIdx, value) => handleFxParamChange(node, key, paramIdx, value)}
                />
              </Show>
            );
          }}
        </For>

        {/* Send effects (global) */}
        <Node id="delay" label="DELAY" color="#5b8cff"><DelayBlock /></Node>
        <Node id="reverb" label="REVERB" color="#b07aff"><ReverbBlock /></Node>

        {/* Master (with input port) */}
        <Node id="master" label="MASTER" color="#ffffff" showInputPort>
          <MasterStrip />
        </Node>

        {/* FX Palette popup (positioned in world space) */}
        <Show when={palettePos()}>
          <div
            class="canvas-node"
            style={{
              transform: `translate(${palettePos()!.x - 90}px, ${palettePos()!.y - 220}px)`,
              "z-index": 1000,
            }}
          >
            <FxPalette
              onSelect={handlePaletteSelect}
              onClose={() => { setPalettePos(null); setPaletteEdgeId(null); }}
            />
          </div>
        </Show>
      </div>

      {/* Bottom bar */}
      <div class="canvas-bottom-bar">
        <span class="canvas-shortcut"><b>Space</b> Play</span>
        <span class="canvas-shortcut"><b>Scroll</b> Zoom</span>
        <span class="canvas-shortcut"><b>Drag</b> Pan</span>
        <span class="canvas-shortcut"><b>F</b> Fit all</span>
        <span class="canvas-shortcut"><b>R</b> Reset</span>
        <span class="canvas-shortcut"><b>+/-</b> Zoom</span>
      </div>

      {/* Minimap */}
      <div class="canvas-minimap" onClick={onMinimapClick}>
        <svg width={MINIMAP_W} height={MINIMAP_H}>
          <For each={allIds()}>
            {(id) => {
              const ms = () => minimapScale();
              const bx = () => (pos[id].x - ms().ox) * ms().s + (MINIMAP_W - ms().cw * ms().s) / 2;
              const by = () => (pos[id].y - ms().oy) * ms().s + (MINIMAP_H - ms().ch * ms().s) / 2;
              const bw = () => getSize(id, trackInfos()).w * ms().s;
              const bh = () => getSize(id, trackInfos()).h * ms().s;
              return (
                <rect
                  x={bx()} y={by()} width={bw()} height={bh()}
                  rx={1}
                  fill={BLOCK_COLORS[id]}
                  opacity={0.7}
                />
              );
            }}
          </For>
        </svg>
      </div>
    </div>
  );
}
