import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

// ---- Types shared with the Rust side ----

type Format = "jpeg" | "png" | "webp" | "avif" | "jxl" | "heic" | "tiff" | "gif" | "bmp" | "ico" | "qoi";
type MetaMode = "keep" | "keep_no_gps" | "strip";

interface FileEntry {
  path: string;
  name: string;
  size: number;
  format: Format;
}

interface Outcome {
  output: string;
  inputBytes: number;
  outputBytes: number;
  width: number;
  height: number;
  millis: number;
  notes: string[];
}

interface Progress {
  path: string;
  outcome: Outcome | null;
  error: string | null;
}

interface TagView {
  name: string;
  group: string;
  value: string;
  editable: boolean;
}

interface MetadataView {
  format: Format;
  writable: boolean;
  tags: TagView[];
  editable: string[];
}

interface Edit {
  tag: string;
  value: string;
}

// ---- Output formats ----

interface FormatInfo {
  id: Format;
  label: string;
  lossy: boolean;
  lossless: boolean;
  hint: string;
}

const FORMATS: FormatInfo[] = [
  { id: "avif", label: "AVIF", lossy: true, lossless: true, hint: "Il più compatto per le foto. Supportato da tutti i browser moderni." },
  { id: "webp", label: "WebP", lossy: true, lossless: true, hint: "Ottimo compromesso tra peso e compatibilità." },
  { id: "jxl", label: "JPEG XL", lossy: false, lossless: true, hint: "Per ora solo senza perdita. Supporto browser ancora limitato." },
  { id: "jpeg", label: "JPEG", lossy: true, lossless: false, hint: "Massima compatibilità. La trasparenza diventa bianca." },
  { id: "png", label: "PNG", lossy: false, lossless: true, hint: "Senza perdita, con trasparenza. File più grandi." },
  { id: "tiff", label: "TIFF", lossy: false, lossless: true, hint: "Per stampa e archiviazione." },
  { id: "gif", label: "GIF", lossy: false, lossless: false, hint: "Massimo 256 colori." },
  { id: "bmp", label: "BMP", lossy: false, lossless: true, hint: "Non compresso." },
  { id: "ico", label: "ICO", lossy: false, lossless: true, hint: "Icona Windows, ridotta a 256×256 se più grande." },
  { id: "qoi", label: "QOI", lossy: false, lossless: true, hint: "Formato lossless velocissimo." },
];

const META_FORMATS: Format[] = ["jpeg", "png", "webp", "tiff", "jxl", "avif"];

const TAG_LABELS: Record<string, string> = {
  ImageDescription: "Descrizione",
  Artist: "Autore",
  Copyright: "Copyright",
  DateTimeOriginal: "Data scatto (AAAA:MM:GG hh:mm:ss)",
  Make: "Produttore fotocamera",
  Model: "Modello fotocamera",
  Software: "Software",
};
const EDITABLE_TAGS = Object.keys(TAG_LABELS);

// ---- State ----

interface Settings {
  format: Format;
  quality: number;
  lossless: boolean;
  speed: number;
  resizeMode: "none" | "fit" | "percent" | "exact";
  resizeW: string;
  resizeH: string;
  resizePercent: string;
  meta: MetaMode;
  edits: Record<string, string>;
  dest: "same" | "custom";
  outputDir: string;
  suffix: string;
  overwrite: boolean;
}

const DEFAULTS: Settings = {
  format: "avif",
  quality: 75,
  lossless: false,
  speed: 6,
  resizeMode: "none",
  resizeW: "1920",
  resizeH: "1920",
  resizePercent: "50",
  meta: "keep_no_gps",
  edits: {},
  dest: "same",
  outputDir: "",
  suffix: "",
  overwrite: false,
};

function loadSettings(): Settings {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem("settings") ?? "{}") };
  } catch {
    return { ...DEFAULTS };
  }
}

const settings = loadSettings();

function saveSettings() {
  try {
    localStorage.setItem("settings", JSON.stringify(settings));
  } catch {
    // Storage unavailable: settings just won't persist.
  }
}

type Status = { kind: "idle" } | { kind: "working" } | { kind: "done"; outcome: Outcome } | { kind: "error"; message: string };

