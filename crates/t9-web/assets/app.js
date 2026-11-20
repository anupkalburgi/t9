// t9 web UI: a small single-page app over the local JSON API. No dependencies.
"use strict";

const $ = (sel) => document.querySelector(sel);
const main = () => $("#main");

// ---------------------------------------------------------------- data

async function api(path) {
  const r = await fetch("/api/" + path, { headers: { Accept: "application/json" } });
  const j = await r.json();
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
async function post(path, body) {
  const r = await fetch("/api/" + path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-T9": "1" },
    body: JSON.stringify(body),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}

// ---------------------------------------------------------------- formatting

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const ID_RE = /\b([A-Z][A-Z0-9]{1,9}-\d+)\b/g;
const linkIds = (html) => html.replace(ID_RE, '<a class="mono" href="#/i/$1">$1</a>');

function inline(s) {
  let h = esc(s);
  h = h.replace(/`([^`]+)`/g, (_, c) => "<code>" + c + "</code>");
  h = h.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  h = h.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  h = h.replace(/(^|[\s(])(https?:\/\/[^\s<)]+)/g, '$1<a href="$2" target="_blank" rel="noopener noreferrer">$2</a>');
  // Item ids and file paths, but not inside code or links.
  return h.split(/(<code>.*?<\/code>|<a .*?<\/a>)/).map((part, i) => (i % 2 ? part : linkFiles(linkIds(part)))).join("");
}
const FILE_RE = /(^|[\s(:])((?:\.\/)?(?:[\w.-]+\/)+[\w.-]+\.(?:md|txt|json|log|csv|yaml|yml|toml|sql|py|rs|ts|js|sh))(?=$|[\s),.;:])/g;
function linkFiles(html) {
  if (!state.key) return html;
  return html.replace(FILE_RE, (m, pre, path) => `${pre}<a class="mono" href="#/f/${esc(state.key)}/${encodeURIComponent(path)}" title="open the file">${path}</a>`);
}

// A small, safe Markdown subset: everything is escaped first; only known tags are produced.
function md(text) {
  const lines = String(text || "").split("\n");
  const out = [];
  let list = null, para = [], code = null;
  const flush = () => {
    if (para.length) { out.push("<p>" + inline(para.join(" ")) + "</p>"); para = []; }
    if (list) { out.push(`<${list.tag}>` + list.items.map((i) => "<li>" + inline(i) + "</li>").join("") + `</${list.tag}>`); list = null; }
  };
  let table = null;
  const cells = (l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());
  const flushTable = () => {
    if (!table) return;
    const [head, ...body] = table;
    out.push('<table class="t">' + "<tr>" + head.map((c) => "<th>" + inline(c) + "</th>").join("") + "</tr>" +
      body.map((r) => "<tr>" + r.map((c) => "<td>" + inline(c) + "</td>").join("") + "</tr>").join("") + "</table>");
    table = null;
  };
  for (const line of lines) {
    if (code === null && /^\s*\|.*\|\s*$/.test(line)) {
      if (!table) { flush(); table = []; }
      if (!/^\s*\|?[\s:|-]+\|?\s*$/.test(line)) table.push(cells(line));
      continue;
    }
    flushTable();
    if (code !== null) {
      if (line.trim().startsWith("```")) { out.push("<pre>" + esc(code.join("\n")) + "</pre>"); code = null; } else code.push(line);
      continue;
    }
    if (line.trim().startsWith("```")) { flush(); code = []; continue; }
    const h = line.match(/^(#{1,3})\s+(.*)$/);
    const ul = line.match(/^\s*[-*]\s+(.*)$/);
    const ol = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (h) { flush(); out.push(`<h3>${inline(h[2])}</h3>`); }
    else if (ul || ol) {
      const tag = ul ? "ul" : "ol";
      if (para.length) flush();
      if (!list || list.tag !== tag) { flush(); list = { tag, items: [] }; }
      list.items.push((ul || ol)[1]);
    } else if (!line.trim()) flush();
    else if (list && /^\s{2,}/.test(line)) list.items[list.items.length - 1] += " " + line.trim();
    else { if (list) flush(); para.push(line.trim()); }
  }
  if (code !== null) out.push("<pre>" + esc(code.join("\n")) + "</pre>");
  flushTable();
  flush();
  return out.join("");
}

const tz = new Intl.DateTimeFormat([], { timeZoneName: "short" }).formatToParts(new Date()).find((p) => p.type === "timeZoneName")?.value || "";
function when(ts, withDate = true) {
  if (!ts) return "";
  const d = new Date(ts);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (!withDate) return `${time} ${tz}`;
  const date = d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric", year: d.getFullYear() !== new Date().getFullYear() ? "numeric" : undefined });
  return `${date}, ${time} ${tz}`;
}
function ago(ts) {
  if (!ts) return "";
  const s = Math.max(0, (Date.now() - new Date(ts)) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return Math.floor(s / 60) + "m ago";
  if (s < 86400) return Math.floor(s / 3600) + "h ago";
  if (s < 172800) return "yesterday";
  if (s < 604800) return Math.floor(s / 86400) + "d ago";
  return new Date(ts).toLocaleDateString([], { month: "short", day: "numeric" });
}
function dot(ts) {
  const m = (Date.now() - new Date(ts)) / 60000;
  const c = m < 15 ? "r15" : m < 60 ? "r60" : m < 1440 ? "rday" : m < 10080 ? "rweek" : "rold";
  return `<span class="dot ${c}" title="changed ${esc(ago(ts))}"></span>`;
}
const kind = (k) => `<span class="badge k-${esc(k)}">${esc(k)}</span>`;
const status = (s) => `<span class="status s-${esc(s)}">${esc(s)}</span>`;
const prio = (p) => (p === null || p === undefined ? "" : `<span class="p${p}">p${p}</span> `);
const touched = (i) => i.touched || i.updated;

function row(i, extra = "") {
  const isNew = state.newest && state.newest === i.id;
  return `<a class="row" href="#/i/${esc(i.id)}">${dot(touched(i))}<span class="id">${esc(i.id)}</span>${kind(i.kind)}` +
    `<span class="title">${prio(i.priority)}${esc(i.title)}${isNew ? '<span class="new">NEW</span>' : ""}</span>` +
    `<span class="extra">${extra}</span></a>`;
}
function rows(list, fmt, cap = 8, more) {
  if (!list || !list.length) return '<div class="empty">Nothing here.</div>';
  let h = '<div class="rows">' + list.slice(0, cap).map(fmt).join("") + "</div>";
  if (list.length > cap) h += `<div class="more">${more ? `<a href="${more}">` : ""}… ${list.length - cap} more${more ? "</a>" : ""}</div>`;
  return h;
}
function card(title, n, body, cls = "") {
  return `<section class="card ${cls}"><h2>${title}${n !== null && n !== undefined ? ` <span class="n">${n}</span>` : ""}</h2>${body}</section>`;
}
// Two-line entry for narrow columns.
function entryCompact(e) {
  return `<div class="entry compact">${dot(e.at)}<div><div class="when">${esc(when(e.at))} · <span class="e-${esc(e.kind)}">${esc(e.kind)}</span>` +
    `${e.item ? ` <a class="mono" href="#/i/${esc(e.item)}">${esc(e.item)}</a>` : ""} · ${esc(e.agent)}</div><div class="text two-lines">${inline(e.body)}</div></div></div>`;
}
function entryRow(e, clip = true) {
  return `<div class="entry">${dot(e.at)}<span class="when" title="${esc(when(e.at))}">${esc(when(e.at))}</span>` +
    `<span class="kind e-${esc(e.kind)}">${esc(e.kind)}${e.item ? ` <a class="mono" href="#/i/${esc(e.item)}">${esc(e.item)}</a>` : ""}</span>` +
    `<span class="text ${clip ? "clip" : ""}" title="${esc(e.agent)}">${inline(e.body)} <span class="faint small">— ${esc(e.agent)}</span></span></div>`;
}
const progressBar = (done, total) =>
  `<div class="prog"><div class="bar"><span style="width:${total ? Math.round((100 * done) / total) : 0}%"></span></div><span>${done}/${total} done</span></div>`;
const legend = () =>
  `<div class="legend"><span><span class="dot r15"></span>15 min</span><span><span class="dot r60"></span>1 hour</span>` +
  `<span><span class="dot rday"></span>today</span><span><span class="dot rweek"></span>this week</span><span><span class="new">NEW</span> most recent</span></div>`;

// ---------------------------------------------------------------- state and routing

const state = { key: null, projects: [], items: null, newest: null, latest: null, typing: false };

function setNav(active) {
  const k = state.key;
  if (!k) { $("#nav").innerHTML = navProjects(); return; }
  const items = state.items || [];
  const open = (f) => items.filter((i) => f(i) && !closedStatus(i.status)).length;
  const q = open((i) => i.kind === "question");
  const links = [
    ["overview", "Overview", ""],
    ["plan", "Plan", open((i) => !["decision", "resource"].includes(i.kind))],
    ["questions", "Questions", q, q > 0],
    ["decisions", "Decisions", items.filter((i) => i.kind === "decision" && i.status === "accepted").length],
    ["experiments", "Experiments", items.filter((i) => i.kind === "experiment").length],
    ["resources", "Resources", open((i) => i.kind === "resource")],
    ["timeline", "Timeline", ""],
    ["journal", "Journal", ""],
    ["about", "About", ""],
  ];
  $("#nav").innerHTML = `<h3>${esc(k)}</h3>` + links.map(([r, label, n, hot]) =>
    `<a href="#/p/${esc(k)}/${r}" class="${active === r ? "on" : ""}"><span>${label}</span><span class="count ${hot ? "hot" : ""}">${n === "" ? "" : n}</span></a>`).join("") +
    `<h3>Projects</h3>` + state.projects.filter((p) => p.key !== k).map((p) => `<a href="#/p/${esc(p.key)}"><span>${esc(p.name)}</span><span class="count">${esc(p.key)}</span></a>`).join("");
}
function navProjects() {
  return `<h3>Projects</h3>` + state.projects.map((p) => `<a href="#/p/${esc(p.key)}"><span>${esc(p.name)}</span><span class="count">${esc(p.key)}</span></a>`).join("");
}
const closedStatus = (s) => ["done", "dropped", "superseded", "rejected", "failed", "inconclusive"].includes(s);

async function loadItems(key) {
  state.items = await api(`p/${key}/items`);
  const live = state.items.slice().sort((a, b) => new Date(touched(b)) - new Date(touched(a)));
  state.newest = live.length ? live[0].id : null;
}

async function route() {
  const h = location.hash.replace(/^#\/?/, "");
  const [a, b, c] = h.split("/").map(decodeURIComponent);
  try {
    if (!state.projects.length) state.projects = (await api("projects")).projects;
    if (a === "f" && b && c) {
      if (state.key !== b || !state.items) { state.key = b; await loadItems(b); }
      setNav(""); crumb(b);
      return renderFile(b, h.split("/").slice(2).join("/"));
    }
    if (a === "i" && b) {
      const page = await api("item/" + encodeURIComponent(b));
      if (state.key !== page.item.project || !state.items) { state.key = page.item.project; await loadItems(state.key); }
      setNav(""); crumb(page.item.project);
      return renderItem(page);
    }
    if (a === "p" && b) {
      if (state.key !== b || !state.items) { state.key = b; await loadItems(b); }
      const view = c || "overview";
      setNav(view.split("?")[0]); crumb(b);
      const views = { overview: renderOverview, plan: renderPlan, questions: renderQuestions, decisions: renderDecisions, experiments: renderExperiments, resources: renderResources, timeline: renderTimeline, journal: renderJournal, about: renderAbout, search: renderSearch };
      return (views[view.split("?")[0]] || renderOverview)(b, view);
    }
    state.key = null; state.items = null; setNav(""); crumb(null);
    return renderProjects();
  } catch (e) {
    main().innerHTML = `<p class="err">${esc(e.message)}</p>`;
  }
}
function crumb(key) {
  const p = state.projects.find((x) => x.key === key);
  $("#crumb").innerHTML = p ? `<a href="#/p/${esc(p.key)}">${esc(p.name)} <span class="faint">(${esc(p.key)})</span></a>` : "";
  $("#q").placeholder = key ? `Search ${key} (/)` : "Search all projects (/)";
}

// ---------------------------------------------------------------- pages

async function renderProjects() {
  const data = await api("projects");
  state.projects = data.projects;
  let h = `<h1>Projects</h1><p class="muted">Everything agents and you have recorded, per project.</p>`;
  h += `<table class="t"><tr><th>Project</th><th>Open</th><th>Issues</th><th>Questions</th><th>Last activity</th></tr>` +
    data.projects.map((p) => `<tr><td><a href="#/p/${esc(p.key)}"><strong>${esc(p.name)}</strong></a> <span class="faint mono">${esc(p.key)}</span><div class="faint small">${esc((p.roots || []).join(", "))}</div></td>` +
      `<td>${p.open}</td><td>${p.issues}</td><td class="${p.questions ? "p0" : ""}">${p.questions}</td><td>${p.last ? dot(p.last) + " " + esc(ago(p.last)) : '<span class="faint">—</span>'}</td></tr>`).join("") + `</table>`;
  if (data.unlinked.length) {
    h += `<h2>Folders not set up yet</h2><p class="muted small">Agents worked here without a project. In that folder, run the command shown (or press <code>i</code> on it in <code>t9 tui</code>).</p>` +
      `<table class="t">` + data.unlinked.map((u) => `<tr><td class="mono">${esc(u.cwd)}</td><td>${esc(u.agent)} · ${esc(ago(u.last_seen))}</td><td><code>t9 project init --name "${esc(u.name)}" --key ${esc(u.key)}</code></td></tr>`).join("") + `</table>`;
  }
  main().innerHTML = h;
}

async function renderOverview(key) {
  const o = await api(`p/${key}/overview`);
  const b = o.brief;
  const latestEntry = b.recent && b.recent[0];
  let h = `<h1>${esc(o.project.name)} <span class="faint mono">${esc(key)}</span></h1>` + legend();
  const about = (b.about || []).find((s) => s.key === "what" && s.text.trim());
  h += about ? `<div class="body">${md(about.text)}</div>` : `<p class="muted">No About yet — <a href="#/p/${esc(key)}/about">what it is, stack, how to run it</a>. Agents draft it from the README.</p>`;
  const grid = [];
  const waiting = [
    ...b.issues.filter((i) => i.kind === "question").map((i) => row(i, `asked ${esc(ago(i.created))} · <strong>answer</strong>`)),
    ...(o.proposed || []).map((i) => row(i, "proposed · accept or reject")),
    ...b.in_progress.filter((i) => i.status === "blocked").map((i) => row(i, "blocked")),
  ];
  if (waiting.length) grid.push(card("Waiting on you", waiting.length, `<div class="rows">${waiting.join("")}</div>`, "alert"));
  if (latestEntry) grid.push(card("Latest", null, entryRow(latestEntry, false)));
  if (b.handoff) grid.push(card(`Last handoff <span class="n">${esc(b.handoff.agent)} · ${esc(ago(b.handoff.meta?.at || b.handoff.at))}</span>`, null, `<div class="body">${md(b.handoff.body)}</div>`, "wide"));
  if (b.focus) grid.push(card("Agent is working on", null, `<div class="rows">${row(b.focus, esc(b.session?.agent || ""))}</div>`));
  for (const pv of b.plans) {
    const outline = pv.outline.map(([d, i, open]) => row(i, `${open} open`)).join("");
    grid.push(card(`Plan <a class="mono" href="#/i/${esc(pv.plan.id)}">${esc(pv.plan.id)}</a>`, null,
      `<p><strong>${esc(pv.plan.title)}</strong></p>${progressBar(pv.done, pv.total)}<div class="rows" style="margin-top:8px">${outline}</div><div class="more"><a href="#/p/${esc(key)}/plan">Full plan →</a></div>`, "wide"));
  }
  grid.push(card("In progress", b.in_progress.filter((i) => i.status === "active").length, rows(b.in_progress.filter((i) => i.status === "active"), (i) => row(i, esc(ago(i.updated))))));
  if (b.experiments.length || b.worth_retrying.length)
    grid.push(card("Experiments", b.experiments.length, rows(b.experiments, (i) => row(i, "running")) +
      (b.worth_retrying.length ? `<h3>Worth retrying</h3>` + rows(b.worth_retrying, (i) => row(i, esc(i.retry_when ? "when: " + i.retry_when : ""))) : "")));
  grid.push(card("Open issues", b.issues.filter((i) => i.kind !== "question").length, rows(b.issues.filter((i) => i.kind !== "question"), (i) => row(i, esc(ago(i.updated))), 10, `#/p/${key}/plan?issues`)));
  grid.push(card("Next up", b.next_up.length, rows(b.next_up, (i) => row(i), 6, `#/p/${key}/plan`)));
  grid.push(card("Decisions in force", b.decisions.length + b.applicable.length,
    rows([...b.applicable, ...b.decisions], (i) => row(i, i.reasons ? "applies to focus" : esc(ago(i.updated))), 8, `#/p/${key}/decisions`)));
  if (b.resources.length) grid.push(card("Resources", b.resources.length, rows(b.resources, (r) => row(r, esc([r.resource_type, r.location].filter(Boolean).join(" · "))), 8, `#/p/${key}/resources`)));
  if (b.ruled_out.length) grid.push(card("Ruled out", b.ruled_out.length, rows(b.ruled_out, (i) => row(i), 5)));
  if (b.ideas.length) grid.push(card("Parked ideas", b.ideas.length, rows(b.ideas, (i) => row(i), 5)));
  grid.push(card("Recent activity", null, `<div class="entries">${(b.recent || []).slice(0, 8).map((e) => entryRow(e)).join("")}</div><div class="more"><a href="#/p/${esc(key)}/timeline">Timeline →</a></div>`, "wide"));
  main().innerHTML = h + `<div class="grid">${grid.join("")}</div>`;
}

// The full plan as a tree, with progress, filters and expand/collapse.
const planState = { closed: false, kinds: new Set(), text: "", collapsed: new Set() };
async function renderPlan(key, view) {
  const items = state.items.filter((i) => !["decision", "resource"].includes(i.kind));
  const issuesOnly = view.includes("?issues");
  const byParent = new Map();
  const ids = new Set(items.map((i) => i.id));
  for (const i of items) {
    const p = i.parent && ids.has(i.parent) ? i.parent : "";
    if (!byParent.has(p)) byParent.set(p, []);
    byParent.get(p).push(i);
  }
  const num = (id) => parseInt(id.split("-").pop(), 10);
  for (const l of byParent.values()) l.sort((a, b) => (a.priority ?? 9) - (b.priority ?? 9) || num(a.id) - num(b.id));
  const stats = new Map();
  const count = (id) => {
    if (stats.has(id)) return stats.get(id);
    let done = 0, total = 0;
    for (const c of byParent.get(id) || []) { total++; if (closedStatus(c.status)) done++; const s = count(c.id); done += s[0]; total += s[1]; }
    stats.set(id, [done, total]); return [done, total];
  };
  const kinds = ["plan", "phase", "feature", "task", "bug", "regression", "revisit", "question", "idea", "experiment", "note"];
  if (issuesOnly && !planState.kinds.size) ["bug", "regression", "revisit"].forEach((k) => planState.kinds.add(k));
  const match = (i) => (planState.closed || !closedStatus(i.status)) && (!planState.kinds.size || planState.kinds.has(i.kind)) &&
    (!planState.text || (i.id + " " + i.title).toLowerCase().includes(planState.text.toLowerCase()));
  const visible = (i) => match(i) || (byParent.get(i.id) || []).some(visible);
  const render = (list) => "<ul>" + list.filter(visible).map((i) => {
    const kids = byParent.get(i.id) || [];
    const [done, total] = count(i.id);
    const col = planState.collapsed.has(i.id);
    const tog = kids.length ? `<span class="tog" data-id="${esc(i.id)}">${col ? "▸" : "▾"}</span>` : "<span></span>";
    const prog = total ? progressBar(done, total) : `<span class="faint small">${esc(ago(touched(i)))}</span>`;
    return `<li><div class="node ${closedStatus(i.status) ? "closed" : ""}">${tog}${dot(touched(i))}<a class="mono small" href="#/i/${esc(i.id)}">${esc(i.id)}</a>${kind(i.kind)}${status(i.status)}` +
      `<a class="title plain" href="#/i/${esc(i.id)}">${prio(i.priority)}${esc(i.title)}</a>${prog}</div>${kids.length && !col ? render(kids) : ""}</li>`;
  }).join("") + "</ul>";
  const toolbar = `<div class="toolbar"><input id="pf" type="text" placeholder="Filter by text" value="${esc(planState.text)}">` +
    `<label class="small"><input id="pc" type="checkbox" ${planState.closed ? "checked" : ""}> show closed</label>` +
    kinds.map((k) => `<span class="chip ${planState.kinds.has(k) ? "on" : ""}" data-k="${k}">${k}</span>`).join("") +
    `<button id="pe">Expand all</button><button id="pk">Collapse all</button></div>`;
  const total = items.length, done = items.filter((i) => closedStatus(i.status)).length;
  main().innerHTML = `<h1>Plan</h1><p class="muted">${done} of ${total} items closed. Click any item for its full story.</p>${toolbar}${legend()}<div class="tree">${render(byParent.get("") || [])}</div>`;
  $("#pf").oninput = (e) => { planState.text = e.target.value; renderPlan(key, view).then(() => { const f = $("#pf"); f.focus(); f.setSelectionRange(f.value.length, f.value.length); }); };
  $("#pc").onchange = (e) => { planState.closed = e.target.checked; renderPlan(key, view); };
  document.querySelectorAll(".chip[data-k]").forEach((c) => (c.onclick = () => { const k = c.dataset.k; planState.kinds.has(k) ? planState.kinds.delete(k) : planState.kinds.add(k); renderPlan(key, view); }));
  document.querySelectorAll(".tog").forEach((t) => (t.onclick = () => { const id = t.dataset.id; planState.collapsed.has(id) ? planState.collapsed.delete(id) : planState.collapsed.add(id); renderPlan(key, view); }));
  $("#pe").onclick = () => { planState.collapsed.clear(); renderPlan(key, view); };
  $("#pk").onclick = () => { items.forEach((i) => (byParent.get(i.id) || []).length && planState.collapsed.add(i.id)); renderPlan(key, view); };
}

function renderList(title, intro, list, fmt) {
  main().innerHTML = `<h1>${title}</h1><p class="muted">${intro}</p>${legend()}${rows(list, fmt, 500)}`;
}
async function renderQuestions(key) {
  const qs = state.items.filter((i) => i.kind === "question");
  const open = qs.filter((i) => !closedStatus(i.status)), closed = qs.filter((i) => closedStatus(i.status));
  main().innerHTML = `<h1>Questions</h1><p class="muted">Questions agents could not answer themselves. Open one to see why it was asked and to answer it.</p>${legend()}` +
    `<h2>Open <span class="faint">${open.length}</span></h2>${rows(open, (i) => row(i, `asked ${esc(ago(i.created))} by ${esc(i.created_by || "?")}`), 500)}` +
    `<h2>Answered <span class="faint">${closed.length}</span></h2>${rows(closed, (i) => row(i, esc(ago(i.updated))), 500)}`;
}
async function renderDecisions(key) {
  const ds = state.items.filter((i) => i.kind === "decision").sort((a, b) => new Date(b.created) - new Date(a.created));
  renderList("Decisions", "Every decision, rejected option and change of mind, newest first.", ds, (i) => row(i, `${status(i.status)} · ${esc((i.areas || []).join(", "))} · ${esc(ago(i.created))}`));
}
async function renderExperiments(key) {
  const xs = state.items.filter((i) => i.kind === "experiment");
  const group = (t, f, ex) => `<h2>${t}</h2>` + rows(xs.filter(f), ex, 500);
  main().innerHTML = `<h1>Experiments</h1><p class="muted">What was tried, what happened, and when it is worth trying again.</p>${legend()}` +
    group("Running", (i) => ["open", "active"].includes(i.status), (i) => row(i, esc(i.hypothesis ? "hypothesis: " + i.hypothesis : ""))) +
    group("Failed or inconclusive", (i) => ["failed", "inconclusive"].includes(i.status), (i) => row(i, `${status(i.status)}${i.retry_when ? " · retry when: " + esc(i.retry_when) : ""}${i.retry_after ? " · after " + esc(i.retry_after) : ""}`)) +
    group("Succeeded", (i) => i.status === "done", (i) => row(i, esc(ago(i.updated))));
}
async function renderResources(key) {
  const rs = state.items.filter((i) => i.kind === "resource" && i.status !== "dropped").sort((a, b) => (a.resource_type || "").localeCompare(b.resource_type || ""));
  main().innerHTML = `<h1>Resources</h1><p class="muted">Datasets, tables, models, endpoints, services … this project works with.</p>` +
    (rs.length ? `<table class="t"><tr><th>Type</th><th>Name</th><th>Where</th><th>Owner</th><th>Notes</th></tr>` +
      rs.map((r) => `<tr><td>${kind(r.resource_type || "other")}</td><td><a href="#/i/${esc(r.id)}">${esc(r.title)}</a> <span class="faint mono">${esc(r.id)}</span></td><td class="mono">${esc(r.location || "")}</td><td>${esc(r.owner || "")}</td><td class="small">${inline((r.body || "").split("\n")[0])}</td></tr>`).join("") + `</table>`
      : '<div class="empty">No resources recorded yet. <code>t9 add resource "…" --type table --at …</code></div>');
}
async function renderTimeline(key) {
  const ev = await api(`p/${key}/timeline?days=90`);
  let day = "", h = `<h1>Timeline</h1><p class="muted">What happened when, newest first (90 days).</p>${legend()}`;
  for (const e of ev) {
    const d = new Date(e.at).toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });
    if (d !== day) { h += `<div class="day">${esc(d)}</div>`; day = d; }
    const id = e.source === "entry" ? e.item : e.id;
    h += `<div class="entry">${dot(e.at)}<span class="when">${esc(when(e.at, false))}</span><span class="kind e-${esc(e.kind.replace(/^new /, ""))}">${esc(e.kind)}${id ? ` <a class="mono" href="#/i/${esc(id)}">${esc(id)}</a>` : ""}</span><span class="text clip">${inline(e.title)}</span></div>`;
  }
  main().innerHTML = ev.length ? h : h + '<div class="empty">Nothing in this period.</div>';
}
async function renderJournal(key) {
  const es = await api(`p/${key}/journal?limit=400`);
  main().innerHTML = `<h1>Journal</h1><p class="muted">Every entry, newest first.</p>${legend()}<div class="entries">${es.map((e) => entryRow(e)).join("")}</div>`;
}
async function renderAbout(key) {
  const a = await api(`p/${key}/about`);
  const secs = a.sections.filter((s) => s.text.trim());
  main().innerHTML = `<h1>About ${esc(a.name)}</h1><p class="muted">${a.reviewed ? "Last reviewed " + esc(when(a.reviewed)) : "Never reviewed"}. Edit with <code>t9 about edit</code> or ask an agent to draft it from the README.</p>` +
    (secs.length ? secs.map((s) => `<h2>${esc(s.title)}</h2><div class="body">${md(s.text)}</div>`).join("") : '<div class="empty">Nothing written yet.</div>');
}
async function renderSearch(key, view) {
  const q = new URLSearchParams(view.split("?")[1] || "").get("q") || "";
  const [hits, hist] = await Promise.all([api(`search?q=${encodeURIComponent(q)}&p=${encodeURIComponent(key)}`), api(`history?q=${encodeURIComponent(q)}&p=${encodeURIComponent(key)}`)]);
  let h = `<h1>“${esc(q)}”</h1>`;
  h += `<h2>History <span class="faint small">everything recorded about it, oldest first</span></h2>`;
  h += hist.length ? `<div class="entries">${hist.map((e) => { const id = e.source === "entry" ? e.item : e.id; return `<div class="entry">${dot(e.at)}<span class="when">${esc(when(e.at))}</span><span class="kind e-${esc(e.kind)}">${esc(e.kind)}${id ? ` <a class="mono" href="#/i/${esc(id)}">${esc(id)}</a>` : ""}</span><span class="text clip">${e.status ? status(e.status) + " " : ""}${inline(e.title)}${e.detail ? ` <span class="faint">— ${inline(e.detail)}</span>` : ""}${e.flag ? ` <span class="flag">← ${esc(e.flag)}</span>` : ""}</span></div>`; }).join("")}</div>` : '<div class="empty">Nothing on this topic.</div>';
  h += `<h2>All matches</h2>` + (hits.length ? `<div class="rows">${hits.map((x) => `<a class="row" href="#/i/${esc(x.kind === "item" ? x.id : "")}"><span></span><span class="id">${esc(x.kind === "item" ? x.id : "entry")}</span><span class="badge">${esc(x.kind)}</span><span class="title">${esc(x.title)} <span class="faint">${esc(x.snippet.replace(/[«»]/g, ""))}</span></span><span></span></a>`).join("")}</div>` : '<div class="empty">No matches.</div>');
  main().innerHTML = h;
}

async function renderFile(key, encoded) {
  const path = decodeURIComponent(encoded);
  const f = await api(`p/${key}/file?path=${encodeURIComponent(path)}`);
  main().innerHTML = `<p class="chain"><a href="#" id="back">← back</a></p><h1 class="mono">${esc(f.path)}</h1>` +
    `<p class="muted small">${esc(f.full)}${f.modified ? " · modified " + esc(when(f.modified)) : ""} · read-only</p>` +
    (f.markdown ? `<div class="body">${md(f.text)}</div>` : `<pre>${esc(f.text)}</pre>`);
  $("#back").onclick = (e) => { e.preventDefault(); history.back(); };
}

// One item, with everything around it.
function renderItem(p) {
  const i = p.item;
  const chain = p.chain.map((c) => `<a href="#/i/${esc(c.id)}">${esc(c.id)} ${esc(c.title)}</a>`).join(" › ");
  const fields = [
    ["Status", `${status(i.status)}${i.priority !== null && i.priority !== undefined ? " · " + prio(i.priority) : ""}`],
    ["Created", `${esc(when(i.created))} by ${esc(i.created_by || "?")}${p.asked.session ? ` in session <span class="mono">${esc(p.asked.session.id.slice(-6))}</span>` : ""}`],
    ["Last change", `${dot(i.touched)} ${esc(ago(i.touched))} (${esc(when(i.touched))})`],
  ];
  if (i.areas?.length) fields.push(["Areas", esc(i.areas.join(", "))]);
  if (i.tags?.length) fields.push(["Tags", esc(i.tags.join(", "))]);
  if (i.hypothesis) fields.push(["Hypothesis", inline(i.hypothesis)]);
  if (i.retry_when || i.retry_after) fields.push(["Retry", esc([i.retry_when && "when " + i.retry_when, i.retry_after && "after " + i.retry_after].filter(Boolean).join(" · "))]);
  if (i.resource_type) fields.push(["Type", esc(i.resource_type)]);
  if (i.location) fields.push(["Where", `<span class="mono">${esc(i.location)}</span>`]);
  if (i.owner) fields.push(["Owner", esc(i.owner)]);
  fields.push(["File", `<span class="mono small faint">${esc(i.path)}</span>`]);
  const isOpen = !closedStatus(i.status);
  let mainCol = `<div class="chain">${chain || `<a href="#/p/${esc(i.project)}/plan">${esc(i.project)} plan</a>`}</div>` +
    `<div class="item-head">${kind(i.kind)}<span class="mono faint">${esc(i.id)}</span>${status(i.status)}</div><h1>${esc(i.title)}</h1>` +
    `<dl class="meta">${fields.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("")}</dl>` +
    (i.body.trim() ? `<div class="body">${md(i.body)}</div>` : "");

  if (i.kind === "question" && isOpen) {
    mainCol += card("Answer", null, `<p class="muted small">Your answer is recorded on ${esc(i.id)}, which is then closed. Agents see it in their next brief.</p>` +
      `<textarea id="ans" placeholder="Your answer…"></textarea><p><label><input id="asdec" type="checkbox" checked> also record it as a decision (so agents follow it from now on)</label></p><button class="primary" id="ansb">Answer</button> <span id="ansmsg" class="small"></span>`, "ask");
  }
  if (p.children.length) {
    mainCol += `<h2>Inside ${esc(i.id)} <span class="faint">${p.children.filter((c) => closedStatus(c.status)).length}/${p.children.length} closed</span></h2>` +
      rows(p.children, (c) => row(c, status(c.status)), 200);
  }
  mainCol += `<h2>History of ${esc(i.id)}</h2>` + (p.entries.length ? `<div class="entries">${p.entries.map((e) => entryRow(e, false)).join("")}</div>` : '<div class="empty">No journal entries yet.</div>');
  mainCol += `<h2>Add a note</h2><textarea id="note" placeholder="A note on ${esc(i.id)}…"></textarea><p><button id="noteb">Add note</button> <span id="notemsg" class="small"></span></p>`;

  let side = "";
  const ctx = p.asked;
  const created = `<p class="small">${esc(when(i.created))} · ${esc(ago(i.created))}<br>by <strong>${esc(i.created_by || "?")}</strong>${ctx.session ? ` in session <span class="mono">${esc(ctx.session.id.slice(-6))}</span> (${esc(ctx.session.agent)}, started ${esc(when(ctx.session.started))})` : ""}</p>`;
  const working = ctx.working_on ? `<p class="small">The agent was working on:</p><div class="rows">${row(ctx.working_on)}</div>` : "";
  const bursts = ctx.status_changes ? `<p class="small muted">Also ${ctx.status_changes} status changes in the 30 minutes before.</p>` : "";
  const before = ctx.before.length || bursts ? `<h3>Just before</h3><div class="entries">${ctx.before.map((e) => entryCompact(e)).join("")}</div>${bursts}` : "";
  side += card(i.kind === "question" ? "Why it was asked" : "Where it came from", null, created + working + before);
  const s = p.since;
  side += card("What happened since", s.entries, (s.entries ? `<p class="small">${s.entries} journal entries since; last ${esc(ago(s.last?.at))}.</p>` : `<p class="small">Nothing recorded since.</p>`) +
    (s.busiest.length ? `<h3>Most active since</h3><div class="rows">${s.busiest.map((b) => row(b.item, `${b.entries} entries`)).join("")}</div>` : "") +
    (s.recent.length ? `<h3>Latest</h3><div class="entries">${s.recent.map((e) => entryCompact(e)).join("")}</div>` : ""));
  if (p.applicable.length) side += card("From history", p.applicable.length, `<div class="rows">${p.applicable.map((a) => row(a, esc(a.reasons.join("; ")))).join("")}</div>`);
  const links = [...p.links.map((l) => ({ rel: l.rel, item: l.item, to: l.to })), ...p.backlinks.map((b) => ({ rel: "← " + b.rel, item: b.item }))];
  if (links.length) side += card("Links", links.length, `<div class="rows">${links.map((l) => l.item ? row(l.item, esc(l.rel)) : `<div class="row"><span></span><span class="id">${esc(l.to)}</span><span></span><span class="title faint">${esc(l.rel)} (missing)</span><span></span></div>`).join("")}</div>`);
  const statuses = i.kind === "decision" ? ["accepted", "proposed", "rejected", "superseded"] : i.kind === "experiment" ? ["open", "active", "done", "failed", "inconclusive", "dropped"] : ["open", "active", "blocked", "done", "dropped"];
  side += card("Change status", null, `<select id="st">${statuses.map((x) => `<option ${x === i.status ? "selected" : ""}>${x}</option>`).join("")}</select> <input id="stn" type="text" placeholder="note (optional)" style="width:150px"> <button id="stb">Save</button> <span id="stmsg" class="small"></span>`);

  main().innerHTML = `<div class="two"><div>${mainCol}</div><aside class="side">${side}</aside></div>`;
  const act = (btn, msg, fn) => {
    $(btn).onclick = async () => {
      $(btn).disabled = true;
      try { await fn(); $(msg).textContent = "saved"; state.items = null; await loadItems(i.project); route(); }
      catch (e) { $(msg).innerHTML = `<span class="err">${esc(e.message)}</span>`; $(btn).disabled = false; }
    };
  };
  if ($("#ansb")) act("#ansb", "#ansmsg", () => post(`item/${i.id}/answer`, { answer: $("#ans").value, decide: $("#asdec").checked }));
  act("#noteb", "#notemsg", () => post(`item/${i.id}/note`, { text: $("#note").value }));
  act("#stb", "#stmsg", () => post(`item/${i.id}/status`, { status: $("#st").value, note: $("#stn").value || null }));
}

// ---------------------------------------------------------------- live refresh, search, keys

function busy() {
  const a = document.activeElement;
  return [...document.querySelectorAll("textarea, input[type=text]")].some((t) => t.value.trim()) || (a && ["TEXTAREA", "INPUT", "SELECT"].includes(a.tagName));
}
async function poll() {
  if (!state.key) { $("#live").textContent = "live"; return; }
  try {
    const c = await api(`p/${state.key}/changes`);
    const changed = state.latest && c.latest !== state.latest;
    state.latest = c.latest;
    if (changed) {
      if (busy()) { $("#live").textContent = "new changes — they'll show when you're done"; $("#live").classList.add("stale"); return; }
      const y = window.scrollY;
      await loadItems(state.key); await route(); window.scrollTo(0, y);
    }
    $("#live").classList.remove("stale");
    $("#live").textContent = `live · last change ${ago(c.latest)}`;
  } catch { $("#live").textContent = "t9 web stopped"; $("#live").classList.add("stale"); }
}
$("#search").onsubmit = (e) => {
  e.preventDefault();
  const q = $("#q").value.trim();
  if (!q) return;
  if (/^[A-Za-z][A-Za-z0-9]{1,9}-\d+$/.test(q)) location.hash = "#/i/" + q.toUpperCase();
  else if (state.key) location.hash = `#/p/${state.key}/search?q=${encodeURIComponent(q)}`;
};
document.addEventListener("keydown", (e) => {
  if (e.key === "/" && !["INPUT", "TEXTAREA"].includes(document.activeElement.tagName)) { e.preventDefault(); $("#q").focus(); }
});
window.addEventListener("hashchange", route);
route();
setInterval(poll, 3000);
poll();
