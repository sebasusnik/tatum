interface KnobProps {
  value: number;        // 0–100
  label: string;
  color: string;
  size?: number;        // px, default 72
  onChange: (value: number) => void;
}

export default function Knob(props: KnobProps) {
  const size = () => props.size ?? 72;
  const angle = () => -135 + (props.value / 100) * 270;

  let dragging = false;
  let startY = 0;
  let startVal = 0;

  function onPointerDown(e: PointerEvent) {
    dragging = true;
    startY = e.clientY;
    startVal = props.value;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const delta = (startY - e.clientY) * 0.6;
    props.onChange(Math.min(100, Math.max(0, startVal + delta)));
  }

  function onPointerUp() {
    dragging = false;
  }

  // CSS custom properties carry dynamic values; all visuals in CSS
  const cssVars = () => ({
    "--knob-color": props.color,
    "--knob-angle": `${angle()}deg`,
    "--knob-size": `${size()}px`,
  } as Record<string, string>);

  return (
    <div class="enc" style={cssVars()}>
      <span class="enc-label">{props.label}</span>
      <div
        class="enc-wheel"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      >
        <div class="enc-ring" />
        <div class="enc-body">
          <div class="enc-indicator">
            <div class="enc-dot" />
          </div>
        </div>
      </div>
    </div>
  );
}
