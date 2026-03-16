import { createSignal } from "solid-js";

interface MiniKnobProps {
  value?: number;  // 0–100
  onChange?: (v: number) => void;
}

export default function MiniKnob(props: MiniKnobProps) {
  const [val, setVal] = createSignal(props.value ?? 50);
  const angle = () => -135 + (val() / 100) * 270;

  let dragging = false;
  let startY = 0;
  let startVal = 0;

  function onPointerDown(e: PointerEvent) {
    dragging = true;
    startY = e.clientY;
    startVal = val();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const nv = Math.min(100, Math.max(0, startVal + (startY - e.clientY) * 1.2));
    setVal(nv);
    props.onChange?.(nv);
  }

  function onPointerUp() {
    dragging = false;
  }

  return (
    <div
      class="fx-mini"
      style={{ "--mini-angle": `${angle()}deg` } as Record<string, string>}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <div class="fx-mini-indicator">
        <div class="fx-mini-dot" />
      </div>
    </div>
  );
}
