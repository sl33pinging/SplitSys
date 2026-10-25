// SplitSys dashboard, Scene 4: the tiny LM (stories260K) on the board.
// Vanilla JS, no libraries. Data: GET /api/lm (records in results/<source>/lm/ and
// the in-process live generation) plus SSE lm_* events. Uses app.js globals
// ($, esc, int, setBadge, S) at render time only.
'use strict';

const LM = { st: null, cur: null, play: null, playKey: '', fetching: false };
const LM_ROWS_NAMES = ['wq', 'wk', 'wv', 'wo', 'w1', 'w3', 'w2'];

// ---------- pure helpers (exported to node for tests) ----------
function lmEsc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])); }

// Which record Scene 4 shows and how it is labelled.
// replayActive: the dashboard's global REPLAY view (R). Returns
// { rec, badge: 'hw'|'sim'|'model'|'replay', kind: 'live'|'recorded'|'replay'|'none', note }.
function lmPick(st, replayActive) {
  if (!st) return { rec: null, badge: 'model', kind: 'none', note: '' };
  if (replayActive) {
    if (!st.replay) return { rec: null, badge: 'replay', kind: 'none', note: 'no recorded generation in ' + (st.replay_dir || 'the replay directory') };
    const what = st.replay.source === 'hw' ? '' : ' · ' + (st.replay.mock ? 'MOCK BOARD (model) record, not hardware' : st.replay.source.toUpperCase() + ' record, not hardware');
    return { rec: st.replay, badge: 'replay', kind: 'replay', note: 'REPLAY · recorded ' + lmUtc(st.replay.started_utc) + ' · not live' + what };
  }
  const r = st.live;
  if (!r) return { rec: null, badge: st.live_source || 'model', kind: 'none', note: 'no generation recorded in ' + (st.live_dir || 'results') };
  const badge = r.source === 'model' ? 'model' : r.source;
  if (st.live_this_session) {
    return { rec: r, badge, kind: 'live', note: (r.status === 'running' ? 'generating now' : 'run from this dashboard') + ' · started ' + lmUtc(r.started_utc) };
  }
  return { rec: r, badge, kind: 'recorded', note: 'recorded ' + lmUtc(r.started_utc) + ' (from ' + st.live_dir + ')' };
}
function lmUtc(s) { return String(s || '').replace('T', ' ').replace('Z', ' UTC'); }

// Prompt text and generated continuation after `upto` steps (all if undefined).
function lmText(rec, upto) {
  if (!rec) return { prompt: '', gen: '', tokens: 0 };
  const steps = rec.steps || [];
  const n = upto === undefined ? steps.length : Math.min(upto, steps.length);
  let gen = '', tokens = 0;
  for (let i = 0; i < n; i++) if (steps[i].phase === 'generate') { gen += steps[i].piece || ''; tokens++; }
  const prompt = rec.text.slice(0, rec.text.length - rec.generated_text.length);
  return { prompt, gen, tokens };
}

// Story markup: prompt muted, continuation bright; long stories keep the tail.
function lmStory(txt, max) {
  const lim = max || 330;
  let p = txt.prompt, g = txt.gen;
  if (p.length + g.length > lim) {
    const cut = p.length + g.length - lim;
    if (cut >= p.length) { g = '…' + g.slice(cut - p.length); p = ''; } else p = '…' + p.slice(cut);
  }
  return `<span class="muted">${lmEsc(p)}</span><span>${lmEsc(g)}</span>`;
}

// The call grid of one step: rows L0..L{n-1} with LM_ROWS_NAMES, then the classifier row.
function lmGrid(calls, nLayers) {
  const rows = [];
  for (let l = 0; l < nLayers; l++) rows.push({ label: 'L' + l, cells: LM_ROWS_NAMES.map((n) => ({ layer: l, name: n, call: null })) });
  const cls = { label: 'cls', cells: [] };
  for (const c of calls || []) {
    if (c.layer === null || c.layer === undefined) {
      cls.cells.push({ layer: null, name: c.name, call: c });
    } else if (rows[c.layer]) {
      const cell = rows[c.layer].cells.find((x) => x.name === c.name);
      if (cell) cell.call = c;
    }
  }
  if (!cls.cells.length) cls.cells = [{ layer: null, name: 'cls0', call: null }, { layer: null, name: 'cls1', call: null }];
  rows.push(cls);
  return rows;
}

