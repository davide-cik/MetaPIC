// Transparent overlay over one monitor: shows a live loupe of the real screen
// under the cursor and reports the clicked color.
import { invoke } from "@tauri-apps/api/core";

const ZOOM_PX = 165; // loupe size in CSS pixels

const zoomCanvas = document.getElementById("zoom") as HTMLCanvasElement;
const loupe = document.getElementById("loupe") as HTMLDivElement;
const center = document.getElementById("center") as HTMLDivElement;
const hexLabel = document.getElementById("hex") as HTMLSpanElement;
const rgbLabel = document.getElementById("rgb") as HTMLSpanElement;
const chip = document.getElementById("chip") as HTMLSpanElement;
const help = document.getElementById("help") as HTMLDivElement;

type Rgb = [number, number, number];

let current: Rgb | null = null;
let pointer = { x: -1, y: -1 }; // last position over this overlay, CSS pixels
let busy = false;
let done = false;

const hex = ([r, g, b]: Rgb) => "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase();

interface Sample {
  color: Rgb;
  column: number;
  row: number;
  image: ImageData;
}

async function sample(): Promise<Sample> {
  const buffer = await invoke<ArrayBuffer>("sample_cursor");
  const header = new DataView(buffer, 0, 16);
  const [column, row, width, height] = [0, 4, 8, 12].map((offset) => header.getUint32(offset, true));
  const pixels = new Uint8ClampedArray(buffer, 16, width * height * 4);
  const i = (row * width + column) * 4;
  return { color: [pixels[i], pixels[i + 1], pixels[i + 2]], column, row, image: new ImageData(pixels, width, height) };
}

function render(s: Sample) {
  current = s.color;
  const value = hex(s.color);
  hexLabel.textContent = value;
  rgbLabel.textContent = s.color.join(", ");
  chip.style.background = value;

  zoomCanvas.width = s.image.width;
  zoomCanvas.height = s.image.height;
  zoomCanvas.getContext("2d")!.putImageData(s.image, 0, 0);
  const cell = ZOOM_PX / s.image.width;
  center.style.left = `${s.column * cell - 1}px`;
  center.style.top = `${s.row * cell - 1}px`;

  // Next to the cursor, flipped near the edges so it stays on screen.
  loupe.style.display = "block";
  const box = loupe.getBoundingClientRect();
  const gap = 28;
  const left = pointer.x + gap + box.width > innerWidth ? pointer.x - gap - box.width : pointer.x + gap;
  const top = pointer.y + gap + box.height > innerHeight ? pointer.y - gap - box.height : pointer.y + gap;
  loupe.style.transform = `translate(${left}px, ${top}px)`;
}

/** Samples at most once per frame; the screen is read live each time. */
async function refresh() {
  if (busy || done || pointer.x < 0) return;
  busy = true;
  try {
    render(await sample());
  } catch {
    // Cursor between monitors or capture hiccup: keep the last reading.
  } finally {
    busy = false;
  }
}

async function finish(pick: boolean) {
  if (done) return;
  let color: Rgb | null = null;
  if (pick) {
    try {
      color = (await sample()).color;
    } catch {
      color = current;
    }
  }
  done = true;
  invoke("finish_color_pick", { color });
}

window.addEventListener("mousemove", (e) => {
  pointer = { x: e.clientX, y: e.clientY };
  help.style.opacity = "0.35";
  requestAnimationFrame(refresh);
});

window.addEventListener("mouseleave", () => {
  // The cursor moved to another monitor's overlay, which takes over.
  loupe.style.display = "none";
  pointer = { x: -1, y: -1 };
});

window.addEventListener("mousedown", (e) => {
  e.preventDefault();
  finish(e.button === 0);
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") return finish(false);
  if (e.key === "Enter" || e.key === " ") return finish(true);
  const step = e.shiftKey ? 10 : 1;
  const moves: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
  const move = moves[e.key];
  if (!move) return;
  e.preventDefault();
  invoke("nudge_cursor", { dx: move[0], dy: move[1] });
});

// Keep the reading live even when the mouse is still (videos, animations).
setInterval(refresh, 150);