interface Item {
  entry: FileEntry;
  status: Status;
  thumb?: string;
  row?: HTMLLIElement;
}

const items = new Map<string, Item>();
let running = false;

// ---- Helpers ----

const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector(sel) as T;

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, children: (Node | string)[] = []): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, props);
  node.append(...children);
  return node;
}

// ---- Tabs ----

function showTab(name: string) {
  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
  document.querySelectorAll(".panel").forEach((p) => p.classList.toggle("active", p.id === `tab-${name}`));
}

document.querySelectorAll<HTMLButtonElement>(".tab").forEach((t) => t.addEventListener("click", () => showTab(t.dataset.tab!)));

// ---- File list ----

async function addPaths(paths: string[]) {
  if (!paths.length || running) return;
  const entries = await invoke<FileEntry[]>("expand_paths", { paths });
  for (const entry of entries) {
    if (!items.has(entry.path)) items.set(entry.path, { entry, status: { kind: "idle" } });
  }
  renderList();
  loadThumbnails();
}

function renderList() {
  const list = $<HTMLUListElement>("#file-list");
  list.replaceChildren();
  for (const item of items.values()) {
    item.row = el("li", { className: "file" });
    renderRow(item);
    list.append(item.row);
  }
  const n = items.size;
  $("#list-count").textContent = n === 0 ? "Nessun file" : n === 1 ? "1 immagine" : `${n} immagini`;
  $<HTMLButtonElement>("#clear-list").disabled = n === 0 || running;
  $("#dropzone").classList.toggle("compact", n > 0);
  updateConvertButton();
}

function renderRow(item: Item) {
  const row = item.row!;
  const { entry, status } = item;
  const thumb = item.thumb ? el("img", { src: item.thumb, alt: "" }) : el("div", { className: "thumb-placeholder" });
  const statusEl = el("div", { className: "status" });

  switch (status.kind) {
    case "idle":
      statusEl.textContent = "";
      break;
    case "working":
      statusEl.textContent = "In corso…";
      statusEl.classList.add("working");
      break;
    case "error":
      statusEl.textContent = status.message;
      statusEl.classList.add("error");
      break;
    case "done": {
      const o = status.outcome;
      const delta = Math.round((1 - o.outputBytes / o.inputBytes) * 100);
      const deltaText = delta >= 0 ? `−${delta}%` : `+${-delta}%`;
      const link = el("button", { className: "link", textContent: "Mostra", title: o.output });
      link.addEventListener("click", () => revealItemInDir(o.output));
      statusEl.append(`✓ ${formatBytes(o.outputBytes)} (${deltaText}) · ${o.width}×${o.height} · `, link);
      if (o.notes.length) statusEl.append(el("span", { className: "note", textContent: ` · ${o.notes.join("; ")}` }));
      statusEl.classList.add(delta >= 0 ? "ok" : "warn");
      break;
    }
  }

  const info = el("button", { className: "icon", title: "Metadati", textContent: "ⓘ" });
  info.addEventListener("click", () => openMetadata(entry.path));
  const remove = el("button", { className: "icon", title: "Rimuovi dalla lista", textContent: "✕" });
  remove.disabled = running;
  remove.addEventListener("click", () => {
    items.delete(entry.path);
    renderList();
  });

  row.replaceChildren(
    thumb,
    el("div", { className: "file-main" }, [
      el("div", { className: "file-name", textContent: entry.name, title: entry.path }),
      el("div", { className: "file-meta" }, [el("span", { className: "badge", textContent: entry.format.toUpperCase() }), formatBytes(entry.size)]),
      statusEl,
    ]),
    info,
    remove,
  );
}

let thumbsRunning = false;
async function loadThumbnails() {
  if (thumbsRunning) return;
  thumbsRunning = true;
  const pending = () => [...items.values()].filter((i) => i.thumb === undefined);
  const worker = async () => {
    for (let next = pending()[0]; next; next = pending()[0]) {
      next.thumb = "";
      try {
        next.thumb = await invoke<string>("thumbnail", { path: next.entry.path });
      } catch {
        // No preview (e.g. HEIC): keep the placeholder.
      }
      if (next.row) renderRow(next);
    }
  };
  await Promise.all([worker(), worker(), worker()]);
  thumbsRunning = false;
}

