// The Timeline: one lane per feature (or plan, or group of loose items), bars from created to closed coloured by
// outcome, marks for what happened, bursts collapsed, quiet gaps compressed, sessions, a now line, and a story
// mode that tells each day in a sentence. Clicking anything opens it in the same side panel as the Map.
"use strict";

const tlState = { key: null, days: 0, mode: "chart", open: new Set(), zoom: 1, sx: null, sy: null, clusters: [] };
const TL = { LABEL: 250, ROW: 26, AXIS: 38, BREAK: 34, GAP: 4 * 3600e3, PAD: 20 * 60e3 };
const MARK_ORDER = ["dead", "blocker", "reopened", "decision", "merged", "done", "partial", "finding", "handoff", "question", "new", "started", "blocked"];
const POINT_KINDS = new Set(["new", "done", "merged", "dead", "partial", "reopened", "started", "blocked", "decision", "finding", "blocker", "handoff"]);
const tlTime = (s) => Date.parse(s);
const tlClock = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
const tlDay = (t) => new Date(t).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });

async function renderTimeline(key) {
  if (tlState.key !== key) Object.assign(tlState, { key, open: new Set(), zoom: 1, sx: null, sy: null, days: 0 });
  const q = tlState.days ? `?days=${tlState.days}` : "";
  const [v, ins] = await Promise.all([api(`p/${key}/lanes${q}`), api(`p/${key}/insights`)]);
  mapState.ins = ins;
  tlState.v = v;
  tlState.spinning = new Set(ins.branches.filter((b) => b.spinning).map((b) => b.id));
  main().classList.add("wide");
  const chip = (on, attr, label) => `<button class="chip ${on ? "on" : ""}" ${attr}>${label}</button>`;
  const ranges = [[1, "24h"], [3, "3 days"], [14, "2 weeks"], [30, "30 days"], [0, "all"]];
  main().innerHTML =
    `<div class="maphead"><h1>Timeline</h1>` +
    chip(tlState.mode === "chart", 'data-mode="chart"', "Lanes") + chip(tlState.mode === "story", 'data-mode="story"', "Story") +
    `<span class="tl-sep"></span>` + ranges.map(([d, l]) => chip(tlState.days === d, `data-days="${d}"`, l)).join("") +
    (tlState.mode === "chart" ? `<span class="tl-sep"></span><button id="tzin">+</button><button id="tzout">−</button><button id="tlexp">open all lanes</button><button id="tlcol">close all</button>` : "") +
    `</div>` +
    (tlState.mode === "chart"
      ? `<div class="mlegend tl-legend"><span>● new</span><span class="tl-done">✓ done</span><span class="tl-merged">⇢ combined / superseded</span><span class="tl-dead">✗ dropped / failed</span><span class="tl-decision">◆ decision</span><span class="tl-finding">★ finding</span><span class="tl-blocker">⚑ blocker</span><span class="tl-handoff">✉ handoff</span><span class="tl-reopened">↺ reopened</span><span>? question</span><span class="muted">≈ quiet hours squeezed · click a lane to open it, anything to see it on the right</span></div>`
      : "") +
    `<div class="mapgrid"><div class="tlwrap" id="tlwrap"></div><aside class="mpanel" id="mpanel"></aside></div>`;
  main().querySelectorAll("[data-mode]").forEach((b) => (b.onclick = () => { tlState.mode = b.dataset.mode; renderTimeline(key); }));
  main().querySelectorAll("[data-days]").forEach((b) => (b.onclick = () => { tlState.days = +b.dataset.days; tlState.sx = null; renderTimeline(key); }));
  if (tlState.mode === "chart") {
    $("#tzin").onclick = () => { tlState.zoom = Math.min(8, tlState.zoom * 1.5); drawTimeline(); };
    $("#tzout").onclick = () => { tlState.zoom = Math.max(1, tlState.zoom / 1.5); drawTimeline(); };
    $("#tlexp").onclick = () => { v.lanes.forEach((l) => tlState.open.add(l.id)); drawTimeline(); };
    $("#tlcol").onclick = () => { tlState.open.clear(); drawTimeline(); };
    drawTimeline();
  } else {
    storyTimeline();
  }
  renderPanel();
}

