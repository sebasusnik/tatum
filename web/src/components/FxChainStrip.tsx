import { For, Show, createSignal } from "solid-js";
import FxNode from "./FxNode";
import FxPalette from "./FxPalette";
import {
  FX_TYPE, type FxTypeId, type InsertFxState,
  tracksFx, masterFx,
  addTrackInsertFx, clearTrackInsertFx, toggleTrackInsertFx,
  addMasterInsertFx, clearMasterInsertFx, toggleMasterInsertFx,
} from "../stores/tatum";

interface FxChainStripProps {
  /** Track index (0-5) or "master" */
  target: number | "master";
  color?: string;
}

/** Horizontal strip showing insert FX slots with add/remove controls. */
export default function FxChainStrip(props: FxChainStripProps) {
  const [paletteSlot, setPaletteSlot] = createSignal<number | null>(null);

  const slots = (): InsertFxState[] =>
    props.target === "master"
      ? masterFx.insertFx
      : tracksFx[props.target as number]?.insertFx ?? [];

  const maxSlots = () =>
    props.target === "master" ? 6 : 4;

  function handleAdd(slotIdx: number, fxType: FxTypeId) {
    if (props.target === "master") {
      addMasterInsertFx(slotIdx, fxType);
    } else {
      addTrackInsertFx(props.target as number, slotIdx, fxType);
    }
  }

  function handleRemove(slotIdx: number) {
    if (props.target === "master") {
      clearMasterInsertFx(slotIdx);
    } else {
      clearTrackInsertFx(props.target as number, slotIdx);
    }
  }

  function handleToggle(slotIdx: number) {
    if (props.target === "master") {
      toggleMasterInsertFx(slotIdx);
    } else {
      toggleTrackInsertFx(props.target as number, slotIdx);
    }
  }

  // Find first empty slot
  function firstEmptySlot(): number | null {
    const s = slots();
    for (let i = 0; i < maxSlots(); i++) {
      if (!s[i] || s[i].type === FX_TYPE.NONE) return i;
    }
    return null;
  }

  const hasAnyFx = () => slots().some(s => s.type !== FX_TYPE.NONE);

  return (
    <div class="fx-chain-strip" style={{ "--strip-color": props.color || "#888" }}>
      <span class="fx-chain-label">INSERT FX</span>
      <div class="fx-chain-slots">
        <For each={slots().slice(0, maxSlots())}>
          {(slot, i) => (
            <Show when={slot.type !== FX_TYPE.NONE}>
              <FxNode
                type={slot.type}
                enabled={slot.enabled}
                onToggle={() => handleToggle(i())}
                onRemove={() => handleRemove(i())}
                compact
              />
            </Show>
          )}
        </For>
        <Show when={!hasAnyFx()}>
          <span class="fx-chain-empty">no fx</span>
        </Show>
        <button
          class="fx-chain-add"
          onClick={(e) => {
            e.stopPropagation();
            const slot = firstEmptySlot();
            if (slot !== null) setPaletteSlot(slot);
          }}
          disabled={firstEmptySlot() === null}
          title="Add FX"
        >
          +
        </button>
      </div>
      <Show when={paletteSlot() !== null}>
        <FxPalette
          onSelect={(fxType) => handleAdd(paletteSlot()!, fxType)}
          onClose={() => setPaletteSlot(null)}
        />
      </Show>
    </div>
  );
}