// Totals over the first `upto` steps (replay playback shows a prefix).
function lmTotals(rec, upto) {
  const t = { steps: 0, gemms: 0, bit_exact: 0, counters_eq_model: 0, cycles: 0, bytes_tx: 0, wall_ms: 0, modes: {} };
  if (!rec) return t;
  const steps = rec.steps || [];
  const n = upto === undefined ? steps.length : Math.min(upto, steps.length);
  for (let i = 0; i < n; i++) {
    const s = steps[i].summary;
    t.steps++; t.gemms += s.gemms; t.bit_exact += s.bit_exact; t.counters_eq_model += s.counters_eq_model;
    t.cycles += s.cycles_total; t.bytes_tx += s.bytes_tx; t.wall_ms += s.wall_ms;
    for (const k in s.modes || {}) t.modes[k] = (t.modes[k] || 0) + s.modes[k];
  }
  return t;
}

// "3.2 s/token" or "4.1 tokens/s" (laptop wall time), never called accelerator speed.
function lmRate(wallMsPerStep) {
  if (!(wallMsPerStep > 0)) return '—';
  const s = wallMsPerStep / 1000;
  return s >= 1 ? s.toFixed(1) + ' s per token' : (1 / s).toFixed(1) + ' tokens/s';
}

function lmModeLine(modes) {
  const f = modes.FULL || 0, s = modes.STRIP || 0;
  if (!f && !s) return 'none yet';
  const n = (x) => Number(x).toLocaleString('en-US');
  if (!f) return `all ${n(s)} GEMMs in STRIP`;
  if (!s) return `all ${n(f)} GEMMs in FULL`;
  return `${n(s)} STRIP · ${n(f)} FULL`;
}

// ---------- rendering ----------
function lmPanelBadges(badge, text) {
  for (const id of ['lm-text-badge', 'lm-ver-badge', 'lm-grid-badge', 'lm-split-badge']) setBadge(id, badge, text);
}

