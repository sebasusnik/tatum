import { createMemo } from "solid-js";
import { createStore } from "solid-js/store";
import {
  tracksFx,
  masterFx,
  sends,
  FX_TYPE,
  FX_LABELS,
  trackInfos,
  type FxTypeId,
  MAX_INSERT_FX,
  MAX_MASTER_FX,
} from "./tatum";

// ── Types ────────────────────────────────────────────

export interface GraphNode {
  id: string;
  kind: "instrument" | "fx" | "master" | "send";
  label: string;
  color: string;
  trackIdx?: number;
  slotIdx?: number;
  fxType?: FxTypeId;
  instanceIdx?: number;
  enabled?: boolean;
}

export interface GraphEdge {
  id: string;
  from: string;
  to: string;
  trackIdx: number;    // -1 for master chain, -2/-3 for sends
  color: string;
  dash?: boolean;       // for send edges
}

// ── Constants ────────────────────────────────────────

const KIND_COLORS: Record<string, string> = {
  bass: "#ff6b35",
  keys: "#00c9b1",
  fm: "#ffd23f",
  beats: "#ff5ea0",
  graph: "#88aaff",
  unknown: "#888888",
};

const FALLBACK_COLORS = ["#ff6b35", "#00c9b1", "#ffd23f", "#ff5ea0", "#88aaff", "#b07aff", "#ff8855"];

export const TRACK_INSTRUMENTS = createMemo(() =>
  trackInfos().map((t, i) => ({
    id: t.name,
    trackIdx: i,
    label: t.name.toUpperCase(),
    color: KIND_COLORS[t.kind] || FALLBACK_COLORS[i % FALLBACK_COLORS.length],
  }))
);

const trackColors = createMemo(() => TRACK_INSTRUMENTS().map((t) => t.color));
const MASTER_COLOR = "#ffffff";
const SEND_COLORS = { delay: "#5b8cff", reverb: "#b07aff" };

// ── Positions (persisted) ────────────────────────────

type PosMap = Record<string, { x: number; y: number }>;

function loadPositions(): PosMap {
  try {
    const s = localStorage.getItem("synth-graph-pos");
    if (s) return JSON.parse(s);
  } catch {}
  return {};
}

function savePositions(p: PosMap) {
  localStorage.setItem("synth-graph-pos", JSON.stringify(p));
}

export const [fxPositions, setFxPositions] = createStore<PosMap>(loadPositions());

export function setFxNodePosition(id: string, x: number, y: number) {
  setFxPositions(id, { x, y });
  // Debounced save
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => savePositions({ ...fxPositions }), 500);
}

let saveTimer = 0;

// ── Auto-layout ──────────────────────────────────────

// Given instrument position and master position, compute FX node positions
// FX nodes are spaced evenly between instrument output and master input
function autoLayoutFxNodes(
  instrX: number, instrY: number, instrH: number,
  masterX: number, _masterY: number,
  fxCount: number, fxIndex: number,
): { x: number; y: number } {
  const startX = instrX + 230; // after instrument block
  const endX = masterX - 140;  // before master block
  const totalGap = endX - startX;
  const spacing = fxCount > 0 ? totalGap / (fxCount + 1) : 0;
  return {
    x: startX + spacing * (fxIndex + 1),
    y: instrY + instrH / 2 - 20, // center vertically with instrument
  };
}

// ── Derived graph ────────────────────────────────────

