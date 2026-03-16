import { onMount, onCleanup, For } from "solid-js";
import { currentModule, mod, page, currentPage, playing, currentStep, accentColor, pageCount } from "../stores/synth";
import { drawBassViz, drawKeysViz, drawFmViz, drawBeatsViz } from "../visualizers";

export default function Screen() {
  let canvasEl: HTMLCanvasElement | undefined;
  let ctx: CanvasRenderingContext2D;
  let frame = 0;
  let animId: number;

  onMount(() => {
    ctx = canvasEl!.getContext("2d")!;
    resize();
    window.addEventListener("resize", resize);
    draw();
  });

  onCleanup(() => {
    cancelAnimationFrame(animId);
    window.removeEventListener("resize", resize);
  });

  function resize() {
    const el = canvasEl!;
    el.width = el.offsetWidth * 2;
    el.height = el.offsetHeight * 2;
    ctx.scale(2, 2);
  }

  function draw() {
    const el = canvasEl!;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    ctx.clearRect(0, 0, w, h);
    frame++;
    const color = accentColor();
    const m = currentModule();

    if (m === "bass") drawBassViz(ctx, w, h, color, frame, playing(), mod().pages[currentPage()].values);
    else if (m === "keys") drawKeysViz(ctx, w, h, color, frame, playing());
    else if (m === "fm") drawFmViz(ctx, w, h, color, frame, playing(), mod().pages[currentPage()].values);
    else drawBeatsViz(ctx, w, h, color, frame, playing(), currentStep());

    animId = requestAnimationFrame(draw);
  }

  return (
    <div class="screen">
      <canvas ref={canvasEl} />
      <div class="screen-overlay">
        <div class="screen-top">
          <span class="screen-module-name">{mod().label}</span>
          <span class="screen-sub" innerHTML={`${mod().sub}<br>PAGE ${currentPage() + 1}/${pageCount()}`} />
        </div>
        <div class="screen-params">
          <For each={page().keys}>
            {(key, i) => (
              <div class={`screen-p sp${i() + 1}`}>
                <div class="screen-p-val">{page().values[i()]}</div>
                <div class="screen-p-name">{key}</div>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