$("#add-files").addEventListener("click", async () => {
  const picked = await open({
    multiple: true,
    filters: [{ name: "Immagini", extensions: ["jpg", "jpeg", "png", "webp", "avif", "jxl", "heic", "heif", "tif", "tiff", "gif", "bmp", "ico", "qoi"] }],
  });
  if (picked) addPaths(picked);
});

$("#add-folder").addEventListener("click", async () => {
  const picked = await open({ directory: true, multiple: true });
  if (picked) addPaths(picked);
});

$("#clear-list").addEventListener("click", () => {
  items.clear();
  renderList();
  $("#summary").textContent = "";
  setProgress(0);
});

getCurrentWebview().onDragDropEvent((event) => {
  const zone = $("#dropzone");
  const type = event.payload.type;
  if (type === "enter" || type === "over") {
    zone.classList.add("hover");
  } else {
    zone.classList.remove("hover");
    if (type === "drop") {
      const paths = event.payload.paths;
      if (document.querySelector("#tab-metadata.active") && paths.length) openMetadata(paths[0]);
      else addPaths(paths);
    }
  }
});

// ---- Options ----

function buildEditGrid(container: HTMLElement, onInput: (tag: string, value: string) => void, values: Record<string, string>) {
  container.replaceChildren();
  for (const tag of EDITABLE_TAGS) {
    const input = el("input", { type: "text", value: values[tag] ?? "", placeholder: tag === "DateTimeOriginal" ? "2026:10:24 15:30:00" : "" });
    input.dataset.tag = tag;
    input.addEventListener("input", () => onInput(tag, input.value));
    container.append(el("label", {}, [el("span", { textContent: TAG_LABELS[tag] }), input]));
  }
}

function initOptions() {
  const formats = $("#formats");
  for (const f of FORMATS) {
    const b = el("button", { textContent: f.label });
    b.dataset.format = f.id;
    b.addEventListener("click", () => {
      settings.format = f.id;
      syncOptions();
    });
    formats.append(b);
  }

  const quality = $<HTMLInputElement>("#quality");
  quality.value = String(settings.quality);
  quality.addEventListener("input", () => {
    settings.quality = Number(quality.value);
    syncOptions();
  });

  const speed = $<HTMLInputElement>("#speed");
  speed.value = String(settings.speed);
  speed.addEventListener("input", () => {
    settings.speed = Number(speed.value);
    syncOptions();
  });

  const lossless = $<HTMLInputElement>("#lossless");
  lossless.checked = settings.lossless;
  lossless.addEventListener("change", () => {
    settings.lossless = lossless.checked;
    syncOptions();
  });

  const bindValue = (sel: string, key: "resizeW" | "resizeH" | "resizePercent" | "suffix") => {
    const input = $<HTMLInputElement>(sel);
    input.value = settings[key];
    input.addEventListener("input", () => {
      settings[key] = input.value;
      saveSettings();
    });
  };
  bindValue("#resize-w", "resizeW");
  bindValue("#resize-h", "resizeH");
  bindValue("#resize-percent", "resizePercent");
  bindValue("#suffix", "suffix");

  const mode = $<HTMLSelectElement>("#resize-mode");
  mode.value = settings.resizeMode;
  mode.addEventListener("change", () => {
    settings.resizeMode = mode.value as Settings["resizeMode"];
    syncOptions();
  });

  document.querySelectorAll<HTMLInputElement>("input[name=meta]").forEach((r) => {
    r.checked = r.value === settings.meta;
    r.addEventListener("change", () => {
      settings.meta = r.value as MetaMode;
      saveSettings();
    });
  });

  buildEditGrid(
    $("#convert-edits"),
    (tag, value) => {
      settings.edits[tag] = value;
      saveSettings();
    },
    settings.edits,
  );

  document.querySelectorAll<HTMLInputElement>("input[name=dest]").forEach((r) => {
    r.checked = r.value === settings.dest;
    r.addEventListener("change", () => {
      settings.dest = r.value as Settings["dest"];
      syncOptions();
    });
  });

  $<HTMLInputElement>("#output-dir").value = settings.outputDir;
  $("#pick-output").addEventListener("click", async () => {
    const dir = await open({ directory: true });
    if (dir) {
      settings.outputDir = dir;
      settings.dest = "custom";
      $<HTMLInputElement>("#output-dir").value = dir;
      document.querySelectorAll<HTMLInputElement>("input[name=dest]").forEach((r) => (r.checked = r.value === "custom"));
      syncOptions();
    }
  });

  const overwrite = $<HTMLInputElement>("#overwrite");
  overwrite.checked = settings.overwrite;
  overwrite.addEventListener("change", () => {
    settings.overwrite = overwrite.checked;
    saveSettings();
  });

  syncOptions();
}