export function useGraph(
  getInstrPos: (id: string) => { x: number; y: number; w: number; h: number },
  getMasterPos: () => { x: number; y: number; w: number; h: number },
) {
  const fxNodes = createMemo<GraphNode[]>(() => {
    const nodes: GraphNode[] = [];

    // Track insert FX nodes
    for (let t = 0; t < 4; t++) {
      const track = tracksFx[t];
      if (!track) continue;
      for (let s = 0; s < MAX_INSERT_FX; s++) {
        const slot = track.insertFx[s];
        if (!slot || slot.type === FX_TYPE.NONE) continue;
        nodes.push({
          id: `fx-${t}-${s}`,
          kind: "fx",
          label: FX_LABELS[slot.type],
          color: trackColors()[t],
          trackIdx: t,
          slotIdx: s,
          fxType: slot.type,
          instanceIdx: slot.instanceIdx,
          enabled: slot.enabled,
        });
      }
    }

    // Master FX nodes
    for (let s = 0; s < MAX_MASTER_FX; s++) {
      const slot = masterFx.insertFx[s];
      if (!slot || slot.type === FX_TYPE.NONE) continue;
      nodes.push({
        id: `mfx-${s}`,
        kind: "fx",
        label: FX_LABELS[slot.type],
        color: MASTER_COLOR,
        trackIdx: -1,
        slotIdx: s,
        fxType: slot.type,
        instanceIdx: slot.instanceIdx,
        enabled: slot.enabled,
      });
    }

    return nodes;
  });

  const edges = createMemo<GraphEdge[]>(() => {
    const result: GraphEdge[] = [];

    // Track chains: instrument → [fx...] → master
    const instruments = TRACK_INSTRUMENTS();
    for (let t = 0; t < instruments.length; t++) {
      const instr = instruments[t];
      const track = tracksFx[t];
      const fxIds: string[] = [];

      if (track) {
        for (let s = 0; s < MAX_INSERT_FX; s++) {
          const slot = track.insertFx[s];
          if (slot && slot.type !== FX_TYPE.NONE) {
            fxIds.push(`fx-${t}-${s}`);
          }
        }
      }

      const chain = [instr.id, ...fxIds, "master"];
      for (let i = 0; i < chain.length - 1; i++) {
        result.push({
          id: `${chain[i]}→${chain[i + 1]}`,
          from: chain[i],
          to: chain[i + 1],
          trackIdx: t,
          color: instr.color.replace(")", ",0.5)").replace("rgb", "rgba").replace("#", ""),
        });
      }
      // Fix color to use hex with alpha
      for (const e of result) {
        if (e.trackIdx === t) {
          e.color = hexToRgba(instr.color, 0.45);
        }
      }
    }

    // Master chain edges
    const masterFxIds: string[] = [];
    for (let s = 0; s < MAX_MASTER_FX; s++) {
      const slot = masterFx.insertFx[s];
      if (slot && slot.type !== FX_TYPE.NONE) {
        masterFxIds.push(`mfx-${s}`);
      }
    }
    if (masterFxIds.length > 0) {
      const masterChain = ["master-in", ...masterFxIds, "master-out"];
      for (let i = 0; i < masterChain.length - 1; i++) {
        result.push({
          id: `${masterChain[i]}→${masterChain[i + 1]}`,
          from: masterChain[i],
          to: masterChain[i + 1],
          trackIdx: -1,
          color: "rgba(255,255,255,0.3)",
        });
      }
    }

    // Send edges (dashed)
    const s = sends();
    for (const instr of TRACK_INSTRUMENTS()) {
      const modSends = s[instr.id as keyof typeof s];
      if (modSends?.delay > 0) {
        result.push({
          id: `${instr.id}→delay`,
          from: instr.id,
          to: "delay",
          trackIdx: -2,
          color: hexToRgba(SEND_COLORS.delay, Math.min(0.5, modSends.delay * 0.5)),
          dash: true,
        });
      }
      if (modSends?.reverb > 0) {
        result.push({
          id: `${instr.id}→reverb`,
          from: instr.id,
          to: "reverb",
          trackIdx: -3,
          color: hexToRgba(SEND_COLORS.reverb, Math.min(0.5, modSends.reverb * 0.5)),
          dash: true,
        });
      }
    }

    return result;
  });

  // Compute positions for FX nodes (user-saved or auto-layout)
  const fxNodePositions = createMemo<Record<string, { x: number; y: number }>>(() => {
    const positions: Record<string, { x: number; y: number }> = {};
    const masterPos = getMasterPos();

    for (const node of fxNodes()) {
      // Check for saved position first
      const saved = fxPositions[node.id];
      if (saved) {
        positions[node.id] = saved;
        continue;
      }

      // Auto-layout
      const allInstr = TRACK_INSTRUMENTS();
      if (node.trackIdx !== undefined && node.trackIdx >= 0 && node.trackIdx < allInstr.length) {
        const instr = allInstr[node.trackIdx];
        const instrPos = getInstrPos(instr.id);
        // Count FX in this track for spacing
        const trackFxCount = fxNodes().filter(n => n.trackIdx === node.trackIdx).length;
        const trackFxIndex = fxNodes()
          .filter(n => n.trackIdx === node.trackIdx)
          .indexOf(node);
        positions[node.id] = autoLayoutFxNodes(
          instrPos.x, instrPos.y, instrPos.h,
          masterPos.x, masterPos.y,
          trackFxCount, trackFxIndex,
        );
      } else if (node.trackIdx === -1) {
        // Master chain FX — position below master
        const masterFxNodes = fxNodes().filter(n => n.trackIdx === -1);
        const idx = masterFxNodes.indexOf(node);
        const count = masterFxNodes.length;
        const spacing = 140;
        const startX = masterPos.x + (masterPos.w / 2) - ((count - 1) * spacing) / 2;
        positions[node.id] = {
          x: startX + idx * spacing,
          y: masterPos.y + masterPos.h + 40,
        };
      }
    }

    return positions;
  });

  return { fxNodes, edges, fxNodePositions };
}

// ── Helpers ──────────────────────────────────────────

function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}

/** Get the track index and next available slot for adding FX to a specific edge. */
export function edgeInsertInfo(edgeId: string, tracksFxState: typeof tracksFx): {
  trackIdx: number;
  slotIdx: number;
  isMaster: boolean;
} | null {
  // Parse edge like "bass→fx-0-1" or "fx-0-1→master"
  const parts = edgeId.split("→");
  if (parts.length !== 2) return null;

  const from = parts[0];
  const to = parts[1];

  // Master chain edges
  if (from.startsWith("master-") || to.startsWith("master-")) {
    // Find next empty master slot
    for (let s = 0; s < MAX_MASTER_FX; s++) {
      const slot = masterFx.insertFx[s];
      if (!slot || slot.type === FX_TYPE.NONE) {
        return { trackIdx: -1, slotIdx: s, isMaster: true };
      }
    }
    return null; // full
  }

  // Find which track this edge belongs to
  let trackIdx = -1;
  for (const instr of TRACK_INSTRUMENTS()) {
    if (from === instr.id || from.startsWith(`fx-${instr.trackIdx}-`)) {
      trackIdx = instr.trackIdx;
      break;
    }
  }
  if (to !== "master") {
    // Check `to` side
    const match = to.match(/^fx-(\d+)-/);
    if (match) trackIdx = parseInt(match[1]);
  }

  if (trackIdx < 0) return null;

  // Find next empty slot in this track
  const track = tracksFxState[trackIdx];
  if (!track) return null;
  for (let s = 0; s < MAX_INSERT_FX; s++) {
    const slot = track.insertFx[s];
    if (!slot || slot.type === FX_TYPE.NONE) {
      return { trackIdx, slotIdx: s, isMaster: false };
    }
  }
  return null; // full
}