function renderLm() {
  if (typeof S === 'undefined' || !S || S.scene !== 4) return;
  const replayActive = S.source === 'replay';
  const pick = lmPick(LM.st, replayActive);
  const rec = pick.rec;
  // Replay: play the recorded story back token by token (accelerated, labelled).
  let upto;
  if (pick.kind === 'replay' && rec) {
    const key = rec.gen_id;
    if (LM.playKey !== key) { LM.playKey = key; LM.play = { i: 0, t: Date.now() }; }
    upto = LM.forceUpto !== undefined ? LM.forceUpto : (LM.shot ? undefined : LM.play.i);
  } else { LM.playKey = ''; LM.play = null; }
  lmPanelBadges(pick.badge);
  const txt = lmText(rec, upto);
  const tot = lmTotals(rec, upto);
  const mock = rec && rec.mock;
  // Mock-board data is placeholder data on every view (incl. replay of a mock record).
  if (mock) $('watermark').classList.add('on');
  // --- story panel
  $('lm-text-title').textContent = rec
    ? (mock ? 'Tiny LM on the mock board: the model, not hardware' : `Tiny LM on the ${pick.badge === 'sim' ? 'simulated chip' : 'board'}: every linear layer of stories260K runs on the ${pick.badge === 'sim' ? 'simulated RTL' : 'FPGA'}`)
    : 'Tiny LM on the board: stories260K';
  const status = rec ? (pick.kind === 'replay' ? `playback ${Math.min(upto, rec.steps.length)}/${rec.steps.length} steps (accelerated)` : rec.status + (rec.stopped ? ' (stopped)' : '')) : '';
  $('lm-text-body').innerHTML = rec ? `
    <div class="lm-story"><div>${lmStory(txt)}${LM.st && LM.st.running && pick.kind === 'live' ? '<span class="lm-caret">▌</span>' : ''}</div></div>
    <div class="kv muted lm-foot">${lmEsc(pick.note)} · ${txt.tokens} tokens generated · ${lmEsc(status)} · greedy decoding · prompt “${lmEsc(rec.prompt)}”</div>`
    : `<div class="pending-box">${lmEsc(pick.note || 'loading…')}<br>Run <span class="mono">splitsys-host lm --prompt "Once upon a time" --tokens 8</span> or press <b>G</b> here.</div>`;
  // --- verification panel
  const steps = rec ? rec.steps.slice(0, upto === undefined ? rec.steps.length : upto) : [];
  const last = steps.length ? steps[steps.length - 1] : null;
  const allOk = tot.gemms > 0 && tot.bit_exact === tot.gemms && tot.counters_eq_model === tot.gemms;
  $('lm-ver-title').textContent = !rec ? 'Every GEMM checked bit-exact' : mock
    ? 'Mock board: checks are circular (golden checks golden)'
    : (allOk ? `Every GEMM bit-exact on the FPGA, counters = model` : (tot.gemms ? 'MISMATCH: generation aborted' : 'Every GEMM checked bit-exact'));
  const perTok = tot.steps ? tot.cycles / tot.steps : 0;
  const wallPer = tot.steps ? tot.wall_ms / tot.steps : 0;
  $('lm-ver-body').innerHTML = rec ? `
    <div class="mid ${allOk ? 'pass' : (tot.gemms ? 'fail' : '')}">${int(tot.bit_exact)} / ${int(tot.gemms)} <span style="font-size:24px" class="muted">GEMMs bit-exact</span></div>
    <div class="kv" style="margin-top:6px"><span class="mono ${tot.counters_eq_model === tot.gemms ? 'pass' : 'fail'}">${int(tot.counters_eq_model)} / ${int(tot.gemms)}</span> GEMMs with all 12 counters = cycle model</div>
    <div class="kv">${last ? `last step: <span class="mono">${last.summary.gemms}</span> GEMMs, <span class="mono">${last.summary.bit_exact}</span> bit-exact, <span class="mono">${last.summary.counters_eq_model}</span> counters = model` : ''}</div>
    <div class="mid" style="font-size:40px;margin-top:14px">${int(Math.round(perTok))} <span style="font-size:22px" class="muted">on-chip cycles per token (counters)</span></div>
    <div class="kv muted">= ${(perTok / (S.clock_mhz || 100)).toFixed(0)} µs at ${S.clock_mhz || 100} MHz, all ${tot.steps ? Math.round(tot.gemms / tot.steps) : 0} linear layers</div>
    <div class="mid" style="font-size:36px;margin-top:14px">${lmRate(wallPer)}</div>
    <div class="note">${lmEsc(rec.summary.wall_note || 'laptop wall time; NOT accelerator speed')} · ${int(Math.round(tot.steps ? tot.bytes_tx / tot.steps : 0))} B sent per token</div>
    ${mock ? '<div class="note fail" style="margin-top:6px">mock board output comes from the golden model: not evidence</div>' : ''}`
    : '<div class="pending-box">no generation yet</div>';
  // --- call grid
  // While generating, the grid fills in as this step's calls finish (SSE lm_call).
  const running = pick.kind === 'live' && LM.st && LM.st.running && LM.liveCalls;
  const calls = running ? LM.liveCalls : (last ? (last.calls || []) : []);
  const nl = rec && rec.model ? rec.model.n_layers : 5;
  $('lm-grid-title').textContent = rec ? `Which layers ran on the array, and in which shape (${lmModeLine(tot.modes)})` : 'Which layers ran on the array';
  renderLmGrid(lmGrid(calls, nl), rec, last);
  // --- split panel
  const p = rec ? rec.wbuf_plan : null;
  $('lm-split-body').innerHTML = `
    <div class="kv"><b class="pass">FPGA</b> · ${lmEsc((LM.st && LM.st.split.fpga) || '')}</div>
    <div class="kv" style="margin-top:6px"><b>Laptop</b> · ${lmEsc((LM.st && LM.st.split.laptop) || '')}</div>
    ${p ? `<div class="kv muted" style="margin-top:10px">WBUF: ${p.pinned.length} matrices stay resident (${int(p.pinned_bytes)} B); ${int(p.streamed_bytes_per_step)} of ${int(p.total_bytes_per_step)} weight bytes re-sent per token</div>` : ''}`;
}

