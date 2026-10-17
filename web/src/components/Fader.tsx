// A vertical fader, 0..127 like a knob: drag the cap or anywhere on the
// slot, scroll for small steps, Shift for fine ones. The value shown is
// the engine's; a drag sends values and the view catches up.

import { createSignal } from "solid-js";

export function Fader(props: { value: number; onChange: (value: number) => void }) {
  const [dragging, setDragging] = createSignal(false);
  let slot!: HTMLDivElement;
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
  const at = (e: PointerEvent) => {
    const r = slot.getBoundingClientRect();
    return (1 - (e.clientY - r.top) / r.height) * 127;
  };
  let from: { y: number; value: number } | null = null;

  const down = (e: PointerEvent) => {
    slot.setPointerCapture(e.pointerId);
    setDragging(true);
    // On the cap: move from where it is; elsewhere: jump there.
    if ((e.target as HTMLElement).classList.contains("fader-cap")) from = { y: e.clientY, value: props.value };
    else {
      from = { y: e.clientY, value: at(e) };
      send(from.value);
    }
  };
  const move = (e: PointerEvent) => {
    if (!from) return;
    const r = slot.getBoundingClientRect();
    const per = 127 / r.height;
    send(from.value + (from.y - e.clientY) * per * (e.shiftKey ? 0.25 : 1));
  };
  const up = () => {
    from = null;
    setDragging(false);
  };
  const wheel = (e: WheelEvent) => {
    e.preventDefault();
    send(props.value - Math.sign(e.deltaY) * (e.shiftKey ? 1 : 3));
  };

  return (
    <div
      ref={slot}
      class={`fader ${dragging() ? "dragging" : ""}`}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onWheel={wheel}
    >
      <div class="fader-slot" />
      <div class="fader-fill" style={{ height: `${(props.value / 127) * 100}%` }} />
      <div class="fader-cap" style={{ bottom: `calc(${(props.value / 127) * 100}% - 9px)` }} />
    </div>
  );
}
