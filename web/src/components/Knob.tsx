// A knob: drag up or down, or scroll, 0..127 like the controller's. The
// value shown is the engine's; a drag sends values and the view catches up.

import { createSignal } from "solid-js";

export function Knob(props: { value: number; onChange: (value: number) => void }) {
  const [dragging, setDragging] = createSignal<{ y: number; start: number } | null>(null);
  let pending: number | null = null;
  let scheduled = false;

  const send = (value: number) => {
    pending = Math.max(0, Math.min(127, Math.round(value)));
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => {
      scheduled = false;
      if (pending !== null) props.onChange(pending);
      pending = null;
    });
  };

  const onPointerDown = (e: PointerEvent) => {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDragging({ y: e.clientY, start: props.value });
  };
  const onPointerMove = (e: PointerEvent) => {
    const d = dragging();
    if (!d) return;
    // 200 px of travel is the whole way; Shift for the fine step.
    const scale = e.shiftKey ? 0.15 : 0.65;
    send(d.start + (d.y - e.clientY) * scale);
  };
  const onPointerUp = () => setDragging(null);
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    send(props.value - Math.sign(e.deltaY) * (e.shiftKey ? 1 : 4));
  };

  // The arc: 270 degrees from 7 o'clock round to 5.
  const angle = () => -135 + (props.value / 127) * 270;
  const r = 18;
  const arc = (from: number, to: number) => {
    const p = (a: number) => {
      const rad = ((a - 90) * Math.PI) / 180;
      return `${24 + r * Math.cos(rad)} ${24 + r * Math.sin(rad)}`;
    };
    const large = to - from > 180 ? 1 : 0;
    return `M ${p(from)} A ${r} ${r} 0 ${large} 1 ${p(to)}`;
  };

  return (
    <svg
      class={`knob ${dragging() ? "dragging" : ""}`}
      viewBox="0 0 48 48"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onWheel={onWheel}
    >
      <path class="track" d={arc(-135, 135)} />
      <path class="fill" d={arc(-135, angle())} />
      <line class="needle" x1="24" y1="24" x2="24" y2="9" transform={`rotate(${angle()} 24 24)`} />
    </svg>
  );
}