// Time → x, with quiet stretches (no events for GAP) squeezed into a fixed-width break.
function tlAxis(v, width) {
  const now = tlTime(v.now);
  const times = [now];
  v.marks.forEach((m) => times.push(tlTime(m.at)));
  v.spans.forEach((s) => { times.push(tlTime(s.created)); if (s.closed) times.push(tlTime(s.closed)); });
  const lo = tlState.days ? now - tlState.days * 864e5 : Math.min(...times);
  const ts = [...new Set(times.filter((t) => t >= lo))].sort((a, b) => a - b);
  const segs = [];
  let a = ts[0] - TL.PAD, b = ts[0] + TL.PAD;
  for (const t of ts) {
    if (t - TL.PAD > b + TL.GAP) { segs.push([a, b]); a = t - TL.PAD; }
    b = Math.max(b, t + TL.PAD);
  }
  segs.push([a, Math.max(b, now + TL.PAD)]);
  const active = segs.reduce((n, s) => n + (s[1] - s[0]), 0);
  const per = (width - 40 - TL.BREAK * (segs.length - 1)) / active;
  const offs = [];
  let acc = 0;
  for (const s of segs) { offs.push(acc); acc += (s[1] - s[0]) * per + TL.BREAK; }
  const L = 18; // room for marks at the left edge
  const x = (t) => L + xr(t);
  const xr = (t) => {
    if (t <= segs[0][0]) return 0;
    for (let i = 0; i < segs.length; i++) {
      if (t < segs[i][0]) return offs[i] - TL.BREAK / 2;
      if (t <= segs[i][1]) return offs[i] + (t - segs[i][0]) * per;
    }
    return width;
  };
  return { x, segs, offs: offs.map((o) => o + L), per, lo, now };
}

function tlClusters(marks, x) {
  const sorted = marks.map((m) => ({ m, x: x(tlTime(m.at)) })).sort((p, q) => p.x - q.x);
  const groups = [];
  for (const p of sorted) {
    const g = groups[groups.length - 1];
    if (g && p.x - g.x1 < 13) { g.ms.push(p.m); g.x1 = p.x; } else groups.push({ x0: p.x, x1: p.x, ms: [p.m] });
  }
  return groups;
}

function tlMarkSvg(g, y) {
  if (g.ms.length === 1) {
    const m = g.ms[0];
    return `<g class="tl-m tl-${esc(m.kind)}" ${m.item ? `data-item="${esc(m.item)}"` : ""}><title>${esc(when(m.at))} · ${esc(m.kind)}${m.item ? " " + esc(m.item) : ""}\n${esc(m.text)}</title><text x="${g.x0}" y="${y + 5}" text-anchor="middle">${esc(m.glyph)}</text></g>`;
  }
  const top = g.ms.slice().sort((p, q) => MARK_ORDER.indexOf(p.kind) - MARK_ORDER.indexOf(q.kind))[0];
  const id = tlState.clusters.push(g.ms) - 1;
  const label = `${top.glyph}${g.ms.length}`;
  const w = Math.max(26, g.x1 - g.x0 + 18, label.length * 7 + 8);
  const cx = (g.x0 + g.x1) / 2;
  const kinds = {};
  g.ms.forEach((m) => (kinds[m.kind] = (kinds[m.kind] || 0) + 1));
  const tip = `${tlClock(tlTime(g.ms[0].at))}–${tlClock(tlTime(g.ms[g.ms.length - 1].at))}: ` + Object.entries(kinds).map(([k, n]) => `${n} ${k}`).join(", ");
  return `<g class="tl-c tl-${esc(top.kind)}" data-cl="${id}"><title>${esc(tip)}</title><rect x="${cx - w / 2}" y="${y - 8}" width="${w}" height="16" rx="8"/><text x="${cx}" y="${y + 4}" text-anchor="middle">${esc(label)}</text></g>`;
}

