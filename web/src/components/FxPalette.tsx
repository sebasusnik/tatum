import { For } from "solid-js";
import { FX_TYPE, FX_LABELS, fxPoolRemaining, type FxTypeId } from "../stores/tatum";

interface FxPaletteProps {
  onSelect: (fxType: FxTypeId) => void;
  onClose: () => void;
}

const FX_OPTIONS: { type: FxTypeId; category: string }[] = [
  { type: FX_TYPE.FILTER, category: "Color" },
  { type: FX_TYPE.SATURATOR, category: "Color" },
  { type: FX_TYPE.CHORUS, category: "Color" },
  { type: FX_TYPE.TILT_EQ, category: "EQ" },
  { type: FX_TYPE.THREE_BAND_EQ, category: "EQ" },
  { type: FX_TYPE.COMPRESSOR, category: "Dynamics" },
  { type: FX_TYPE.LIMITER, category: "Dynamics" },
  { type: FX_TYPE.DELAY, category: "Space" },
  { type: FX_TYPE.REVERB, category: "Space" },
  { type: FX_TYPE.BITCRUSHER, category: "DJ" },
  { type: FX_TYPE.TAPE_STOP, category: "DJ" },
];

/** Popup menu for selecting an FX type to add to a slot. */
export default function FxPalette(props: FxPaletteProps) {
  return (
    <div class="fx-palette" onClick={(e) => e.stopPropagation()}>
      <div class="fx-palette-header">
        <span>Add FX</span>
        <button class="fx-palette-close" onClick={props.onClose}>x</button>
      </div>
      <div class="fx-palette-list">
        <For each={FX_OPTIONS}>
          {(opt) => {
            const remaining = () => fxPoolRemaining(opt.type);
            const exhausted = () => remaining() === 0;
            return (
              <button
                class="fx-palette-item"
                classList={{ exhausted: exhausted() }}
                disabled={exhausted()}
                onClick={() => { props.onSelect(opt.type); props.onClose(); }}
              >
                <span class="fx-palette-name">{FX_LABELS[opt.type]}</span>
                <span class="fx-palette-cat">{opt.category}</span>
                <span class="fx-palette-count">{remaining()}</span>
              </button>
            );
          }}
        </For>
      </div>
    </div>
  );
}
