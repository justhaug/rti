//! Single-page UI served at `/`. Vanilla HTML/CSS/JS, no build step.

pub const INDEX_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RTI</title>
<style>
:root{--bg:#0f1217;--panel:#171b23;--panel2:#1e2430;--line:#2a3140;--fg:#d9dee7;--dim:#8b94a5;--acc:#5aa9ff;--ok:#4fd18b;--warn:#f0b35a;--err:#ff6b6b;--ora:#ff9a3c}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:13px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
header{display:flex;flex-wrap:wrap;gap:8px;align-items:center;padding:8px 12px;border-bottom:1px solid var(--line);background:var(--panel)}
header h1{font-size:15px;margin:0 12px 0 0;color:var(--acc)}
.chip{background:var(--panel2);border:1px solid var(--line);border-radius:12px;padding:2px 9px;color:var(--dim);white-space:nowrap}
.chip b{color:var(--fg)}
.chip.on b{color:var(--ok)}
button{background:var(--panel2);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:4px 10px;cursor:pointer;font:inherit}
button:hover{border-color:var(--acc)}
button.primary{border-color:var(--acc);color:var(--acc)}
main{display:grid;grid-template-columns:minmax(320px,38%) 1fr;height:calc(100vh - 46px)}
@media (max-width:900px){main{grid-template-columns:1fr;height:auto}}
#chat{display:flex;flex-direction:column;border-right:1px solid var(--line);min-height:300px}
#msgs{flex:1;overflow:auto;padding:10px}
.msg{margin:0 0 10px;padding:8px 10px;border-radius:8px;white-space:pre-wrap;word-break:break-word}
.msg.user{background:#1f2a3d;margin-left:20px}
.msg.assistant{background:var(--panel2);margin-right:20px}
.msg.tool{background:transparent;color:var(--dim);font-size:12px;padding:2px 10px;margin:0 0 4px}
.badge{display:inline-block;background:#2a3547;color:var(--acc);border-radius:4px;padding:0 6px;margin:2px 4px 2px 0;font-size:11px}
#chatform{display:flex;gap:6px;padding:8px;border-top:1px solid var(--line)}
#chatform textarea{flex:1;background:var(--panel2);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:6px;font:inherit;resize:vertical;min-height:38px}
#right{display:flex;flex-direction:column;min-height:0}
nav{display:flex;flex-wrap:wrap;gap:2px;padding:6px 8px;border-bottom:1px solid var(--line);background:var(--panel)}
nav button.active{border-color:var(--acc);color:var(--acc)}
#panel{flex:1;overflow:auto;padding:10px}
table{border-collapse:collapse;width:100%}
th,td{text-align:left;padding:4px 6px;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--dim);font-weight:normal;position:sticky;top:0;background:var(--bg)}
tr.row:hover{background:var(--panel2);cursor:pointer}
.card{background:var(--panel2);border:1px solid var(--line);border-radius:8px;padding:8px 10px;margin:0 0 8px}
.card h4{margin:0 0 4px;font-size:13px}
.dim{color:var(--dim)}
.ok{color:var(--ok)}.err{color:var(--err)}.warn{color:var(--warn)}
pre{white-space:pre-wrap;word-break:break-word;background:var(--panel2);padding:8px;border-radius:6px;margin:0}
textarea.sql{width:100%;min-height:80px;background:var(--panel2);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:6px;font:inherit}
input,select{background:var(--panel2);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:4px 6px;font:inherit}
.log div{padding:2px 0;border-bottom:1px dotted var(--line)}
.log .k{display:inline-block;min-width:80px;color:var(--dim)}
canvas{background:#0b0e13;border:1px solid var(--line);border-radius:6px;max-width:100%}
#modal{position:fixed;inset:0;background:rgba(0,0,0,.6);display:none;align-items:center;justify-content:center;padding:20px}
#modal .box{background:var(--panel);border:1px solid var(--line);border-radius:10px;max-width:900px;width:100%;max-height:90vh;overflow:auto;padding:14px}
.legend span{display:inline-block;width:12px;height:3px;margin:0 4px 2px 0;vertical-align:middle}
.card.media.big { border: 1px solid #4fd18b; }
.badge { display:inline-block; padding:2px 6px; border-radius:4px; background:#2a3140; color:#cfd6e4; font-size:11px; }
.badge.ok { background:#1f5a3a; color:#bff3d2; }
</style>
</head>
<body>
<header>
  <h1>RTI</h1>
  <span class="chip" id="c-loop">loop <b>–</b></span>
  <span class="chip">cycles <b id="c-cycles">–</b></span>
  <span class="chip">experiments <b id="c-exp">–</b></span>
  <span class="chip">LLM <b id="c-usd">–</b></span>
  <span class="chip">oracle <b id="c-oracle">–</b></span>
  <span class="chip">sim <b id="c-mticks">–</b> Mticks/s</span>
  <span class="chip">brain <b id="c-brain">–</b></span>
  <span style="flex:1"></span>
  <button class="primary" onclick="loopStart()">Start loop</button>
  <button onclick="loopStop()">Stop</button>
  <button onclick="cycleOnce()">Cycle once</button>
  <button onclick="runTasks()">Run tasks</button>
</header>
<main>
  <section id="chat">
    <div id="msgs"><div class="msg assistant dim">Operator ready. Ask about the archive, or say things like "optimize hairpin", "start a 20-cycle campaign", "why is the sim diverging?".</div></div>
    <form id="chatform" onsubmit="return sendChat(event)">
      <textarea id="chatin" placeholder="Talk to the operator… (Enter to send, Shift+Enter for newline)"></textarea>
      <button class="primary" type="submit">Send</button>
      <button type="button" onclick="clearChat()">Clear</button>
    </form>
  </section>
  <section id="right">
    <nav id="tabs"></nav>
    <div id="discovery"></div><div id="panel"></div>
  </section>
</main>
<div id="modal" onclick="if(event.target===this)this.style.display='none'"><div class="box" id="modalbox"></div></div>
<script>
const $ = s => document.querySelector(s);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const fmt = (v, d=2) => typeof v === 'number' ? v.toFixed(d) : (v ?? '');
async function api(path, body) {
  const r = await fetch(path, body === undefined ? {} : {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify(body)});
  const j = await r.json();
  if (!r.ok || j.error) throw new Error(j.error || r.statusText);
  return j;
}
function toast(msg, cls) {
  const m = $('#msgs'); const d = document.createElement('div');
  d.className = 'msg tool ' + (cls||''); d.textContent = msg; m.appendChild(d); m.scrollTop = m.scrollHeight;
}

// ---------- header / status ----------
async function refreshStatus() {
  try {
    const s = await api('/api/status');
    const loop = s.loop || {};
    const c = $('#c-loop'); c.className = 'chip' + (loop.running ? ' on' : '');
    c.innerHTML = 'loop <b>' + (loop.running ? 'running' + (loop.cycles_target ? ' ' + loop.cycles_done + '/' + loop.cycles_target : '') : 'stopped') + '</b>';
    $('#c-cycles').textContent = s.cycles;
    $('#c-exp').textContent = s.experiments;
    $('#c-usd').textContent = '$' + fmt(s.llm.spend_total_usd, 3) + (s.llm.configured ? '' : ' (no key)');
    $('#c-oracle').textContent = s.oracle;
    $('#c-mticks').textContent = fmt(s.mticks_per_s, 0);
    $('#c-brain').textContent = s.brain + (s.llm.configured ? '' : '→scripted');
  } catch (e) { console.warn(e); }
}
async function loopStart() { const n = prompt('Cycles (0 = until stopped)', '0'); if (n === null) return; try { await api('/api/loop/start', {cycles: +n}); toast('loop started'); } catch (e) { toast(e.message, 'err'); } refreshStatus(); }
async function loopStop() { try { await api('/api/loop/stop'); toast('stop requested'); } catch (e) { toast(e.message, 'err'); } }
async function cycleOnce() { toast('running one cycle…'); try { const r = await api('/api/loop/once'); toast('cycle ' + r.report.cycle + ': ' + r.report.summary); } catch (e) { toast(e.message, 'err'); } refreshStatus(); render(); }
async function runTasks() { try { const r = await api('/api/tasks/run'); toast(r.result); } catch (e) { toast(e.message, 'err'); } render(); }

// ---------- chat ----------
function renderChat(history) {
  const m = $('#msgs'); m.innerHTML = '';
  for (const h of history) {
    if (h.role === 'tool') { const d = document.createElement('div'); d.className = 'msg tool'; d.textContent = '↳ ' + h.name + ': ' + String(h.content).slice(0, 200); m.appendChild(d); continue; }
    const d = document.createElement('div'); d.className = 'msg ' + h.role;
    let html = esc(h.content);
    if (h.tool_calls && h.tool_calls.length) html += '<div>' + h.tool_calls.map(t => '<span class="badge">' + esc(t.name) + '</span>').join('') + '</div>';
    d.innerHTML = html; m.appendChild(d);
  }
  m.scrollTop = m.scrollHeight;
}
async function loadChat() { try { const c = await api('/api/chat'); if (c.history.length) renderChat(c.history); } catch (e) {} }
async function sendChat(ev) {
  ev.preventDefault();
  const ta = $('#chatin'); const text = ta.value.trim(); if (!text) return false;
  ta.value = '';
  const m = $('#msgs'); const d = document.createElement('div'); d.className = 'msg user'; d.textContent = text; m.appendChild(d);
  const th = document.createElement('div'); th.className = 'msg assistant dim'; th.textContent = 'thinking…'; m.appendChild(th); m.scrollTop = m.scrollHeight;
  try { await api('/api/chat', {message: text}); const c = await api('/api/chat'); renderChat(c.history); }
  catch (e) { th.className = 'msg assistant err'; th.textContent = e.message; }
  refreshStatus();
  return false;
}
async function clearChat() { await api('/api/chat/clear'); $('#msgs').innerHTML = ''; }
$('#chatin').addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); $('#chatform').requestSubmit(); } });

// ---------- tabs ----------
const tabs = {
  log: renderLog, experiments: renderExperiments, findings: renderFindings, tracks: renderTracks,
  tasks: renderTasks, ledger: renderLedger, methods: renderMethods, sql: renderSql, context: renderContext, media: renderMedia,
};
let current = 'log';
for (const k of Object.keys(tabs)) { const b = document.createElement('button'); b.textContent = k; b.onclick = () => { current = k; render(); }; b.dataset.tab = k; $('#tabs').appendChild(b); }
function render() { document.querySelectorAll('#tabs button').forEach(b => b.classList.toggle('active', b.dataset.tab === current)); tabs[current]().catch(e => { $('#panel').innerHTML = '<div class="err">' + esc(e.message) + '</div>'; }); }

let lastSeq = 0; let logLines = [];
async function renderLog() {
  const r = await api('/api/events?since=0');
  logLines = r.events; lastSeq = r.seq;
  $('#panel').innerHTML = '<div class="log">' + logLines.slice().reverse().map(e => '<div><span class="dim">' + esc(e.ts.slice(11, 19)) + '</span> <span class="k ' + (e.kind === 'error' ? 'err' : e.kind === 'cycle' ? 'ok' : '') + '">' + esc(e.kind) + '</span> ' + esc(e.text) + '</div>').join('') + '</div>';
}
async function renderExperiments() {
  const rows = await api('/api/experiments?n=100');
  $('#panel').innerHTML = '<table><tr><th>id</th><th>cycle</th><th>kind</th><th>method</th><th>track</th><th>status</th><th>V</th><th>summary</th></tr>' +
    rows.map(e => '<tr class="row" onclick="showExperiment(\'' + e.id + '\')"><td>' + esc(e.id) + '</td><td>' + esc(e.cycle ?? '') + '</td><td>' + esc(e.kind) + '</td><td>' + esc(e.method ?? '') + '</td><td>' + esc(e.track ?? '') + '</td><td class="' + (e.status === 'done' ? 'ok' : e.status === 'failed' ? 'err' : 'warn') + '">' + esc(e.status) + '</td><td>' + fmt(e.value_total) + '</td><td>' + esc((e.summary || '').slice(0, 220)) + '</td></tr>').join('') + '</table>';
}
async function showExperiment(id) {
  const r = await api('/api/experiment/' + id);
  const e = r.experiment;
  $('#modalbox').innerHTML = '<h3>' + esc(e.kind) + ' ' + esc(e.id) + ' <span class="dim">cycle ' + esc(e.cycle ?? '-') + ' · ' + esc(e.status) + '</span></h3>' +
    '<p>' + esc(e.summary || '') + '</p>' +
    '<p><b>spec</b></p><pre>' + esc(JSON.stringify(e.spec, null, 1)) + '</pre>' +
    '<p><b>value</b> ' + esc(JSON.stringify(e.value)) + ' total ' + fmt(e.value_total) + ' · <b>cost</b> ' + esc(JSON.stringify(e.cost)) + '</p>' +
    (r.trajectories.length ? '<p><b>trajectories</b></p>' + r.trajectories.map(t => '<div class="card">' + esc(t.hash) + ' · ' + esc(t.world) + ' · ' + (t.finished ? '<span class="ok">' + t.time_ms + ' ms</span>' : '<span class="warn">DNF ' + fmt(t.progress, 0) + ' m</span>') + ' <button onclick="showTrajectory(\'' + t.hash + '\',\'' + esc(t.track_name) + '\')">draw</button></div>').join('') : '') +
    '<p><b>result</b></p><pre>' + esc(JSON.stringify(e.result, null, 1).slice(0, 20000)) + '</pre>';
  $('#modal').style.display = 'flex';
}
async function renderFindings() {
  const rows = await api('/api/findings?n=100');
  $('#panel').innerHTML = rows.length ? rows.map(f => '<div class="card"><h4>' + esc(f.title) + ' <span class="dim">[' + esc(f.kind) + '] conf ' + fmt(f.confidence) + ' · cycle ' + esc(f.cycle ?? '-') + '</span></h4><div style="white-space:pre-wrap">' + esc(f.body) + '</div>' + (f.tags.length ? '<div class="dim">' + f.tags.map(esc).join(', ') + '</div>' : '') + '</div>').join('') : '<div class="dim">no findings yet</div>';
}
async function renderTracks() {
  const rows = await api('/api/tracks');
  const maps = await api('/api/maps?n=100');
  const mapByTrack = {}; for (const m of maps) mapByTrack[m.track_name] = m;
  $('#panel').innerHTML = '<div class="card"><b>Import a real map</b> <span class="dim">Trackmania Exchange id / URL, or a local .Map.Gbx path</span><br>' +
    '<input id="imp-src" placeholder="e.g. 356566 or https://trackmania.exchange/maps/356566" style="width:60%"> <input id="imp-name" placeholder="track name (optional)" style="width:22%"> <button onclick="importMap()">Import</button> <span id="imp-status" class="dim"></span>' +
    '<div style="margin-top:6px"><input id="tmx-q" placeholder="search TMX by name" style="width:40%"> <input id="tmx-tag" placeholder="tag ids e.g. 3 (Tech), 25 (Mini)" style="width:25%"> <button onclick="tmxSearch()">Search</button></div><div id="tmx-results"></div></div>' +
    '<table><tr><th>track</th><th>length</th><th>nodes</th><th>best sim</th><th>verified</th><th>source</th></tr>' +
    rows.map(t => { const m = mapByTrack[t.name]; const src = m ? ('TMX ' + (m.tmx_id ?? '-') + ' · author ' + (m.author_ms ?? '-') + ' ms' + (m.wr_ms ? ' · WR ' + m.wr_ms + ' ms' : '') + (m.report ? ' · ' + m.report.blocks_chained + '/' + m.report.blocks_recognized + ' chained' + (m.report.finish_found ? '' : ' (no finish)') : '')) : ''; return '<tr class="row" onclick="showTrack(\'' + esc(t.name) + '\')"><td>' + esc(t.name) + '</td><td>' + fmt(t.length_m, 0) + ' m</td><td>' + t.n_nodes + '</td><td>' + (t.best_sim ? t.best_sim.time_ms + ' ms' : '-') + '</td><td>' + (t.best_oracle ? t.best_oracle.time_ms + ' ms' : '-') + '</td><td class="dim">' + esc(src) + '</td></tr>'; }).join('') + '</table><div id="trackview"></div>';
}
async function importMap(src) {
  const source = src || $('#imp-src').value.trim(); if (!source) return;
  $('#imp-status').textContent = 'importing ' + source + '…';
  try {
    const r = await api('/api/import', {source, name: $('#imp-name') ? $('#imp-name').value.trim() : ''});
    $('#imp-status').textContent = r.summary;
    await renderTracks(); $('#imp-status').textContent = r.summary;
  } catch (e) { $('#imp-status').textContent = 'import failed: ' + e.message; }
}
async function tmxSearch() {
  const q = $('#tmx-q').value.trim(), tag = $('#tmx-tag').value.trim();
  $('#tmx-results').innerHTML = '<span class="dim">searching…</span>';
  try {
    const r = await api('/api/tmx/search?count=15' + (q ? '&name=' + encodeURIComponent(q) : '') + (tag ? '&tag=' + encodeURIComponent(tag) : ''));
    $('#tmx-results').innerHTML = '<table><tr><th>id</th><th>name</th><th>authors</th><th>author time</th><th>WR</th><th>awards</th><th>tags</th><th></th></tr>' + r.results.map(m => '<tr><td>' + m.map_id + '</td><td>' + esc(m.name) + '</td><td>' + esc(m.authors.join(', ')) + '</td><td>' + m.author_ms + ' ms</td><td>' + (m.wr_ms ? m.wr_ms + ' ms' : '-') + '</td><td>' + m.awards + '</td><td class="dim">' + esc(m.tags.join(', ')) + '</td><td><button onclick="importMap(\'' + m.map_id + '\')">import</button></td></tr>').join('') + '</table>';
  } catch (e) { $('#tmx-results').innerHTML = '<span class="dim">search failed: ' + esc(e.message) + '</span>'; }
}
function drawTrack(canvas, track, trajs) {
  const ctx = canvas.getContext('2d');
  const xs = track.nodes.map(n => n.x), ys = track.nodes.map(n => n.y);
  for (const t of trajs) for (const s of t.states) { xs.push(s.x); ys.push(s.y); }
  const minx = Math.min(...xs) - 20, maxx = Math.max(...xs) + 20, miny = Math.min(...ys) - 20, maxy = Math.max(...ys) + 20;
  const W = canvas.width, H = canvas.height;
  const sc = Math.min(W / (maxx - minx), H / (maxy - miny));
  const X = x => (x - minx) * sc + (W - (maxx - minx) * sc) / 2, Y = y => H - ((y - miny) * sc + (H - (maxy - miny) * sc) / 2);
  ctx.clearRect(0, 0, W, H);
  const surf = {asphalt:'#3a4250', dirt:'#5a4a30', grass:'#2e5a34', ice:'#3d6a80'};
  for (let i = 0; i + 1 < track.nodes.length; i++) {
    const a = track.nodes[i], b = track.nodes[i + 1];
    ctx.strokeStyle = surf[a.surface] || surf.asphalt; ctx.lineWidth = Math.max(2, a.half_width * 2 * sc); ctx.lineCap = 'round';
    ctx.beginPath(); ctx.moveTo(X(a.x), Y(a.y)); ctx.lineTo(X(b.x), Y(b.y)); ctx.stroke();
  }
  ctx.strokeStyle = '#8b94a5'; ctx.lineWidth = 1; ctx.setLineDash([4, 4]); ctx.beginPath();
  track.nodes.forEach((n, i) => i ? ctx.lineTo(X(n.x), Y(n.y)) : ctx.moveTo(X(n.x), Y(n.y))); ctx.stroke(); ctx.setLineDash([]);
  for (const c of track.checkpoints || []) { const n = track.nodes[c]; ctx.fillStyle = '#f0b35a'; ctx.beginPath(); ctx.arc(X(n.x), Y(n.y), 4, 0, 7); ctx.fill(); }
  const s0 = track.nodes[0]; ctx.fillStyle = '#4fd18b'; ctx.beginPath(); ctx.arc(X(s0.x), Y(s0.y), 5, 0, 7); ctx.fill();
  const fin = track.nodes[track.finish ?? track.nodes.length - 1]; ctx.fillStyle = '#ff6b6b'; ctx.beginPath(); ctx.arc(X(fin.x), Y(fin.y), 5, 0, 7); ctx.fill();
  for (const t of trajs) {
    ctx.strokeStyle = t.color; ctx.lineWidth = 2; ctx.beginPath();
    t.states.forEach((s, i) => i ? ctx.lineTo(X(s.x), Y(s.y)) : ctx.moveTo(X(s.x), Y(s.y))); ctx.stroke();
  }
}
async function showTrack(name) {
  const r = await api('/api/track/' + encodeURIComponent(name));
  const trajs = [];
  const pick = async (rows, color, label) => { if (!rows.length) return ''; const t = await api('/api/trajectory/' + rows[0].hash); if (t.states.length) trajs.push({states: t.states, color}); return '<span class="legend"><span style="background:' + color + '"></span>' + label + ' ' + (rows[0].finished ? rows[0].time_ms + ' ms' : 'DNF') + ' <button onclick="showTrajectory(\'' + rows[0].hash + '\',\'' + esc(name) + '\')">inputs</button></span> '; };
  const l1 = await pick(r.best_sim, '#5aa9ff', 'sim best');
  const l2 = await pick(r.best_oracle, '#ff9a3c', 'oracle best');
  $('#trackview').innerHTML = '<h3>' + esc(name) + ' <span class="dim">' + esc(r.track.description) + '</span></h3><div>' + l1 + l2 + '</div><canvas id="tc" width="900" height="520"></canvas>';
  drawTrack($('#tc'), r.track, trajs);
}
async function showTrajectory(hash, name) {
  const t = await api('/api/trajectory/' + hash);
  const tr = await api('/api/track/' + encodeURIComponent(name));
  $('#modalbox').innerHTML = '<h3>trajectory ' + esc(hash.slice(0, 12)) + ' <span class="dim">' + esc(t.method) + ' · ' + esc(t.world) + ' · ' + (t.result.finished ? t.result.time_ms + ' ms' : 'DNF') + '</span></h3><canvas id="mc" width="860" height="420"></canvas><pre>' + esc(JSON.stringify(t.result)) + '</pre><p><b>inputs</b> (run-length)</p><pre>' + esc(t.actions.map(a => (a.ticks * 10) + 'ms steer ' + a.action.steer.toFixed(2) + (a.action.gas ? ' gas' : '') + (a.action.brake ? ' brake' : '')).join('\n').slice(0, 12000)) + '</pre>';
  $('#modal').style.display = 'flex';
  drawTrack($('#mc'), tr.track, t.states.length ? [{states: t.states, color: t.world.startsWith('oracle') ? '#ff9a3c' : '#5aa9ff'}] : []);
}
async function renderTasks() {
  const rows = await api('/api/tasks?n=100');
  $('#panel').innerHTML = '<div class="card"><b>add coding task</b><br><input id="t-title" placeholder="title" style="width:40%"> <input id="t-prio" type="number" value="2" style="width:60px"><br><textarea id="t-desc" class="sql" placeholder="description: what to change, why, how to verify"></textarea><br><button onclick="addTask()">queue</button> <button onclick="runTasks()">run next</button></div>' +
    '<table><tr><th>id</th><th>status</th><th>kind</th><th>prio</th><th>cycle</th><th>title</th><th>result</th></tr>' +
    rows.map(t => '<tr><td>' + esc(t.id) + '</td><td class="' + (t.status === 'merged' || t.status === 'done' ? 'ok' : t.status === 'failed' || t.status === 'rejected' ? 'err' : 'warn') + '">' + esc(t.status) + '</td><td>' + esc(t.kind) + '</td><td>' + t.priority + '</td><td>' + esc(t.cycle ?? '') + '</td><td>' + esc(t.title) + '</td><td class="dim">' + esc(JSON.stringify(t.result_json || '').slice(0, 300)) + '</td></tr>').join('') + '</table>';
}
async function addTask() { try { await api('/api/tasks', {kind: 'coding', title: $('#t-title').value, description: $('#t-desc').value, priority: +$('#t-prio').value}); toast('task queued'); render(); } catch (e) { toast(e.message, 'err'); } }
// ---------- media ----------
function mediaCard(m, big) {
  const st = m.status;
  const badge = m.verified ? '<span class="badge ok">VERIFIED IN ORACLE</span>' : '<span class="badge">sim only</span>';
  const btns = st === 'review' || st === 'approved'
    ? '<button onclick="mediaAct(\'' + m.id + '\',\'publish\')">Publish</button> <button onclick="mediaAct(\'' + m.id + '\',\'upload\')">Upload private</button> <button onclick="mediaAct(\'' + m.id + '\',\'approve\')">Approve</button> <button onclick="mediaAct(\'' + m.id + '\',\'reject\')">Ignore</button>'
    : (m.youtube_id ? '<a href="https://youtube.com/watch?v=' + esc(m.youtube_id) + '" target="_blank">youtube</a>' : '');
  return '<div class="card media' + (big ? ' big' : '') + '"><div class="dim">' + (big ? '🏁 POTENTIAL DISCOVERY · ' : '') + esc(m.track_name) + ' · score ' + fmt(m.score) + ' · ' + esc(st) + ' ' + badge + '</div>' +
    '<h3>' + esc(m.title) + '</h3>' +
    '<video controls preload="metadata" src="/media/' + esc(m.id) + '.mp4" style="max-height:' + (big ? '520' : '360') + 'px;max-width:100%;background:#000"></video>' +
    '<div style="white-space:pre-wrap" class="dim">' + esc(m.explanation || '') + '</div>' +
    '<details><summary class="dim">description</summary><pre>' + esc(m.description) + '</pre></details>' +
    '<div>' + btns + '</div></div>';
}
async function mediaAct(id, action) {
  try { const r = await api('/api/media/' + id + '/' + action, {}); toast(action + ': ' + (r.url || r.status), 'ok'); }
  catch (e) { toast(action + ' failed: ' + e.message, 'err'); }
  render(); refreshDiscovery();
}
async function refreshDiscovery() {
  try {
    const rows = await api('/api/media?n=20');
    const pending = rows.filter(m => m.status === 'review' || m.status === 'approved');
    const box = $('#discovery');
    if (!box) return;
    box.innerHTML = pending.length ? mediaCard(pending[0], true) + (pending.length > 1 ? '<div class="dim">' + (pending.length - 1) + ' more waiting in the media tab</div>' : '') : '';
  } catch (e) { /* ignore */ }
}
async function renderMedia() {
  const rows = await api('/api/media?n=100');
  $('#panel').innerHTML = '<div class="card"><button onclick="mediaScan(true)">Scan verified bests</button> <button onclick="mediaScan(false)">Scan sim bests too</button> <span id="media-status" class="dim"></span></div>' +
    (rows.length ? rows.map(m => mediaCard(m, false)).join('') : '<div class="dim">no videos yet — they appear when a verified best clears the interestingness threshold</div>');
}
async function mediaScan(verifiedOnly) {
  $('#media-status').textContent = 'rendering…';
  try { const r = await api('/api/media/scan', {require_verified: verifiedOnly}); $('#media-status').textContent = 'produced ' + r.produced; render(); refreshDiscovery(); }
  catch (e) { $('#media-status').textContent = 'failed: ' + e.message; }
}
setInterval(refreshDiscovery, 15000);
setTimeout(refreshDiscovery, 500);

async function renderLedger() {
  const l = await api('/api/ledger');
  const row = (k, c) => '<tr><td>' + esc(k) + '</td><td>' + fmt(c.sim_ticks / 1e6, 1) + '</td><td>' + c.oracle_ticks + '</td><td>' + fmt(c.wall_ms / 1000, 1) + '</td><td>' + c.llm_calls + '</td><td>' + fmt(c.llm_usd, 4) + '</td></tr>';
  $('#panel').innerHTML = '<table><tr><th>category</th><th>Mticks</th><th>oracle ticks</th><th>wall s</th><th>LLM calls</th><th>USD</th></tr>' + l.by_category.map(r => row(r.category, r.cost)).join('') + row('TOTAL', l.totals) + '</table>' +
    '<h4>models</h4><table><tr><th>model</th><th>calls</th><th>prompt</th><th>completion</th><th>USD</th></tr>' + l.models.map(m => '<tr><td>' + esc(m.model) + '</td><td>' + m.n + '</td><td>' + m.prompt_tokens + '</td><td>' + m.completion_tokens + '</td><td>' + fmt(m.usd, 4) + '</td></tr>').join('') + '</table>';
}
async function renderMethods() {
  const rows = await api('/api/methods'); const v = await api('/api/verifications');
  $('#panel').innerHTML = '<table><tr><th>method</th><th>track</th><th>runs</th><th>finished</th><th>best ms</th><th>Mticks</th><th>wall s</th><th>USD</th></tr>' + rows.map(m => '<tr><td>' + esc(m.method) + '</td><td>' + esc(m.track) + '</td><td>' + m.n_runs + '</td><td>' + m.finished_runs + '</td><td>' + (m.best_time_ms ?? '-') + '</td><td>' + fmt(m.sim_ticks / 1e6, 1) + '</td><td>' + fmt(m.wall_ms / 1000, 1) + '</td><td>' + fmt(m.llm_usd, 4) + '</td></tr>').join('') + '</table>' +
    '<h4>verifications <span class="dim">n=' + v.stats.n + ' mean err ' + fmt(v.stats.mean_pos_err) + ' m · mean |Δt| ' + fmt(v.stats.mean_abs_time_diff_ms, 0) + ' ms</span></h4><table><tr><th>trajectory</th><th>oracle</th><th>mean err</th><th>max err</th><th>sim ms</th><th>oracle ms</th></tr>' + v.recent.map(x => '<tr><td>' + esc(x.trajectory_hash.slice(0, 12)) + '</td><td>' + esc(x.oracle) + '</td><td>' + fmt(x.mean_pos_err) + '</td><td>' + fmt(x.max_pos_err) + '</td><td>' + x.time_ms_sim + '</td><td>' + x.time_ms_oracle + (x.both_finished ? '' : ' <span class="warn">DNF</span>') + '</td></tr>').join('') + '</table>';
}
let sqlText = 'select method, track_name, min(time_ms) as best_ms, count(*) as n from trajectories where finished group by 1,2 order by 3';
async function renderSql() {
  $('#panel').innerHTML = '<textarea id="sql" class="sql">' + esc(sqlText) + '</textarea><div style="margin:6px 0"><button class="primary" onclick="runSql()">run</button></div><div id="sqlout"></div>';
}
async function runSql() {
  sqlText = $('#sql').value;
  try { const r = await api('/api/query', {sql: sqlText}); const rows = r.rows; if (!rows.length) { $('#sqlout').innerHTML = '<div class="dim">no rows</div>'; return; }
    const cols = Object.keys(rows[0]);
    $('#sqlout').innerHTML = '<table><tr>' + cols.map(c => '<th>' + esc(c) + '</th>').join('') + '</tr>' + rows.map(r => '<tr>' + cols.map(c => '<td>' + esc(typeof r[c] === 'object' ? JSON.stringify(r[c]) : r[c]) + '</td>').join('') + '</tr>').join('') + '</table>';
  } catch (e) { $('#sqlout').innerHTML = '<div class="err">' + esc(e.message) + '</div>'; }
}
async function renderContext() { const r = await api('/api/context'); $('#panel').innerHTML = '<pre>' + esc(r.text) + '</pre>'; }

refreshStatus(); loadChat(); render();
setInterval(() => { refreshStatus(); if (current === 'log') render(); }, 3000);
</script>
</body>
</html>
"##;