function renderLmGrid(rows, rec, last) {
  const svg = $('lm-grid-svg');
  if (!svg) return;
  while (svg.firstChild) svg.removeChild(svg.firstChild);
  const x0 = 64, y0 = 30, cw = 146, ch = 36, gap = 6;
  const live = LM.live || {};
  LM_ROWS_NAMES.forEach((n, i) => svg.appendChild(el('text', { x: x0 + i * (cw + gap) + cw / 2, y: y0 - 8, 'text-anchor': 'middle', 'font-size': 18, fill: C.muted, class: 'mono' }, n)));
  rows.forEach((r, ri) => {
    const y = y0 + ri * (ch + gap);
    svg.appendChild(el('text', { x: x0 - 12, y: y + ch / 2 + 6, 'text-anchor': 'end', 'font-size': 19, fill: C.muted, class: 'mono' }, r.label));
    r.cells.forEach((cell, ci) => {
      const x = x0 + ci * (cw + gap);
      const c = cell.call;
      const now = live.name === cell.name && (live.layer === undefined ? null : live.layer) === cell.layer && LM.st && LM.st.running;
      const fill = c ? (c.mode === 'STRIP' ? C.strip : C.full) : C.idle;
      svg.appendChild(el('rect', { x, y, width: cw, height: ch, rx: 4, fill, 'fill-opacity': c ? 0.85 : 1, stroke: now ? C.text : C.line, 'stroke-width': now ? 3 : 1 }));
      const ok = c && c.bit_exact && c.counters_eq_model;
      const label = c ? `${c.mode === 'STRIP' ? 'S' : 'F'} ${c.K}×${c.N} ${ok ? '✓' : '✗'}` : (cell.layer === null ? cell.name : '');
      svg.appendChild(el('text', { x: x + cw / 2, y: y + ch / 2 + 7, 'text-anchor': 'middle', 'font-size': 19, fill: c ? '#0B0D10' : C.muted, class: 'mono', 'font-weight': 700 }, label));
    });
  });
  const yl = y0 + rows.length * (ch + gap) + 22;
  svg.appendChild(el('text', { x: 0, y: yl, 'font-size': 19, fill: C.muted },
    last ? `step ${last.index} (${last.phase}): F = FULL 8×8, S = STRIP 4×16 (${rec && rec.mode_requested === 'AUTO' ? 'picked per GEMM by the cycle model' : 'mode ' + (rec ? rec.mode_requested : '')}); ✓ = bit-exact and counters = model` : 'F = FULL 8×8, S = STRIP 4×16'));
}

async function lmRefresh(force) {
  if (LM.fetching) return;
  if (!force && LM.st && typeof S !== 'undefined' && S && S.scene !== 4) return;
  LM.fetching = true;
  try {
    const r = await fetch('/api/lm', { cache: 'no-store' });
    LM.st = await r.json();
  } catch (e) { /* keep the last view */ }
  LM.fetching = false;
  renderLm();
}

function lmTick() {
  // Replay playback: one step per 1.2 s, then hold the full story for 8 s and loop.
  if (!LM.play || !LM.st || !LM.st.replay) return;
  const n = LM.st.replay.steps.length;
  const dt = Date.now() - LM.play.t;
  if (LM.play.i < n && dt > 1200) { LM.play.i++; LM.play.t = Date.now(); renderLm(); }
  else if (LM.play.i >= n && dt > 8000) { LM.play.i = 0; LM.play.t = Date.now(); renderLm(); }
}

if (typeof document === 'undefined') {
  module.exports = { lmStory, lmPick, lmText, lmGrid, lmTotals, lmRate, lmModeLine };
} else {
  const qs4 = new URLSearchParams(location.search);
  if (qs4.has('shot')) {
    try {
      const x = new XMLHttpRequest();
      x.open('GET', '/api/lm', false);
      x.send();
      LM.st = JSON.parse(x.responseText);
      LM.shot = true;
      if (qs4.has('upto')) LM.forceUpto = +qs4.get('upto');
    } catch (e) { /* render the empty state */ }
    renderLm();
  } else {
    lmRefresh();
    setInterval(lmRefresh, 5000);
    setInterval(lmTick, 200);
    const es4 = new EventSource('/events');
    const re = () => lmRefresh(true);
    es4.addEventListener('lm_started', re);
    es4.addEventListener('lm_step', re);
    es4.addEventListener('lm_finished', re);
    es4.addEventListener('lm_failed', (e) => { try { LM.err = JSON.parse(e.data).error; } catch (x) { /* */ } re(); });
    es4.addEventListener('lm_call', (e) => {
      LM.live = JSON.parse(e.data);
      if (LM.cur !== LM.live.step) { LM.cur = LM.live.step; LM.liveCalls = []; }
      LM.liveCalls.push(LM.live);
      if (S && S.scene === 4) renderLmGridOnly();
    });
    es4.addEventListener('scene', re);
    es4.addEventListener('state', re);
    document.addEventListener('keydown', (e) => {
      if (!S || S.scene !== 4) return;
      if (e.key === 'g' || e.key === 'G') fetch('/api/lm', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ prompt: 'Once upon a time', tokens: 8, mode: 'auto' }) }).then(re).catch(() => {});
      else if (e.key === 'x' || e.key === 'X') fetch('/api/lm', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ stop: true }) }).then(re).catch(() => {});
    });
  }
}

// Live call progress between step records: mark the running cell only.
function renderLmGridOnly() {
  if (!LM.st) return;
  const pick = lmPick(LM.st, S.source === 'replay');
  if (pick.kind !== 'live' || !pick.rec) return;
  const last = pick.rec.steps.length ? pick.rec.steps[pick.rec.steps.length - 1] : null;
  renderLmGrid(lmGrid(LM.liveCalls || [], pick.rec.model.n_layers), pick.rec, last);
}
