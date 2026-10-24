// Full-screen overlay showing a frozen capture of one monitor.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

const ZOOM = 11; // pixels per side shown in the loupe (odd, so there is a center)

const screenCanvas = document.getElementById("screen") as HTMLCanvasElement;
const zoomCanvas = document.getElementById("zoom") as HTMLCanvasElement;
const loupe = document.getElementById("loupe") as HTMLDivElement;
const hexLabel = document.getElementById("hex") as HTMLSpanElement;
const chip = document.getElementById("chip") as HTMLSpanElement;
const help = document.getElementById("help") as HTMLDivElement;

let width = 0;
let height = 0;
let pixels = new Uint8ClampedArray();
let cursor = { x: 0, y: 0 }; // in capture pixels
let done = false;

const hex = (r: number, g: number, b: number) => "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase();

function colorAt(x: number, y: number): [number, number, number] {
  const i = (y * width + x) * 4;
  return [pixels[i], pixels[i + 1], pixels[i + 2]];
}

function finish(color: [number, number, number] | null) {
  if (done) return;
  done = true;
  invoke("finish_color_pick", { color });
}

function render(clientX: number, clientY: number) {
  const [r, g, b] = colorAt(cursor.x, cursor.y);
  const value = hex(r, g, b);
  hexLabel.textContent = value;
  chip.style.background = value;

  const ctx = zoomCanvas.getContext("2d")!;
  const half = (ZOOM - 1) / 2;
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, ZOOM, ZOOM);
  ctx.drawImage(screenCanvas, cursor.x - half, cursor.y - half, ZOOM, ZOOM, 0, 0, ZOOM, ZOOM);

  // Keep the loupe inside the window, flipping to the other side near edges.
  loupe.style.display = "block";
  const box = loupe.getBoundingClientRect();
  const gap = 24;
  const left = clientX + gap + box.width > innerWidth ? clientX - gap - box.width : clientX + gap;
  const top = clientY + gap + box.height > innerHeight ? clientY - gap - box.height : clientY + gap;
  loupe.style.transform = `translate(${left}px, ${top}px)`;
}

/** Maps CSS coordinates to capture pixels (the capture is in physical pixels). */
function toPixel(clientX: number, clientY: number) {
  return {
    x: Math.min(width - 1, Math.max(0, Math.floor((clientX * width) / innerWidth))),
    y: Math.min(height - 1, Math.max(0, Math.floor((clientY * height) / innerHeight))),
  };
}

function toClient(p: { x: number; y: number }) {
  return { x: ((p.x + 0.5) * innerWidth) / width, y: ((p.y + 0.5) * innerHeight) / height };
}

async function load() {
  const label = getCurrentWebviewWindow().label;
  const index = Number(label.split("-")[1]);
  const buffer = await invoke<ArrayBuffer>("capture_pixels", { index });
  const header = new DataView(buffer, 0, 8);
  width = header.getUint32(0, true);
  height = header.getUint32(4, true);
  pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
  screenCanvas.width = width;
  screenCanvas.height = height;
  screenCanvas.getContext("2d")!.putImageData(new ImageData(pixels, width, height), 0, 0);
  zoomCanvas.getContext("2d")!.imageSmoothingEnabled = false;
}

window.addEventListener("mousemove", (e) => {
  if (!width) return;
  help.style.opacity = "0.35";
  cursor = toPixel(e.clientX, e.clientY);
  render(e.clientX, e.clientY);
});

window.addEventListener("mousedown", (e) => {
  if (!width) return;
  if (e.button === 0) finish(colorAt(cursor.x, cursor.y));
  else finish(null);
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") return finish(null);
  if (!width) return;
  if (e.key === "Enter") return finish(colorAt(cursor.x, cursor.y));
  const step = e.shiftKey ? 10 : 1;
  const moves: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
  const move = moves[e.key];
  if (!move) return;
  e.preventDefault();
  cursor = { x: Math.min(width - 1, Math.max(0, cursor.x + move[0])), y: Math.min(height - 1, Math.max(0, cursor.y + move[1])) };
  const c = toClient(cursor);
  render(c.x, c.y);
});

load().catch(() => finish(null));