function drawTimeline() {
  const v = tlState.v, wrap = $("#tlwrap");
  if (!v || !wrap) return;
  tlState.clusters = [];
  const W = Math.max(500, (wrap.clientWidth - TL.LABEL - 24) * tlState.zoom);
  const ax = tlAxis(v, W);
  const x = ax.x, nowX = x(ax.now);
  const inRange = (m) => tlTime(m.at) >= ax.lo;
  const fresh = new Map((state.items || []).filter((i) => i.fresh).map((i) => [i.id, i.fresh]));
  const labels = [], rows = [];
  let y = TL.AXIS;
  const row = (labelHtml, cls, body) => { labels.push(`<div class="tl-label ${cls}" style="height:${TL.ROW}px">${labelHtml}</div>`); rows.push(body(y + TL.ROW / 2)); y += TL.ROW; };

  // Sessions, with handoffs and entries not filed under an item.
  row(`<span class="tl-lt">Sessions</span><span class="faint small">${v.sessions.length}</span>`, "tl-sess", (cy) => {
    let s = v.sessions.map((ss) => {
      const a = x(Math.max(tlTime(ss.started), ax.lo)), b = x(tlTime(ss.ended || ss.last_seen || v.now));
      return `<rect class="tl-session" x="${a}" y="${cy - 5}" width="${Math.max(3, b - a)}" height="10" rx="5"><title>${esc(ss.agent)} session ${esc(ss.id.slice(-6))}: ${esc(when(ss.started))} → ${ss.ended ? esc(when(ss.ended)) : "open"}</title></rect>`;
    }).join("");
    const loose = v.marks.filter((m) => m.lane === "~sessions" && POINT_KINDS.has(m.kind) && inRange(m));
    s += tlClusters(loose, x).map((g) => tlMarkSvg(g, cy)).join("");
    return s;
  });
  // Activity: every entry, progress included, as a histogram.
  row(`<span class="tl-lt">Activity</span>`, "tl-sess", (cy) => {
    const bins = new Map();
    v.marks.filter(inRange).forEach((m) => { const b = Math.floor(x(tlTime(m.at)) / 6); bins.set(b, (bins.get(b) || 0) + 1); });
    const max = Math.max(1, ...bins.values());
    return [...bins].map(([b, n]) => { const h = 3 + (n / max) * (TL.ROW - 8); return `<rect class="tl-hist" x="${b * 6}" y="${cy + TL.ROW / 2 - 3 - h}" width="5" height="${h}"><title>${n} entries</title></rect>`; }).join("");
  });

  const spansBy = new Map();
  v.spans.forEach((s) => { if (!spansBy.has(s.lane)) spansBy.set(s.lane, []); spansBy.get(s.lane).push(s); });
  const marksBy = new Map();
  v.marks.filter((m) => m.lane !== "~sessions" && POINT_KINDS.has(m.kind) && inRange(m)).forEach((m) => { if (!marksBy.has(m.lane)) marksBy.set(m.lane, []); marksBy.get(m.lane).push(m); });
  const bar = (s, cy, h, cls) => {
    const a = x(Math.max(tlTime(s.created), ax.lo)), b = x(s.closed ? tlTime(s.closed) : ax.now);
    const f = fresh.get(s.id);
    const hatch = !s.closed && f && ["stale", "overdue"].includes(f.reason) ? " tl-idle" : "";
    let out = `<rect class="tl-bar tl-o-${esc(s.outcome)}${hatch} ${cls}" data-item="${esc(s.id)}" x="${a}" y="${cy - h / 2}" width="${Math.max(2, b - a)}" height="${h}" rx="${h / 2}"><title>${esc(s.id)} ${esc(s.title)}\n${esc(when(s.created))} → ${s.closed ? esc(when(s.closed)) + " (" + esc(s.status) + ")" : "open"}${f ? "\n" + esc(f.detail) : ""}</title></rect>`;
    if (s.due) {
      const d = Date.parse(s.due + "T23:59:00");
      const dx = d <= ax.now ? x(d) : Math.min(W - 6, nowX + 14);
      out += `<g class="tl-due ${!s.closed && d < ax.now ? "late" : ""}"><title>due ${esc(s.due)}</title><text x="${dx}" y="${cy + 5}" text-anchor="middle">◇</text></g>`;
    }
    return out;
  };

  for (const l of v.lanes) {
    const spans = (spansBy.get(l.id) || []).slice().sort((a, b) => tlTime(a.created) - tlTime(b.created));
    const marks = marksBy.get(l.id) || [];
    if (!spans.length && !marks.length) continue;
    const open = tlState.open.has(l.id);
    const isItem = !l.id.startsWith("~");
    const att = spans.filter((s) => fresh.has(s.id) && fresh.get(s.id).reason !== "unexplained").length;
    const lbl = `<span class="tl-tog">${open ? "▾" : "▸"}</span><span class="tl-lt" title="${esc(l.title)}">${isItem ? `<span class="mono faint">${esc(l.id)}</span> ` : ""}${esc(l.title)}</span>` +
      `<span class="tl-counts">${tlState.spinning.has(l.id) ? '<b class="spinb" title="spinning">↻</b> ' : ""}${att ? `<span class="fb">${att}!</span> ` : ""}<span class="oc-done">${l.closed}✓</span> <span class="muted">${l.open} open</span></span>`;
    row(lbl, `tl-lane ${isItem && l.status === "active" ? "tl-active" : ""}" data-lane="${esc(l.id)}`, (cy) => {
      let s = "";
      if (spans.length) {
        const a = Math.min(...spans.map((p) => tlTime(p.created)));
        const allClosed = spans.every((p) => p.closed);
        const b = allClosed ? Math.max(...spans.map((p) => tlTime(p.closed))) : ax.now;
        s += `<rect class="tl-lanebar ${allClosed ? "closed" : ""}" x="${x(Math.max(a, ax.lo))}" y="${cy - 3}" width="${Math.max(2, x(b) - x(Math.max(a, ax.lo)))}" height="6" rx="3"/>`;
      }
      return s + tlClusters(marks, x).map((g) => tlMarkSvg(g, cy)).join("");
    });
    if (open) {
      for (const sp of spans) {
        const ms = marks.filter((m) => m.item === sp.id);
        row(`<span class="tl-ind"></span><a class="tl-it" href="#/i/${esc(sp.id)}" data-item="${esc(sp.id)}"><span class="mono faint">${esc(sp.id)}</span> ${esc(sp.title)}</a>`, "tl-itemrow", (cy) =>
          bar(sp, cy, 12, "") + tlClusters(ms, x).map((g) => tlMarkSvg(g, cy)).join(""));
      }
    }
  }

  // Axis: days, times at the start of each stretch, the squeezed breaks, and now.
  let axis = "";
  let lastDay = "";
  let lastLabel = -1e9;
  const timeLabel = (tx, t, anchor) => {
    if (tx - lastLabel < 58) return "";
    lastLabel = tx;
    return `<text class="tl-time" x="${tx}" y="29" ${anchor ? `text-anchor="${anchor}"` : ""}>${esc(tlClock(t))}</text>`;
  };
  ax.segs.forEach((sg, i) => {
    const x0 = ax.offs[i];
    const d = tlDay(sg[0]);
    if (d !== lastDay) { axis += `<text class="tl-day" x="${x0 + 2}" y="13">${esc(d)}</text>`; lastDay = d; }
    axis += timeLabel(x0 + 2, sg[0] + TL.PAD);
    const span = (sg[1] - sg[0]) * ax.per;
    const step = 3600e3 * Math.max(1, Math.ceil(110 / (ax.per * 3600e3)));
    for (let t = Math.ceil(sg[0] / step) * step; t < sg[1]; t += step) {
      const tx = x0 + (t - sg[0]) * ax.per;
      if (tx - x0 > 60 && x0 + span - tx > 30) axis += timeLabel(tx, t, "middle") + `<line class="tl-grid" x1="${tx}" x2="${tx}" y1="${TL.AXIS}" y2="${y}"/>`;
    }
    if (i > 0) {
      const gap = sg[0] - ax.segs[i - 1][1];
      const bx = x0 - TL.BREAK / 2;
      const label = gap > 864e5 ? `${Math.round(gap / 864e5)}d` : `${Math.round(gap / 3600e3)}h`;
      axis += `<line class="tl-break" x1="${bx}" x2="${bx}" y1="${TL.AXIS - 6}" y2="${y}"/><text class="tl-breakt" x="${bx}" y="29" text-anchor="middle">≈${label}</text>`;
    }
  });
  axis += `<line class="tl-now" x1="${nowX}" x2="${nowX}" y1="${TL.AXIS - 8}" y2="${y}"/><text class="tl-nowt" x="${nowX - 3}" y="13" text-anchor="end">now</text>`;
  const stripes = rows.map((_, i) => (i % 2 ? `<rect class="tl-stripe" x="0" y="${TL.AXIS + i * TL.ROW}" width="${W}" height="${TL.ROW}"/>` : "")).join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${y + 6}" class="tl-svg"><defs><pattern id="tlhatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="3" height="6" class="tl-hatchp"/></pattern></defs>${stripes}${axis}${rows.join("")}</svg>`;
  wrap.innerHTML = `<div class="tl-grid"><div class="tl-labels"><div style="height:${TL.AXIS}px"></div>${labels.join("")}</div><div class="tl-scroll" id="tlscroll">${svg}</div></div>`;

  const sc = $("#tlscroll");
  if (tlState.sx === null) sc.scrollLeft = sc.scrollWidth; else sc.scrollLeft = tlState.sx;
  if (tlState.sy !== null) wrap.scrollTop = tlState.sy;
  sc.onscroll = () => (tlState.sx = sc.scrollLeft);
  wrap.onscroll = () => (tlState.sy = wrap.scrollTop);
  wrap.querySelectorAll("[data-lane]").forEach((d) => (d.onclick = () => {
    const id = d.dataset.lane;
    tlState.open.has(id) ? tlState.open.delete(id) : tlState.open.add(id);
    if (!id.startsWith("~")) selectNode(id);
    drawTimeline();
  }));
  wrap.querySelectorAll("[data-item]").forEach((g) => {
    g.onclick = (e) => { e.preventDefault(); e.stopPropagation(); selectNode(g.dataset.item); };
    g.ondblclick = () => (location.hash = "#/i/" + g.dataset.item);
  });
  wrap.querySelectorAll("[data-cl]").forEach((g) => (g.onclick = () => tlClusterPanel(tlState.clusters[+g.dataset.cl])));
  tlHighlight();
}

// The members of a collapsed burst, in the panel.
function tlClusterPanel(ms) {
  mapState.sel = null;
  tlHighlight();
  const el = $("#mpanel");
  const first = tlTime(ms[0].at), last = tlTime(ms[ms.length - 1].at);
  el.innerHTML = `<div class="mp-top"><button id="mpx" title="back">←</button></div><h2>${ms.length} events</h2>` +
    `<p class="small muted">${esc(when(ms[0].at))}${last - first > 60e3 ? " – " + esc(tlClock(last)) : ""}</p>` +
    `<div class="entries">${ms.map((m) => `<div class="entry compact"><span class="tl-${esc(m.kind)}">${esc(m.glyph)}</span><div><div class="when">${esc(tlClock(tlTime(m.at)))} · ${esc(m.kind)}${m.item ? ` <a class="mono" href="#/i/${esc(m.item)}">${esc(m.item)}</a>` : ""} · ${esc(m.agent || "")}</div><div class="text two-lines">${inline(m.text)}</div></div></div>`).join("")}</div>`;
  $("#mpx").onclick = () => selectNode(null);
  panelLinks(el);
}

function tlHighlight() {
  const wrap = $("#tlwrap");
  if (!wrap) return;
  wrap.querySelectorAll(".tl-sel").forEach((e) => e.classList.remove("tl-sel"));
  wrap.classList.toggle("has-sel", !!mapState.sel);
  if (mapState.sel) wrap.querySelectorAll(`[data-item="${CSS.escape(mapState.sel)}"], [data-lane="${CSS.escape(mapState.sel)}"]`).forEach((e) => e.classList.add("tl-sel"));
}

// ---------------------------------------------------------------- story mode: each day in a sentence

function storyTimeline() {
  const v = tlState.v, wrap = $("#tlwrap");
  const laneTitle = new Map(v.lanes.map((l) => [l.id, l.title]));
  const byDay = new Map();
  for (const m of v.marks) {
    const d = new Date(m.at).toDateString();
    if (!byDay.has(d)) byDay.set(d, []);
    byDay.get(d).push(m);
  }
  const names = (ids) => ids.slice(0, 3).map((i) => `<a class="mono" href="#/i/${esc(i)}">${esc(i)}</a>`).join(", ") + (ids.length > 3 ? ` +${ids.length - 3}` : "");
  const topLane = (ms) => {
    const c = {};
    ms.forEach((m) => (c[m.lane] = (c[m.lane] || 0) + 1));
    const [l] = Object.entries(c).sort((a, b) => b[1] - a[1])[0] || [];
    return l && !l.startsWith("~") ? ` in <a href="#/i/${esc(l)}">${esc(shortText(laneTitle.get(l) || l, 40))}</a>` : "";
  };
  let h = "";
  for (const [d, ms] of [...byDay].sort((a, b) => Date.parse(b[0]) - Date.parse(a[0]))) {
    const of = (k) => ms.filter((m) => m.kind === k);
    const closed = ms.filter((m) => ["done", "merged", "dead", "partial"].includes(m.kind));
    const parts = [];
    if (closed.length) {
      const counts = [["done", "✓"], ["merged", "⇢"], ["dead", "✗"], ["partial", "~"]].map(([k, g]) => { const n = of(k).length; return n ? `${n} ${g}` : ""; }).filter(Boolean).join(", ");
      const sec = {};
      closed.forEach((m) => (sec[m.at] = (sec[m.at] || 0) + 1));
      const [batchAt, batchN] = Object.entries(sec).sort((a, b) => b[1] - a[1])[0];
      parts.push(`Closed ${closed.length} (${counts})${topLane(closed)}${batchN >= 5 ? `, ${batchN} of them in one batch at ${esc(tlClock(Date.parse(batchAt)))}` : ""}.`);
    }
    const made = of("new");
    if (made.length) {
      const min = {};
      made.forEach((m) => { const k = m.at.slice(0, 16); min[k] = (min[k] || 0) + 1; });
      const burst = Math.max(...Object.values(min));
      parts.push(`Opened ${made.length}${topLane(made)}${burst >= 8 ? ` (${burst} within a minute: a plan written down)` : ""}.`);
    }
    if (of("decision").length) parts.push(`Decided ${names(of("decision").map((m) => m.item).filter(Boolean))}.`);
    if (of("reopened").length) parts.push(`<span class="tl-reopened">Reopened ${names(of("reopened").map((m) => m.item))}.</span>`);
    if (of("blocker").length || of("blocked").length) parts.push(`<span class="tl-blocker">${of("blocker").length + of("blocked").length} blocker${of("blocker").length + of("blocked").length > 1 ? "s" : ""}.</span>`);
    if (of("finding").length) parts.push(`${of("finding").length} finding${of("finding").length > 1 ? "s" : ""}.`);
    const prog = of("progress");
    if (prog.length) {
      const c = {};
      prog.forEach((m) => m.item && (c[m.item] = (c[m.item] || 0) + 1));
      const top = Object.entries(c).sort((a, b) => b[1] - a[1])[0];
      parts.push(`${prog.length} progress notes${top ? `, mostly on <a class="mono" href="#/i/${esc(top[0])}">${esc(top[0])}</a>` : ""}.`);
    }
    if (of("handoff").length) parts.push(`${of("handoff").length} handoff${of("handoff").length > 1 ? "s" : ""}.`);
    // Key events, with runs of the same kind in the same lane folded.
    const key = ms.filter((m) => POINT_KINDS.has(m.kind) && m.kind !== "started").sort((a, b) => tlTime(b.at) - tlTime(a.at));
    const runs = [];
    for (const m of key) {
      const r = runs[runs.length - 1];
      if (r && r.kind === m.kind && r.lane === m.lane && tlTime(r.ms[r.ms.length - 1].at) - tlTime(m.at) < 10 * 60e3) r.ms.push(m);
      else runs.push({ kind: m.kind, lane: m.lane, ms: [m] });
    }
    const line = (m) => `<div class="tl-ev" ${m.item ? `data-item="${esc(m.item)}"` : ""}><span class="tl-evg tl-${esc(m.kind)}">${esc(m.glyph)}</span><span class="when">${esc(tlClock(tlTime(m.at)))}</span>${m.item ? `<a class="mono" href="#/i/${esc(m.item)}">${esc(m.item)}</a>` : "<span></span>"}<span class="text two-lines">${inline(m.text)}</span></div>`;
    const evs = runs.slice(0, 40).map((r) => r.ms.length < 3 ? r.ms.map(line).join("") :
      `<details class="tl-run"><summary><span class="tl-evg tl-${esc(r.kind)}">${esc(r.ms[0].glyph)}×${r.ms.length}</span> <span class="muted">${esc(r.kind)}${r.lane.startsWith("~") ? "" : " · " + esc(shortText(laneTitle.get(r.lane) || r.lane, 40))} · ${esc(tlClock(tlTime(r.ms[r.ms.length - 1].at)))}</span> ${names(r.ms.map((m) => m.item).filter(Boolean))}</summary>${r.ms.map(line).join("")}</details>`).join("");
    h += `<section class="tl-dayc"><h2>${esc(tlDay(Date.parse(d)))} <span class="n">${ms.length} events</span></h2><p class="tl-sum">${parts.join(" ") || "Quiet."}</p><div class="tl-evs">${evs}</div></section>`;
  }
  wrap.innerHTML = h || '<div class="empty">Nothing recorded in this period.</div>';
  wrap.querySelectorAll(".tl-ev[data-item]").forEach((e) => (e.onclick = (ev) => { ev.preventDefault(); selectNode(e.dataset.item); }));
  panelLinks(wrap);
}