/** Reflects settings in the controls that depend on each other. */
function syncOptions() {
  const info = FORMATS.find((f) => f.id === settings.format)!;
  document.querySelectorAll<HTMLButtonElement>("#formats button").forEach((b) => b.classList.toggle("active", b.dataset.format === info.id));

  const metaNote = META_FORMATS.includes(info.id) ? "" : " Non conserva i metadati EXIF.";
  $("#format-hint").textContent = info.hint + metaNote;

  const lossless = $<HTMLInputElement>("#lossless");
  lossless.disabled = !(info.lossy && info.lossless);
  lossless.checked = info.lossy ? settings.lossless : info.lossless;
  const usesQuality = info.lossy && !(info.lossless && settings.lossless);
  $<HTMLInputElement>("#quality").disabled = !usesQuality;
  $("#quality-out").textContent = usesQuality ? String(settings.quality) : "—";
  $("#quality-box").classList.toggle("muted", !info.lossy);
  $("#speed-row").hidden = info.id !== "avif";
  $("#speed-out").textContent = String(settings.speed);

  $("#resize-wh").hidden = !(settings.resizeMode === "fit" || settings.resizeMode === "exact");
  $("#resize-pct").hidden = settings.resizeMode !== "percent";
  $<HTMLInputElement>("#output-dir").disabled = settings.dest !== "custom";

  saveSettings();
  updateConvertButton();
}

function buildOptions() {
  const num = (s: string) => Math.max(0, Math.round(Number(s) || 0));
  const resize =
    settings.resizeMode === "fit" || settings.resizeMode === "exact"
      ? { mode: settings.resizeMode, width: num(settings.resizeW), height: num(settings.resizeH) }
      : settings.resizeMode === "percent"
        ? { mode: "percent", percent: Number(settings.resizePercent) || 100 }
        : { mode: "none" };
  const edits: Edit[] = Object.entries(settings.edits)
    .filter(([, v]) => v.trim() !== "")
    .map(([tag, value]) => ({ tag, value }));
  return {
    format: settings.format,
    quality: settings.quality,
    lossless: settings.lossless,
    speed: settings.speed,
    resize,
    metadata: settings.meta,
    edits,
    outputDir: settings.dest === "custom" && settings.outputDir ? settings.outputDir : null,
    suffix: settings.suffix,
    overwrite: settings.overwrite,
  };
}

// ---- Conversion ----

function updateConvertButton() {
  const button = $<HTMLButtonElement>("#convert");
  const n = items.size;
  const needsDir = settings.dest === "custom" && !settings.outputDir;
  button.disabled = running || n === 0 || needsDir;
  const label = FORMATS.find((f) => f.id === settings.format)!.label;
  button.textContent = n > 1 ? `Converti ${n} immagini in ${label}` : `Converti in ${label}`;
}

function setProgress(fraction: number) {
  $("#progress-bar").style.width = `${Math.round(fraction * 100)}%`;
}

let done = 0;
let total = 0;

listen<Progress>("progress", ({ payload }) => {
  const item = items.get(payload.path);
  if (!item) return;
  item.status = payload.outcome ? { kind: "done", outcome: payload.outcome } : { kind: "error", message: payload.error ?? "errore" };
  renderRow(item);
  done++;
  setProgress(done / total);
  $("#summary").textContent = `${done} di ${total}`;
});

