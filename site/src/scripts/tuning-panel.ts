// Runtime tuning panel - see docs/runtime-tuning-design.md §7. Renders
// itself entirely from the schema the wasm exports (bb_tuning_schema_json),
// so a new row in src/tuning.rs's tunables! table shows up here with no
// site change. Every edit is a JSON patch pushed through
// bb_tuning_apply_json and lands at the game's next frame boundary. The
// changed set persists in localStorage and is re-applied on reload.
//
// Only a dev-tools wasm (`just build-web-dev`, PR previews) exports the
// `bb_*` C API; on a production build `initTuningPanel` returns without
// showing the panel.

import type { EmscriptenModule } from "./runtime";

type Scalar = number | boolean;
type Value = Scalar | number[];
type Patch = Record<string, Scalar>;

interface Row {
  name: string;
  group: string;
  kind: string;
  min: number;
  max: number;
  default: Value;
  /** Non-empty for array rows: one element per label. */
  labels: string[];
  applies: string;
  doc?: string;
}

const STORAGE_KEY = "bongbong.tuning.diff";
const TAB_KEY = "bongbong.tuning.tab";

const $ = (id: string) => document.getElementById(id) as HTMLElement;
const $input = (id: string) => document.getElementById(id) as HTMLInputElement | HTMLTextAreaElement;

