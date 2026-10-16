// The mix as a scrolling spectrogram, the way the terminal draws it: log
// frequency, 30 Hz to 16 kHz, the inferno ramp. From an AnalyserNode on
// the engine's output; the engine itself is not asked.

import { createEffect, onCleanup } from "solid-js";
import { analyser } from "../store";

const LOW = 30;
const HIGH = 16000;
const FLOOR = -90;
const TOP = -10;

/** The inferno ramp at t in 0..1, as the terminal's palette has it. */
function inferno(t: number): string {
  const stops: [number, number, number][] = [
    [0, 0, 4],
    [40, 11, 84],
    [101, 21, 110],
    [159, 42, 99],
    [212, 72, 66],
    [245, 125, 21],
    [250, 193, 39],
    [252, 255, 164],
  ];
  const x = Math.max(0, Math.min(1, t)) * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(x));
  const f = x - i;
  const c = stops[i].map((a, k) => Math.round(a + (stops[i + 1][k] - a) * f));
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

export function Spectrum() {
  let canvas!: HTMLCanvasElement;
  createEffect(() => {
    const a = analyser();
    if (!a) return;
    const ctx = canvas.getContext("2d")!;
    const data = new Float32Array(a.frequencyBinCount);
    const rows = 120;
    const binHz = a.context.sampleRate / a.fftSize;
    let last = 0;
    let frame = 0;
    const draw = (now: number) => {
      frame = requestAnimationFrame(draw);
      if (now - last < 40) return;
      last = now;
      const { width, height } = canvas;
      ctx.drawImage(canvas, -2, 0);
      a.getFloatFrequencyData(data);
      for (let row = 0; row < rows; row++) {
        const lo = LOW * Math.pow(HIGH / LOW, row / rows);
        const hi = LOW * Math.pow(HIGH / LOW, (row + 1) / rows);
        const from = Math.floor(lo / binHz);
        const to = Math.max(from + 1, Math.ceil(hi / binHz));
        let peak = -Infinity;
        for (let b = from; b < to && b < data.length; b++) peak = Math.max(peak, data[b]);
        const t = (peak - FLOOR) / (TOP - FLOOR);
        ctx.fillStyle = inferno(t);
        const y = height - ((row + 1) / rows) * height;
        ctx.fillRect(width - 2, y, 2, height / rows + 1);
      }
    };
    frame = requestAnimationFrame(draw);
    onCleanup(() => cancelAnimationFrame(frame));
  });
  return <canvas class="spectrum" ref={canvas} width={1200} height={160} />;
}