$("#convert").addEventListener("click", async () => {
  const paths = [...items.keys()];
  running = true;
  done = 0;
  total = paths.length;
  for (const item of items.values()) {
    item.status = { kind: "working" };
  }
  renderList();
  setProgress(0);
  $("#cancel").hidden = false;
  const started = performance.now();
  try {
    await invoke("convert_files", { paths, options: buildOptions() });
  } catch (e) {
    $("#summary").textContent = `Errore: ${e}`;
  }
  running = false;
  $("#cancel").hidden = true;
  renderList();

  const results = [...items.values()].map((i) => i.status);
  const ok = results.filter((s): s is Extract<Status, { kind: "done" }> => s.kind === "done");
  const failed = results.length - ok.length;
  const before = ok.reduce((a, s) => a + s.outcome.inputBytes, 0);
  const after = ok.reduce((a, s) => a + s.outcome.outputBytes, 0);
  const seconds = ((performance.now() - started) / 1000).toFixed(1);
  let text = `${ok.length} convertite in ${seconds}s`;
  if (ok.length) text += ` · ${formatBytes(before)} → ${formatBytes(after)}`;
  if (failed) text += ` · ${failed} con errori`;
  $("#summary").textContent = text;
});

$("#cancel").addEventListener("click", () => invoke("cancel_conversion"));

// ---- Metadata editor ----

let metaPath: string | null = null;
const metaEdits: Record<string, string> = {};

async function openMetadata(path: string) {
  showTab("metadata");
  metaPath = path;
  $("#meta-file").textContent = path;
  $("#meta-status").textContent = "";
  await refreshMetadata();
}

async function refreshMetadata() {
  if (!metaPath) return;
  let view: MetadataView;
  try {
    view = await invoke<MetadataView>("read_metadata", { path: metaPath });
  } catch (e) {
    $("#meta-status").textContent = `Errore: ${e}`;
    return;
  }

  for (const tag of EDITABLE_TAGS) {
    metaEdits[tag] = view.tags.find((t) => t.name === tag)?.value ?? "";
  }
  buildEditGrid($("#meta-edits"), (tag, value) => (metaEdits[tag] = value), metaEdits);
  $<HTMLInputElement>("#meta-remove-gps").checked = false;

  const tbody = $("#meta-tags");
  tbody.replaceChildren();
  if (!view.tags.length) {
    tbody.append(el("tr", {}, [el("td", { colSpan: 3, className: "empty", textContent: "Nessun metadato EXIF" })]));
  }
  for (const t of view.tags) {
    tbody.append(el("tr", { className: t.editable ? "editable" : "" }, [el("td", { textContent: t.name }), el("td", { textContent: t.group }), el("td", { textContent: t.value })]));
  }

  const canWrite = view.writable;
  $<HTMLButtonElement>("#meta-save").disabled = !canWrite;
  $<HTMLButtonElement>("#meta-strip").disabled = !canWrite;
  if (!canWrite) $("#meta-status").textContent = `Il formato ${view.format.toUpperCase()} non supporta la modifica dei metadati.`;
}

async function writeMetadata(stripAll: boolean) {
  if (!metaPath) return;
  const edits = stripAll ? [] : EDITABLE_TAGS.map((tag) => ({ tag, value: metaEdits[tag] ?? "" }));
  const removeGps = $<HTMLInputElement>("#meta-remove-gps").checked;
  try {
    await invoke("write_metadata", { path: metaPath, edits, removeGps, stripAll });
    await refreshMetadata();
    $("#meta-status").textContent = stripAll ? "Metadati rimossi." : "Modifiche salvate.";
  } catch (e) {
    $("#meta-status").textContent = `Errore: ${e}`;
  }
}

$("#meta-open").addEventListener("click", async () => {
  const picked = await open({ filters: [{ name: "Immagini", extensions: ["jpg", "jpeg", "png", "webp", "avif", "jxl", "tif", "tiff"] }] });
  if (picked) openMetadata(picked);
});
$("#meta-save").addEventListener("click", () => writeMetadata(false));
$("#meta-strip").addEventListener("click", () => {
  if (confirm("Rimuovere tutti i metadati EXIF da questo file?")) writeMetadata(true);
});

initOptions();
renderList();
getVersion()
  .then((v) => ($("#app-version").textContent = v))
  .catch(() => {});

// Links open in the system browser / mail client, not inside the app window.
document.querySelectorAll<HTMLAnchorElement>("a[data-external]").forEach((a) =>
  a.addEventListener("click", (e) => {
    e.preventDefault();
    openUrl(a.href);
  }),
);