function store(key: string, value: string): void {
  try { localStorage.setItem(key, value); } catch { /* private mode etc. */ }
}
function load(key: string): string | null {
  try { return localStorage.getItem(key); } catch { return null; }
}
function isInt(row: Row): boolean {
  return row.kind === "i32" || row.kind === "u32" || row.kind === "usize";
}
function fmt(v: Value, row: Row): string {
  if (row.kind === "bool") return v ? "true" : "false";
  if (isInt(row)) return String(Math.round(v as number));
  return String(Number(Number(v).toPrecision(5)));
}
function niceStep(row: Row): number {
  if (isInt(row) || row.kind === "bool") return 1;
  const raw = (row.max - row.min) / 500;
  if (raw <= 0) return 0.01;
  const pow = Math.pow(10, Math.floor(Math.log10(raw)));
  const m = raw / pow;
  return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * pow;
}
function sameValue(a: Value, b: Value): boolean {
  if (Array.isArray(a)) return a.every((x, i) => x === (b as number[])[i]);
  return a === b;
}
function escapeAttr(s: unknown): string {
  return String(s).replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

export function initTuningPanel(module: EmscriptenModule): void {
  if (typeof module._bb_tuning_schema_json !== "function") return; // production wasm: no panel
  const panel = $("tuning");
  const byName: Record<string, Row> = {};
  const defaults: Record<string, Value> = {};
  let values: Record<string, Value> = {};
  let activeTab: string | null = null;
  let filter = "";

  const api = (name: string, ret: "number" | "string" | null, argTypes: "string"[] = [], args: unknown[] = []) =>
    module.ccall!(name, ret, argTypes, args);
  const apiString = (name: string) => api(name, "string") as string;

  function status(text: string, isError = false): void {
    const el = $("tun-status");
    el.textContent = text;
    el.className = `status${isError ? " error" : ""}`;
  }
  function rowFor(key: string): Row {
    return byName[key.split(".")[0]];
  }
  /** Element index an `name.label` / `name.<i>` key addresses. */
  function elemIndex(row: Row, elem: string): number {
    const i = row.labels.indexOf(elem);
    return i < 0 ? parseInt(elem, 10) : i;
  }
  function diff(): Record<string, Value> {
    const out: Record<string, Value> = {};
    for (const row of schema) {
      if (!sameValue(values[row.name], defaults[row.name])) out[row.name] = values[row.name];
    }
    return out;
  }
  function persist(): void {
    const d = diff();
    store(STORAGE_KEY, Object.keys(d).length ? JSON.stringify(d) : "");
  }
  // Push a patch to the game; on success mirror it into `values`.
  function apply(patch: Record<string, Value>, note?: string): boolean {
    const rc = api("bb_tuning_apply_json", "number", ["string"], [JSON.stringify(patch)]) as number;
    if (rc < 0) {
      status(apiString("bb_last_error"), true);
      return false;
    }
    for (const key of Object.keys(patch)) {
      const dot = key.indexOf(".");
      if (dot < 0) { values[key] = patch[key]; continue; }
      const row = byName[key.slice(0, dot)];
      (values[row.name] as number[])[elemIndex(row, key.slice(dot + 1))] = patch[key] as number;
    }
    persist();
    status(note || `applied ${rc} knob${rc === 1 ? "" : "s"}`);
    return true;
  }

  // ---- rendering -------------------------------------------------
  function badge(row: Row): string {
    return row.applies === "live" ? "" : `<span title="takes effect on ${row.applies}">${row.applies}</span>`;
  }
  function tooltip(row: Row): string {
    return `${(row.doc || "").trim()}\n\nrange ${row.min} ..= ${row.max}, default ${JSON.stringify(row.default)}`;
  }
  function scalarControl(key: string, row: Row, value: Scalar): string {
    if (row.kind === "bool") {
      return `<input type="checkbox" data-key="${key}"${value ? " checked" : ""}/>`;
    }
    return `<input type="range" data-key="${key}" min="${row.min}" max="${row.max}" step="${niceStep(row)}" value="${value}"/>`;
  }
  function numberBox(key: string, row: Row, value: Scalar, changed: boolean): string {
    return `<input type="number" data-key="${key}" min="${row.min}" max="${row.max}" step="${isInt(row) ? 1 : "any"}" value="${fmt(value, row)}"${changed ? ' class="changed"' : ""}/>`;
  }
  function renderScalars(rows: Row[]): string {
    if (!rows.length) return "";
    let h = "<table><thead><tr><th>knob</th><th></th><th>value</th><th>default</th><th></th><th></th></tr></thead><tbody>";
    for (const row of rows) {
      const v = values[row.name] as Scalar;
      const changed = v !== defaults[row.name];
      h += `<tr><td class="name${changed ? " changed" : ""}" title="${escapeAttr(tooltip(row))}">${row.name}</td>` +
        `<td class="ctl">${scalarControl(row.name, row, v)}</td>` +
        `<td class="val">${numberBox(row.name, row, v, changed)}</td>` +
        `<td class="def">${fmt(row.default, row)}</td>` +
        `<td class="badge">${badge(row)}</td>` +
        `<td><button class="reset" data-reset="${row.name}" title="back to default">&#x21ba;</button></td></tr>`;
    }
    return `${h}</tbody></table>`;
  }
  // Array rows sharing one label set pivot into a grid: one line per
  // label (a tank model, a wall material), one column per knob.
  function renderMatrix(rows: Row[]): string {
    const labels = rows[0].labels;
    let h = "<table><thead><tr><th></th>";
    for (const row of rows) {
      h += `<th title="${escapeAttr(tooltip(row))}">${row.name} ${badge(row)} ` +
        `<button class="reset" data-reset="${row.name}" title="whole column back to default">&#x21ba;</button></th>`;
    }
    h += "</tr></thead><tbody>";
    labels.forEach((label, i) => {
      h += `<tr><td class="name">${label}</td>`;
      for (const row of rows) {
        const v = (values[row.name] as number[])[i];
        const changed = v !== (defaults[row.name] as number[])[i];
        h += `<td>${numberBox(`${row.name}.${label}`, row, v, changed)}</td>`;
      }
      h += "</tr>";
    });
    return `${h}</tbody></table>`;
  }
  function render(): void {
    const groups: string[] = [];
    for (const row of schema) if (!groups.includes(row.group)) groups.push(row.group);
    if (!activeTab || !groups.includes(activeTab)) activeTab = groups[0];
    $("tun-tabs").innerHTML = groups
      .map((g) => `<button data-tab="${g}" aria-selected="${g === activeTab && !filter}">${g}</button>`)
      .join("");
    const rows = schema.filter((row) => (filter ? row.name.includes(filter) : row.group === activeTab));
    const scalars = rows.filter((r) => !r.labels.length);
    const matrices: Record<string, Row[]> = {};
    for (const r of rows.filter((r) => r.labels.length)) {
      const sig = r.labels.join(",");
      (matrices[sig] = matrices[sig] || []).push(r);
    }
    let h = renderScalars(scalars);
    for (const sig of Object.keys(matrices)) {
      const group = matrices[sig];
      const n = group[0].labels.length;
      h += `<h4>${n} × ${group.length} (per ${n === 12 ? "tank model" : "variant"})</h4>${renderMatrix(group)}`;
    }
    if (!rows.length) h = `<p>no knob matches "${escapeAttr(filter)}"</p>`;
    $("tun-body").innerHTML = h;
  }

  // ---- events ----------------------------------------------------
  let pending: Patch = {};
  let timer: number | undefined;
  function queue(key: string, value: Scalar): void {
    pending[key] = value;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      const patch = pending;
      pending = {};
      apply(patch);
      // keep the paired inputs (slider / number box) in sync
      for (const k of Object.keys(patch)) {
        document.querySelectorAll<HTMLInputElement>(`[data-key="${k}"]`).forEach((el) => {
          if (el.type === "checkbox") el.checked = !!patch[k];
          else if (document.activeElement !== el) el.value = el.type === "number" ? fmt(patch[k], rowFor(k)) : String(patch[k]);
        });
      }
      markChanged();
    }, 60);
  }
  function markChanged(): void {
    document.querySelectorAll<HTMLElement>("#tun-body td.name[title]").forEach((td) => {
      const row = byName[td.textContent ?? ""];
      if (row) td.classList.toggle("changed", !sameValue(values[row.name], defaults[row.name]));
    });
    document.querySelectorAll<HTMLInputElement>("#tun-body input[type=number]").forEach((el) => {
      const key = el.dataset.key!;
      const row = rowFor(key);
      const dot = key.indexOf(".");
      let v: Value, d: Value;
      if (dot < 0) { v = values[key]; d = defaults[key]; }
      else {
        const i = row.labels.indexOf(key.slice(dot + 1));
        v = (values[row.name] as number[])[i];
        d = (defaults[row.name] as number[])[i];
      }
      el.classList.toggle("changed", v !== d);
    });
  }
  function readInput(el: HTMLInputElement): Scalar | null {
    const row = rowFor(el.dataset.key!);
    if (el.type === "checkbox") return el.checked;
    let v = parseFloat(el.value);
    if (!isFinite(v)) return null;
    if (isInt(row)) v = Math.round(v);
    return Math.min(row.max, Math.max(row.min, v));
  }
  const body = $("tun-body");
  body.addEventListener("input", (e) => {
    const el = e.target as HTMLInputElement;
    if (!el.dataset || !el.dataset.key) return;
    if (el.type === "number") return; // wait for change on typed values
    const v = readInput(el);
    if (v !== null) queue(el.dataset.key, v);
  });
  body.addEventListener("change", (e) => {
    const el = e.target as HTMLInputElement;
    if (!el.dataset || !el.dataset.key || el.type !== "number") return;
    const v = readInput(el);
    if (v !== null) queue(el.dataset.key, v);
  });
  body.addEventListener("click", (e) => {
    const name = (e.target as HTMLElement).dataset?.reset;
    if (!name) return;
    if (apply({ [name]: defaults[name] }, `${name} reset`)) render();
  });
  $("tun-tabs").addEventListener("click", (e) => {
    const tab = (e.target as HTMLElement).dataset?.tab;
    if (!tab) return;
    activeTab = tab;
    filter = "";
    $input("tun-filter").value = "";
    store(TAB_KEY, tab);
    render();
  });
  $("tun-filter").addEventListener("input", (e) => {
    filter = (e.target as HTMLInputElement).value.trim().toLowerCase();
    render();
  });
  $("tun-reset").addEventListener("click", () => {
    api("bb_tuning_reset", null);
    for (const row of schema) {
      const d = defaults[row.name];
      values[row.name] = Array.isArray(d) ? d.slice() : d;
    }
    persist();
    status("all knobs reset");
    render();
  });
  $("tun-restart").addEventListener("click", () => {
    api("bb_game_restart", null);
    status("round restarted");
    module.canvas?.focus();
  });
  function showInBox(text: string, note: string): void {
    $input("tun-import-text").value = text;
    $("tun-import-box").classList.add("open");
    status(note);
  }
  function copy(text: string, note: string): void {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(
        () => status(note),
        () => showInBox(text, "clipboard blocked - copy from the box"),
      );
    } else {
      showInBox(text, "copy from the box");
    }
  }
  $("tun-copy-json").addEventListener("click", () => {
    copy(JSON.stringify(diff(), null, 2), "changed knobs copied as JSON");
  });
  $("tun-copy-rust").addEventListener("click", () => {
    // Read from the game rather than recomputing: the Rust rendering
    // (types, ranges, labels, applies markers) lives there.
    const rust = apiString("bb_tuning_diff_rust");
    copy(rust || "// nothing differs from the defaults\n", "changed knobs copied as tunables! rows");
  });
  $("tun-import").addEventListener("click", () => {
    $("tun-import-box").classList.toggle("open");
  });
  $("tun-import-apply").addEventListener("click", () => {
    let patch: unknown;
    try { patch = JSON.parse($input("tun-import-text").value); }
    catch (e) { status(`not valid JSON: ${(e as Error).message}`, true); return; }
    if (!patch || typeof patch !== "object" || Array.isArray(patch)) { status("expected a JSON object", true); return; }
    const p = patch as Record<string, Value>;
    if (apply(p, `imported ${Object.keys(p).length} knob(s)`)) render();
  });

  // ---- bring-up ---------------------------------------------------
  const schema: Row[] = JSON.parse(apiString("bb_tuning_schema_json"));
  values = JSON.parse(apiString("bb_tuning_current_json"));
  for (const row of schema) { byName[row.name] = row; defaults[row.name] = row.default; }
  activeTab = load(TAB_KEY);
  const saved = load(STORAGE_KEY);
  if (saved) {
    try {
      const patch = JSON.parse(saved) as Record<string, Value>;
      if (!apply(patch, `restored ${Object.keys(patch).length} saved knob(s)`)) store(STORAGE_KEY, "");
    } catch { store(STORAGE_KEY, ""); }
  }
  panel.hidden = false;
  render();
}
