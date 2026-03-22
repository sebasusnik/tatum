import { Show } from "solid-js";
import { FX_LABELS, type FxTypeId, FX_TYPE } from "../stores/synth";

interface FxNodeProps {
  type: FxTypeId;
  enabled: boolean;
  onToggle?: () => void;
  onRemove?: () => void;
  onClick?: () => void;
  compact?: boolean;
}

/** Small FX chip showing type label with enable/remove controls. */
export default function FxNode(props: FxNodeProps) {
  if (props.type === FX_TYPE.NONE) return null;

  return (
    <div
      class="fx-node"
      classList={{ disabled: !props.enabled, compact: props.compact }}
      onClick={(e) => { e.stopPropagation(); props.onClick?.(); }}
    >
      <span class="fx-node-label">{FX_LABELS[props.type]}</span>
      <div class="fx-node-controls">
        <Show when={props.onToggle}>
          <button
            class="fx-node-btn"
            classList={{ off: !props.enabled }}
            onClick={(e) => { e.stopPropagation(); props.onToggle!(); }}
            title={props.enabled ? "Bypass" : "Enable"}
          >
            {props.enabled ? "ON" : "OFF"}
          </button>
        </Show>
        <Show when={props.onRemove}>
          <button
            class="fx-node-btn fx-node-remove"
            onClick={(e) => { e.stopPropagation(); props.onRemove!(); }}
            title="Remove"
          >
            x
          </button>
        </Show>
      </div>
    </div>
  );
}
