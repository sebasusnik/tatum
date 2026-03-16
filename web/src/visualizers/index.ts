export function drawBassViz(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  color: string, frame: number, isPlaying: boolean, values: number[]
) {
  const cutoff = values[0] / 100;
  const reso = values[1] / 100;
  const mid = h * 0.45;

  ctx.beginPath();
  ctx.strokeStyle = color;
  ctx.lineWidth = 3;
  ctx.shadowColor = color;
  ctx.shadowBlur = isPlaying ? 20 : 8;
  for (let x = 0; x < w; x++) {
    const t = x / w;
    const freq = 2 + cutoff * 6;
    let y = Math.sin(t * Math.PI * 2 * freq + frame * 0.04) * (30 + cutoff * 25);
    y += Math.sin(t * Math.PI * 2 * freq * 2 + frame * 0.06) * reso * 15;
    y += Math.sin(t * Math.PI * 2 * freq * 3 + frame * 0.02) * 5;
    if (x === 0) ctx.moveTo(x, mid + y);
    else ctx.lineTo(x, mid + y);
  }
  ctx.stroke();
  ctx.shadowBlur = 0;

  // Ghost line
  ctx.beginPath();
  ctx.strokeStyle = color;
  ctx.globalAlpha = 0.1;
  ctx.lineWidth = 20;
  for (let x = 0; x < w; x++) {
    const t = x / w;
    const y = Math.sin(t * Math.PI * 2 * 3 + frame * 0.03) * 35;
    if (x === 0) ctx.moveTo(x, mid + y);
    else ctx.lineTo(x, mid + y);
  }
  ctx.stroke();
  ctx.globalAlpha = 1;
}

export function drawKeysViz(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  color: string, frame: number, isPlaying: boolean
) {
  const cx = w / 2, cy = h * 0.42;
  const notes = [0, 4, 7, 11, 12];
  ctx.shadowColor = color;
  ctx.shadowBlur = isPlaying ? 12 : 4;
  notes.forEach((_, i) => {
    const bw = 120 - i * 10 + Math.sin(frame * 0.03 + i) * 8;
    const bh = 12;
    const by = cy - (notes.length / 2 - i) * 22 + Math.sin(frame * 0.025 + i * 0.8) * 4;
    const alpha = 0.3 + (1 - i / notes.length) * 0.5;
    ctx.globalAlpha = alpha;
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.roundRect(cx - bw / 2, by - bh / 2, bw, bh, 6);
    ctx.fill();
  });
  ctx.globalAlpha = 1;
  ctx.shadowBlur = 0;
}

export function drawFmViz(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  color: string, frame: number, isPlaying: boolean, values: number[]
) {
  const cx = w / 2, cy = h * 0.42;
  const modIdx = values[1] / 100;
  ctx.beginPath();
  ctx.strokeStyle = color;
  ctx.lineWidth = 2;
  ctx.shadowColor = color;
  ctx.shadowBlur = isPlaying ? 15 : 6;
  const pts = 300;
  for (let i = 0; i <= pts; i++) {
    const t = (i / pts) * Math.PI * 2;
    const r = 40 + Math.sin(t * 3 + frame * 0.02) * 15 * modIdx;
    const x = cx + Math.cos(t + Math.sin(t * 3 + frame * 0.015) * modIdx * 2) * r;
    const y = cy + Math.sin(t * 2 + Math.cos(t * 5 + frame * 0.02) * modIdx) * r;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
  ctx.stroke();

  ctx.globalAlpha = 0.05;
  ctx.fillStyle = color;
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.shadowBlur = 0;
}

export function drawBeatsViz(
  ctx: CanvasRenderingContext2D, w: number, h: number,
  color: string, frame: number, isPlaying: boolean, step: number
) {
  const drums = [
    { label: "K", x: w * 0.25, r: 30 },
    { label: "S", x: w * 0.42, r: 22 },
    { label: "H", x: w * 0.58, r: 18 },
    { label: "C", x: w * 0.75, r: 16 },
  ];
  const cy = h * 0.42;

  drums.forEach((d, i) => {
    const pulse =
      isPlaying && (step % 4 === 0 && i === 0 || step % 4 === 2 && i === 2)
        ? Math.sin(frame * 0.2) * 8
        : 0;
    const r = d.r + pulse + Math.sin(frame * 0.015 + i * 1.5) * 3;

    ctx.beginPath();
    ctx.strokeStyle = color;
    ctx.lineWidth = 2.5;
    ctx.shadowColor = color;
    ctx.shadowBlur = isPlaying ? 10 + pulse : 4;
    ctx.arc(d.x, cy, r, 0, Math.PI * 2);
    ctx.stroke();

    ctx.beginPath();
    ctx.globalAlpha = 0.08;
    ctx.fillStyle = color;
    ctx.arc(d.x, cy, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;

    ctx.fillStyle = color;
    ctx.font = '700 11px "Space Mono", monospace';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.shadowBlur = 0;
    ctx.fillText(d.label, d.x, cy);
  });
  ctx.shadowBlur = 0;
}
