// Keeps the app version in sync across every file that stores it.
//
//   node scripts/bump-version.mjs          bump for the next commit (used by the pre-commit hook)
//   node scripts/bump-version.mjs 0.3.0    set an explicit version
//
// Bump rule: if the working version is already ahead of HEAD (a manual
// bump), it is kept; otherwise HEAD's patch number is incremented.

import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const root = new URL("..", import.meta.url);
const read = (p) => readFileSync(new URL(p, root), "utf8");
const write = (p, s) => writeFileSync(new URL(p, root), s);

const parse = (v) => v.split(".").map(Number);
const compare = (a, b) => {
  const [x, y] = [parse(a), parse(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
};

function headVersion() {
  try {
    const pkg = execSync("git show HEAD:package.json", { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    return JSON.parse(pkg).version;
  } catch {
    return null; // no commits yet
  }
}

const current = JSON.parse(read("package.json")).version;
let target = process.argv[2];
if (!target) {
  const head = headVersion();
  if (!head || compare(current, head) > 0) {
    target = current;
  } else {
    const [major, minor, patch] = parse(head);
    target = `${major}.${minor}.${patch + 1}`;
  }
}
if (!/^\d+\.\d+\.\d+$/.test(target)) {
  console.error(`Versione non valida: ${target}`);
  process.exit(1);
}

const files = {
  "package.json": (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${target}$2`),
  // Root package entry and its mirror under "packages" > "".
  "package-lock.json": (s) =>
    s.replace(/^(\s{2}"version":\s*")[^"]+(")/m, `$1${target}$2`).replace(/(""\s*:\s*\{\s*"name":\s*"[^"]+",\s*"version":\s*")[^"]+(")/, `$1${target}$2`),
  "src-tauri/tauri.conf.json": (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${target}$2`),
  "src-tauri/Cargo.toml": (s) => s.replace(/^(version = ")[^"]+(")/m, `$1${target}$2`),
  "src-tauri/Cargo.lock": (s) => s.replace(/(name = "metapic"\r?\nversion = ")[^"]+(")/, `$1${target}$2`),
  "index.html": (s) => s.replace(/(<span id="app-version">)[^<]*(<\/span>)/, `$1${target}$2`),
  "README.md": (s) => s.replace(/^(Versione: \*\*)[^*]+(\*\*)/m, `$1${target}$2`),
  VERSION: () => `${target}\n`,
};

for (const [path, update] of Object.entries(files)) {
  let before = "";
  try {
    before = read(path);
  } catch {
    // VERSION may not exist yet.
  }
  const after = update(before);
  if (path !== "VERSION" && after === before && !before.includes(target)) {
    console.error(`Versione non trovata in ${path}`);
    process.exit(1);
  }
  write(path, after);
}

if (process.argv.includes("--stage") || !process.argv[2]) {
  execSync(`git add ${Object.keys(files).join(" ")}`, { cwd: root });
}
console.log(`MetaPIC ${target}`);
