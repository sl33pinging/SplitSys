// SplitSys dashboard. Vanilla JS + hand-written SVG; no libraries, no network
// beyond the local splitsys-host server. Every number shown comes from
// /api/state (results/*.json and live runs) and carries a source badge.
'use strict';

const C = { full: '#4C9AFF', strip: '#FF9F43', pass: '#3DDC97', fail: '#FF5C5C', text: '#E8ECF1', muted: '#9AA4AF', line: '#262C33', panel: '#14181D', bg: '#0B0D10', idle: '#0E1115' };
const SVGNS = 'http://www.w3.org/2000/svg';
let S = null;              // last /api/state snapshot
let anim = null;           // running PE-grid animation
let pendingFinish = null;  // run_finished held until the animation ends
const $ = (id) => document.getElementById(id);

// ---------- helpers ----------
function el(tag, attrs, text) {
  const e = document.createElementNS(SVGNS, tag);
  for (const k in attrs || {}) e.setAttribute(k, attrs[k]);
  if (text !== undefined) e.textContent = text;
  return e;
}
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])); }
function pct(x, d = 1) { return (x * 100).toFixed(d) + '%'; }
function signed(x, d = 1) { const v = (x * 100).toFixed(d); return (x >= 0 ? '+' : '−') + Math.abs(v).toFixed(d) + '%'; }
function int(n) { return Number(n).toLocaleString('en-US'); }
function modeColor(m) { return m === 'STRIP' ? C.strip : C.full; }
function badgeFor(src) { return { hw: ['HW', 'hw'], sim: ['SIM', 'sim'], model: ['MODEL', 'model'], replay: ['REPLAY', 'replay'] }[src] || ['—', 'model']; }
function setBadge(id, src, extra) {
  const b = $(id); if (!b) return;
  const [t, c] = badgeFor(src);
  b.textContent = extra || t; b.className = 'badge ' + c;
}
function runs() { return (S && S.runs) || []; }
function lastRun() { const r = runs(); return r.length ? r[r.length - 1] : null; }
function sameShape(a, b) { return a.gemm.name === b.gemm.name && a.gemm.M === b.gemm.M && a.gemm.K === b.gemm.K && a.gemm.N === b.gemm.N && a.gemm.requant === b.gemm.requant; }
function latestOf(pred) { const r = runs(); for (let i = r.length - 1; i >= 0; i--) if (pred(r[i])) return r[i]; return null; }
function srcLabel() { return S ? S.source : 'model'; }

// ---------- scaling ----------
function fit() {
  const s = Math.min(window.innerWidth / 1920, window.innerHeight / 1080);
  const st = $('stage');
  st.style.transform = `scale(${s})`;
  st.style.left = ((window.innerWidth - 1920 * s) / 2) + 'px';
  st.style.top = ((window.innerHeight - 1080 * s) / 2) + 'px';
}
window.addEventListener('resize', fit);

// ---------- header, banners, watermark ----------
function renderChrome() {
  const b = S.board || {};
  $('hdr-meta').textContent = `build ${b.build_id || '—'} · ${S.clock_mhz} MHz`;
  const pill = $('status-pill');
  let banner = '', bcls = '';
  if (S.source === 'replay') {
    pill.textContent = 'REPLAY'; pill.className = 'pill replay';
    banner = `REPLAY · recorded ${S.replay ? S.replay.recorded_utc : ''} · not live`; bcls = 'replay';
  } else if (b.status === 'live') {
    if (S.source === 'hw') { pill.textContent = '● BOARD LIVE'; pill.className = 'pill live'; }
    else if (S.source === 'sim') { pill.textContent = '● CO-SIM (RTL)'; pill.className = 'pill model'; }
    else { pill.textContent = '● MOCK BOARD · MODEL'; pill.className = 'pill model'; }
  } else if (b.status === 'connecting') {
    pill.textContent = 'CONNECTING…'; pill.className = 'pill';
  } else {
    pill.textContent = '● BOARD DISCONNECTED'; pill.className = 'pill down';
    banner = 'BOARD DISCONNECTED · showing last results · reconnecting…'; bcls = 'down';
  }
  const bn = $('banner'); bn.textContent = banner; bn.className = bcls;
  document.body.classList.toggle('has-banner', !!banner);
  $('watermark').classList.toggle('on', !!S.demo_data);
  document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('on', +t.dataset.s === S.scene));
  for (let i = 1; i <= 3; i++) $('scene' + i).classList.toggle('on', S.scene === i);
}

// ---------- Scene 1: PE grid ----------
const G = { x0: 40, y0: 70, cell: 64, gap: 6 };
function peUsed(g, r, c) {
  // Does PE(r,c) ever hold an in-bounds output for this GEMM shape? (SPEC §3.2 mapping)
  if (!g) return false;
  if (g.mode === 'STRIP') {
    const i = r % 4, col = r < 4 ? c : 8 + c;
    return i < Math.min(g.M, 4) && col < g.N;
  }
  return r < Math.min(g.M, 8) && c < Math.min(g.N, 8);
}
function peOutput(mode, mb, nb, r, c) {
  if (mode === 'STRIP') return [mb * 4 + (r % 4), nb * 16 + (r < 4 ? c : 8 + c)];
  return [mb * 8 + r, nb * 8 + c];
}
function renderGrid(active) {
  const svg = $('grid-svg'); svg.innerHTML = '';
  const run = active ? null : lastRun();
  const g = active ? active.gemm : (run ? run.gemm : null);
  const mode = g ? g.mode : 'FULL';
  const col = modeColor(mode);
  const { x0, y0, cell, gap } = G;
  const pitch = cell + gap;
  // column / row labels
  for (let i = 0; i < 8; i++) {
    svg.appendChild(el('text', { x: x0 + i * pitch + cell / 2, y: y0 - 14, 'text-anchor': 'middle', fill: C.muted, 'font-size': 18, class: 'mono' }, 'c' + i));
    svg.appendChild(el('text', { x: x0 - 12, y: y0 + i * pitch + cell / 2 + 6, 'text-anchor': 'end', fill: C.muted, 'font-size': 18, class: 'mono' }, 'r' + i));
  }
  for (let r = 0; r < 8; r++) for (let c = 0; c < 8; c++) {
    let fill = C.idle, op = 1, stroke = '#1d232a';
    if (g) {
      const used = peUsed(g, r, c);
      if (used) { fill = col; op = 0.28; stroke = col; }
      if (active && active.on && active.on[r * 8 + c]) { fill = col; op = 0.95; }
    }
    svg.appendChild(el('rect', { x: x0 + c * pitch, y: y0 + r * pitch, width: cell, height: cell, rx: 6, fill, 'fill-opacity': op, stroke, 'stroke-width': 1.5 }));
  }
  // quadrant borders (heavier)
  const qs = 4 * pitch - gap;
  for (let q = 0; q < 4; q++) {
    const qx = x0 + (q % 2) * 4 * pitch - 4, qy = y0 + Math.floor(q / 2) * 4 * pitch - 4;
    svg.appendChild(el('rect', { x: qx, y: qy, width: qs + 8, height: qs + 8, rx: 9, fill: 'none', stroke: '#5b6672', 'stroke-width': 3 }));
  }
  // STRIP seam at row 4
  const yseam = y0 + 4 * pitch - gap / 2;
  if (mode === 'STRIP' && g) {
    svg.appendChild(el('line', { x1: x0 - 20, y1: yseam, x2: x0 + 8 * pitch + 6, y2: yseam, stroke: C.strip, 'stroke-width': 4, 'stroke-dasharray': '10 6' }));
    const lw = 300, lx0 = x0 + 8 * pitch - lw - 6;
    svg.appendChild(el('rect', { x: lx0, y: yseam - 18, width: lw, height: 36, rx: 6, fill: '#0B0D10', stroke: C.strip, 'stroke-width': 2 }));
    svg.appendChild(el('text', { x: lx0 + lw / 2, y: yseam + 7, 'text-anchor': 'middle', fill: C.strip, 'font-size': 20 }, 'W lanes 8–15 injected here'));
  }
  // per-quadrant HW utilization from mac_useful_q
  if (run && !active) {
    const ct = run.counters.cycles_total || 1;
    for (let q = 0; q < 4; q++) {
      const u = run.counters.mac_useful_q[q] / (16 * ct);
      const cx = x0 + (q % 2) * 4 * pitch + qs / 2, cy = y0 + Math.floor(q / 2) * 4 * pitch + qs / 2;
      svg.appendChild(el('rect', { x: cx - 74, y: cy - 22, width: 148, height: 44, rx: 6, fill: '#0B0D10', 'fill-opacity': 0.82, stroke: '#3a434d' }));
      svg.appendChild(el('text', { x: cx, y: cy + 8, 'text-anchor': 'middle', 'font-size': 22, class: 'mono', fill: u > 0 ? C.text : C.muted }, `Q${q} ${pct(u)}`));
    }
  }
  // footer labels
  const fy = y0 + 8 * pitch + 30;
  if (active) {
    svg.appendChild(el('text', { x: x0, y: fy, 'font-size': 20, fill: C.muted }, `dataflow animation: model · tile ${active.tileLabel}`));
  } else if (run) {
    svg.appendChild(el('text', { x: x0, y: fy, 'font-size': 20, fill: C.muted }, `Q0–Q3: per-quadrant utilization from mac_useful_q (${badgeFor(srcLabel())[0]})`));
  } else {
    svg.appendChild(el('text', { x: x0, y: fy, 'font-size': 20, fill: C.muted }, 'no runs yet · press Space for the scripted demo'));
  }
  const legY = fy + 34;
  svg.appendChild(el('rect', { x: x0, y: legY - 16, width: 18, height: 18, rx: 3, fill: col, 'fill-opacity': 0.5 }));
  svg.appendChild(el('text', { x: x0 + 26, y: legY, 'font-size': 20, fill: C.muted }, 'PE used by this shape'));
  svg.appendChild(el('rect', { x: x0 + 270, y: legY - 16, width: 18, height: 18, rx: 3, fill: C.idle, stroke: '#3a434d' }));
  svg.appendChild(el('text', { x: x0 + 296, y: legY, 'font-size': 20, fill: C.muted }, 'PE idle for this shape'));

  // title states the finding
  let title = 'PE array: waiting for the first run';
  if (g) {
    let usedN = 0; for (let r = 0; r < 8; r++) for (let c = 0; c < 8; c++) if (peUsed(g, r, c)) usedN++;
    title = `${g.mode} at M=${g.M}: ${usedN} of 64 PEs hold outputs`;
  }
  $('grid-title').textContent = title;
  setBadge('grid-badge', srcLabel());
}

function startAnimation(info) {
  const g = info.gemm;
  if (!info.timeline || !g) return;
  const streams = info.timeline.filter((e) => e.phase === 'STREAM');
  if (!streams.length) return;
  const first = info.timeline[0].start;
  const last = streams[streams.length - 1];
  const endCycle = last.start + last.len + 14;
  const tiles = streams.length;
  const dur = Math.min(900 * tiles, 2700);
  anim = { info, g: { name: g.name, M: g.M, K: g.K, N: g.N, mode: g.mode }, t0: performance.now(), dur, first, endCycle, streams, lrd: info.l_rd || 2 };
  requestAnimationFrame(stepAnimation);
}
function stepAnimation(now) {
  if (!anim) return;
  const p = Math.min(1, (now - anim.t0) / anim.dur);
  const cyc = anim.first + p * (anim.endCycle - anim.first);
  const on = new Array(64).fill(false);
  let tileLabel = `1/${S && S.running ? S.running.model_tiles : '?'}`;
  for (const st of anim.streams) {
    const base = st.start + anim.lrd;
    if (cyc < st.start || cyc > base + anim.g.K + 16) continue;
    tileLabel = `${st.tile + 1}/${(anim.info.model_tiles)}`;
    for (let r = 0; r < 8; r++) for (let c = 0; c < 8; c++) {
      const k = cyc - (base + r + c);
      if (k >= 0 && k < anim.g.K) {
        const [m, n] = peOutput(anim.g.mode, st.mb, st.nb, r, c);
        if (m < anim.g.M && n < anim.g.N) on[r * 8 + c] = true;
      }
    }
  }
  renderGrid({ gemm: anim.g, on, tileLabel });
  if (p < 1) requestAnimationFrame(stepAnimation);
  else {
    anim = null;
    if (pendingFinish) { pendingFinish = null; refresh(); } else renderAll();
  }
}

// ---------- Scene 1: this run ----------
function renderRun() {
  const r = lastRun();
  const body = $('run-body');
  setBadge('run-badge', srcLabel());
  if (!r) { $('run-title').textContent = 'No run yet'; body.innerHTML = '<div class="pending-box">Press Space to run the scripted demo (ffn_up at M=1 and M=64, FULL then STRIP).</div>'; return; }
  const g = r.gemm, d = r.derived;
  const other = latestOf((x) => sameShape(x, r) && x.gemm.mode !== g.mode && x.source === r.source);
  let xf = '';
  if (other && other.derived.util > 0) {
    const f = d.util / other.derived.util;
    xf = `<div class="mid ${g.mode === 'STRIP' ? 'strip' : 'full'}" style="font-size:40px;margin-top:14px">${f.toFixed(2)}× <span class="muted" style="font-size:24px">vs ${other.gemm.mode} on the same GEMM</span></div>`;
  } else {
    xf = `<div class="note" style="margin-top:14px">× vs the other mode appears once both modes ran this GEMM</div>`;
  }
  const errU = d.util_model > 0 ? (d.util - d.util_model) / d.util_model : 0;
  $('run-title').textContent = `${g.mode} runs ${g.name} at ${pct(d.util)} utilization`;
  const ok = r.check.golden_match && r.check.invariants_ok;
  body.innerHTML = `
    <div class="kv mono" style="font-size:24px">${esc(g.name)} · M=${g.M} K=${g.K} N=${g.N} · <span class="${g.mode === 'STRIP' ? 'strip' : 'full'}">${g.mode}</span></div>
    <div class="row" style="margin-top:22px"><div class="big ${g.mode === 'STRIP' ? 'strip' : 'full'}">${pct(d.util)}</div><div class="muted" style="font-size:24px">utilization<br>(mac_useful / 64·cycles)</div></div>
    <div class="kv" style="margin-top:12px">model <span class="mono">${pct(d.util_model)}</span> · err <span class="mono">${signed(errU, 2)}</span> · cycles <span class="mono">${int(r.counters.cycles_total)}</span> (model ${int(r.model.cycles_total)})</div>
    ${xf}
    <div style="margin-top:22px;font-size:22px;color:${C.muted}">Cycle breakdown (measured bar, model outline)</div>
    <svg id="brk-svg" width="596" height="150" viewBox="0 0 596 150"></svg>
    <div class="kv" style="margin-top:4px">check: <span class="${ok ? 'pass' : 'fail'}">${ok ? '✓ bit-exact vs golden' : '✗ FAILED'}</span> · ${int(r.check.outputs_compared)} outputs · ${r.check.mismatches} mismatches · invariants ${r.check.invariants_ok ? 'ok' : 'FAIL'}</div>
    <div class="note" style="margin-top:6px">operand bytes/cycle ${d.operand_bytes_per_cycle} (stream) · stall ${int(r.counters.cycles_stall)} cycles</div>`;
  renderBreakdown($('brk-svg'), r);
}
function renderBreakdown(svg, r) {
  const keys = [['cycles_setup', 'setup', '#6b7785'], ['cycles_stream', 'stream', modeColor(r.gemm.mode)], ['cycles_flush', 'flush', '#a78bfa'], ['cycles_drain', 'drain', '#2dd4bf'], ['cycles_stall', 'stall', C.fail]];
  const W = 596, total = Math.max(r.counters.cycles_total, r.model.cycles_total) || 1;
  const draw = (cnt, y, h, outline) => {
    let x = 0;
    for (const [k, , col] of keys) {
      const w = (cnt[k] / total) * W;
      if (w <= 0) continue;
      svg.appendChild(el('rect', outline ? { x, y, width: w, height: h, fill: 'none', stroke: '#fff', 'stroke-width': 1.5 } : { x, y, width: w, height: h, fill: col }));
      x += w;
    }
  };
  draw(r.counters, 4, 40, false);
  draw(r.model, 50, 14, true);
  let lx = 0;
  for (const [k, name, col] of keys) {
    const share = r.counters[k] / (r.counters.cycles_total || 1);
    svg.appendChild(el('rect', { x: lx, y: 88, width: 16, height: 16, fill: col, rx: 3 }));
    svg.appendChild(el('text', { x: lx + 22, y: 102, 'font-size': 19, fill: C.muted }, name));
    svg.appendChild(el('text', { x: lx + 22, y: 128, 'font-size': 19, class: 'mono' }, pct(share, 0)));
    lx += 118;
  }
}

// ---------- Scene 1: verified + cost ----------
function renderVerified() {
  const s = S.session || {};
  setBadge('ver-badge', srcLabel());
  const where = { hw: 'on hardware', sim: 'in RTL simulation', model: 'on the mock board', replay: 'on hardware (replay)' }[srcLabel()] || '';
  if (srcLabel() === 'model') {
    // Mock-board output is computed by the golden model itself: comparing it to
    // golden is circular, so nothing is counted as verified here.
    $('ver-title').textContent = 'Verification: needs RTL co-sim or the board';
    $('ver-body').innerHTML = `<div class="pending-box">Mock-board results are produced by the golden model, so checking them against golden proves nothing. They are <b>not counted</b> as verified.</div>
      <div class="kv muted" style="margin-top:14px">${int(s.gemms_run || 0)} mock runs since ${esc((s.started_utc || '').replace('T', ' ').replace('Z', ' UTC'))}</div>`;
    return;
  }
  const mm = s.mismatches || 0;
  $('ver-title').textContent = `Verified ${where}: ${mm === 0 ? 'zero mismatches' : mm + ' mismatches'}`;
  $('ver-body').innerHTML = `
    <div class="mid">${int(s.gemms_verified || 0)} <span style="font-size:26px" class="muted">GEMMs</span></div>
    <div class="mid" style="font-size:38px;margin-top:6px">${int(s.macs_verified || 0)} <span style="font-size:24px" class="muted">MACs bit-exact</span></div>
    <div class="mid ${mm === 0 ? 'pass' : 'fail'}" style="font-size:38px;margin-top:6px">${int(mm)} <span style="font-size:24px">mismatches</span></div>
    <div class="kv muted" style="margin-top:10px">vs Rust golden model · ${s.failed_runs ? s.failed_runs + ' failed runs · ' : ''}since ${esc((s.started_utc || '').replace('T', ' ').replace('Z', ' UTC'))}</div>`;
}
function renderCost() {
  const c = S.cost;
  const body = $('cost-body');
  if (!c || !c.full_only || !c.split) {
    $('cost-title').textContent = 'Cost of splitting: pending build';
    setBadge('cost-badge', null, 'PENDING'); $('cost-badge').className = 'badge pending';
    body.innerHTML = `<div class="pending-box">Waiting for <span class="mono">results/cost.json</span> from the <span class="mono">ENABLE_STRIP=0</span> vs split Vivado builds. No number is shown until it exists.</div>
      <div class="kv" style="margin-top:16px">Known by design: STRIP reads <span class="mono">20</span> operand B/cycle vs <span class="mono">16</span> (+25%).<span class="badge model" style="position:static;margin-left:10px;font-size:16px">SPEC</span></div>`;
    return;
  }
  const f = c.full_only, s = c.split;
  const d = (k) => s[k] - f[k];
  const rel = (k) => f[k] ? ` (${d(k) >= 0 ? '+' : '−'}${Math.abs(100 * d(k) / f[k]).toFixed(1)}%)` : '';
  const sg = (v) => (v >= 0 ? '+' : '−') + int(Math.abs(v));
  $('cost-title').textContent = `Splitting costs ${sg(d('lut'))} LUTs${rel('lut')} and ${sg(d('dsp'))} DSPs`;
  setBadge('cost-badge', 'hw', c.source === 'hw-build' ? 'BUILD' : esc(c.source || '?').toUpperCase());
  const bd = c.breakdown && c.breakdown.delta;
  const parts = bd ? ['requant', 'array', 'sequencer'].filter((k) => bd[k]).map((k) => `${k} ${sg(bd[k].lut)}${bd[k].dsp ? ` / ${sg(bd[k].dsp)} DSP` : ''}`).join(' · ') : '';
  const wp = c.worst_path && c.worst_path.split;
  const fmaxNote = wp ? `split worst path: ${wp.logic_levels} logic levels, ${Math.round(wp.route_pct)}% routing` : 'see fmax_note';
  body.innerHTML = `
    <table class="err"><tr><th></th><th>FULL only</th><th>split</th><th>Δ</th></tr>
    ${['lut', 'ff', 'dsp'].map((k) => `<tr><td>${k.toUpperCase()}</td><td>${int(f[k])}</td><td>${int(s[k])}</td><td>${sg(d(k))}${rel(k)}</td></tr>`).join('')}</table>
    <div class="kv" style="margin-top:8px;font-size:20px;line-height:1.3">Δ LUT by block: <span class="mono" style="font-size:20px">${parts}</span></div>
    <div class="note" style="margin-top:6px;line-height:1.3">BRAM ${f.bram} in both. Δ includes requant lanes 8–15 (STRIP drains 16 wide). Both meet ${c.clock_mhz || 100} MHz; the Fmax estimates are not attributed to STRIP (${fmaxNote}).</div>`;
}
function renderTicker() {
  const t = $('ticker'); t.innerHTML = '';
  const r = runs().slice(-8).reverse();
  for (let i = 0; i < 8; i++) {
    const x = r[i];
    const d = document.createElement('div');
    if (!x) { d.className = 'tick empty'; d.textContent = '—'; t.appendChild(d); continue; }
    const ok = x.check.golden_match && x.check.invariants_ok;
    d.className = 'tick';
    d.innerHTML = `<span class="mono">${esc(x.gemm.name)} M=${x.gemm.M}</span> <span class="${x.gemm.mode === 'STRIP' ? 'strip' : 'full'}">${x.gemm.mode}</span><br><span class="mono">${pct(x.derived.util)}</span> <span class="${ok ? 'pass' : 'fail'}">${ok ? '✓' : '✗'}</span> <span class="muted">${badgeFor(x.source)[0]}</span>`;
    t.appendChild(d);
  }
}

// ---------- Scene 2 ----------
function modelPoints(name) {
  if (!S.sweep || !S.sweep.points) return [];
  return S.sweep.points.filter((p) => p.gemm.name === name);
}
function measuredPoint(name, M, mode) {
  return latestOf((x) => x.gemm.name === name && x.gemm.M === M && x.gemm.mode === mode && x.gemm.requant);
}
function renderSweep() {
  const svg = $('sweep-svg'); svg.innerHTML = '';
  const name = 'ffn_up';
  const L = 90, T = 20, W = 960, H = 380;
  const ms = S.sweep_m || [1, 2, 4, 8, 16, 32, 64];
  const xi = (m) => L + (Math.log2(m) / Math.log2(64)) * W;
  const yv = (u) => T + H - u * H;
  // decode regime M <= 4
  svg.appendChild(el('rect', { x: L - 20, y: T, width: xi(4) - L + 40, height: H, fill: '#ffffff', 'fill-opacity': 0.05 }));
  svg.appendChild(el('text', { x: L - 8, y: T + 26, 'font-size': 20, fill: C.muted }, 'decode regime (M ≤ 4)'));
  for (let u = 0; u <= 1.0001; u += 0.25) {
    svg.appendChild(el('line', { x1: L - 20, y1: yv(u), x2: L + W + 20, y2: yv(u), stroke: C.line }));
    svg.appendChild(el('text', { x: L - 30, y: yv(u) + 7, 'text-anchor': 'end', 'font-size': 20, fill: C.muted, class: 'mono' }, (u * 100).toFixed(0) + '%'));
  }
  for (const m of ms) svg.appendChild(el('text', { x: xi(m), y: T + H + 32, 'text-anchor': 'middle', 'font-size': 20, fill: C.muted, class: 'mono' }, 'M=' + m));
  svg.appendChild(el('text', { x: L + W / 2, y: T + H + 64, 'text-anchor': 'middle', 'font-size': 20, fill: C.muted }, `${name} (K=64, N=256), M on a log₂ axis · y = utilization`));
  let anyMeasured = false;
  const highlight = new Set(S.highlight || []);
  for (const mode of ['FULL', 'STRIP']) {
    const col = modeColor(mode);
    const pts = ms.map((m) => [m, measuredPoint(name, m, mode)]).filter((p) => p[1]);
    if (pts.length) anyMeasured = true;
    // Connect only adjacent sweep points that were both measured (no interpolation across gaps).
    for (let i = 0; i + 1 < ms.length; i++) {
      const a = measuredPoint(name, ms[i], mode), b = measuredPoint(name, ms[i + 1], mode);
      if (a && b) svg.appendChild(el('line', { x1: xi(ms[i]), y1: yv(a.derived.util), x2: xi(ms[i + 1]), y2: yv(b.derived.util), stroke: col, 'stroke-width': 4 }));
    }
    for (const [m, r] of pts) {
      const hl = highlight.has(r.run_id);
      svg.appendChild(el('circle', { cx: xi(m), cy: yv(r.derived.util), r: hl ? 13 : 9, fill: col, stroke: hl ? '#fff' : 'none', 'stroke-width': 3 }));
    }
    for (const p of modelPoints(name).filter((p) => p.gemm.mode === mode)) {
      svg.appendChild(el('circle', { cx: xi(p.gemm.M), cy: yv(p.derived.util_model), r: 7, fill: 'none', stroke: '#fff', 'stroke-width': 2.5 }));
    }
  }
  // legend
  const lg = el('g', {});
  const ly = T + 4, lx = L + W - 470;
  [['FULL measured', C.full, 'fill'], ['STRIP measured', C.strip, 'fill'], ['model', '#fff', 'ring']].forEach(([t, c, k], i) => {
    const x = lx + i * 175;
    lg.appendChild(k === 'fill' ? el('circle', { cx: x, cy: ly + 12, r: 8, fill: c }) : el('circle', { cx: x, cy: ly + 12, r: 7, fill: 'none', stroke: c, 'stroke-width': 2.5 }));
    lg.appendChild(el('text', { x: x + 14, y: ly + 19, 'font-size': 20, fill: C.muted }, t));
  });
  svg.appendChild(lg);
  if (!anyMeasured) svg.appendChild(el('text', { x: L + W / 2, y: T + H / 2, 'text-anchor': 'middle', 'font-size': 24, fill: C.muted }, 'no measured points yet · model markers only · press Space'));
  if (!S.sweep) svg.appendChild(el('text', { x: L + W / 2, y: T + H / 2 + 34, 'text-anchor': 'middle', 'font-size': 22, fill: C.fail }, 'results/model/sweep.json missing: model markers unavailable'));
  // finding title from the data shown (measured if present, else model)
  const u = (m, mode) => { const r = measuredPoint(name, m, mode); if (r) return r.derived.util; const p = modelPoints(name).find((p) => p.gemm.M === m && p.gemm.mode === mode); return p ? p.derived.util_model : null; };
  const f1 = u(1, 'FULL'), s1 = u(1, 'STRIP'), f64 = u(64, 'FULL'), s64 = u(64, 'STRIP');
  let title = 'Utilization vs M';
  if (f1 && s1 && f64 && s64) {
    const conv = Math.abs(s64 - f64) / Math.max(f64, s64) < 0.1;
    title = s1 > f1 ? `STRIP wins at small M (${(s1 / f1).toFixed(1)}× at M=1)${conv ? '; both converge at prefill' : ''}` : `FULL ≥ STRIP at M=1 (${pct(f1)} vs ${pct(s1)})`;
  }
  $('sweep-title').textContent = title;
  setBadge('sweep-badge', anyMeasured ? srcLabel() : 'model', anyMeasured ? undefined : 'MODEL');
}
function renderBench() {
  const svg = $('bench-svg'); svg.innerHTML = '';
  const shapes = S.bench_shapes || [];
  const groups = [];
  for (const b of shapes) for (const m of [1, 64]) groups.push([b.name, m]);
  const L = 70, T = 14, H = 220, gw = (1828 - L - 10) / Math.max(groups.length, 1), bw = Math.min(56, gw / 3.2);
  const yv = (u) => T + H - u * H;
  for (let u = 0; u <= 1.0001; u += 0.5) {
    svg.appendChild(el('line', { x1: L - 10, y1: yv(u), x2: 1828, y2: yv(u), stroke: C.line }));
    svg.appendChild(el('text', { x: L - 18, y: yv(u) + 7, 'text-anchor': 'end', 'font-size': 20, fill: C.muted, class: 'mono' }, (u * 100) + '%'));
  }
  let measured = 0, wins = 0, decodeGroups = 0;
  groups.forEach(([name, m], gi) => {
    const cx = L + gi * gw + gw / 2;
    const vals = {};
    ['FULL', 'STRIP'].forEach((mode, k) => {
      const x = cx + (k === 0 ? -bw - 4 : 4);
      const r = measuredPoint(name, m, mode);
      const p = modelPoints(name).find((p) => p.gemm.M === m && p.gemm.mode === mode);
      if (r) {
        measured++; vals[mode] = r.derived.util;
        svg.appendChild(el('rect', { x, y: yv(r.derived.util), width: bw, height: r.derived.util * H, fill: modeColor(mode), rx: 3 }));
        svg.appendChild(el('text', { x: x + bw / 2, y: yv(r.derived.util) - 8, 'text-anchor': 'middle', 'font-size': 18, class: 'mono' }, (r.derived.util * 100).toFixed(0)));
      } else {
        svg.appendChild(el('rect', { x, y: T + H - 40, width: bw, height: 40, fill: 'none', stroke: modeColor(mode), 'stroke-dasharray': '5 4', rx: 3 }));
      }
      if (p) svg.appendChild(el('line', { x1: x - 5, x2: x + bw + 5, y1: yv(p.derived.util_model), y2: yv(p.derived.util_model), stroke: '#fff', 'stroke-width': 3 }));
    });
    if (m === 1 && vals.FULL !== undefined && vals.STRIP !== undefined) { decodeGroups++; if (vals.STRIP > vals.FULL) wins++; }
    svg.appendChild(el('text', { x: cx, y: T + H + 30, 'text-anchor': 'middle', 'font-size': 20 }, name));
    svg.appendChild(el('text', { x: cx, y: T + H + 56, 'text-anchor': 'middle', 'font-size': 19, fill: C.muted, class: 'mono' }, 'M=' + m));
  });
  $('bench-title').textContent = decodeGroups ? `STRIP beats FULL on ${wins} of ${decodeGroups} decode (M=1) benchmarks` : 'Benchmarks: not run yet (dashed = not run; white tick = model)';
  setBadge('bench-badge', measured ? srcLabel() : 'model', measured ? undefined : 'MODEL');
}
function renderErrTable() {
  const r = lastRun();
  setBadge('err-badge', srcLabel());
  const body = $('err-body');
  if (!r) { $('err-title').textContent = 'Model vs measured: no runs yet'; body.innerHTML = ''; return; }
  const keys = ['cycles_total', 'cycles_setup', 'cycles_stream', 'cycles_flush', 'cycles_drain', 'cycles_stall', 'mac_useful', 'tiles'];
  const all = runs();
  let worst = 0;
  const rows = keys.map((k) => {
    const meas = r.counters[k], mod = r.model[k];
    const e = mod ? (meas - mod) / mod : (meas === 0 ? 0 : 1);
    let mx = 0;
    for (const x of all) { const mo = x.model[k], me = x.counters[k]; const ee = mo ? Math.abs((me - mo) / mo) : (me === 0 ? 0 : 1); if (ee > mx) mx = ee; }
    worst = Math.max(worst, mx);
    const hot = Math.abs(e) > 0.05, hot2 = mx > 0.05;
    return `<tr><td>${k}</td><td>${int(meas)}</td><td>${int(mod)}</td><td class="${hot ? 'hot' : ''}">${signed(e, 2)}</td><td class="${hot2 ? 'hot' : ''}">${(mx * 100).toFixed(2)}%</td></tr>`;
  }).join('');
  $('err-title').textContent = worst <= 0.05 ? `Model within 5% on every counter (${all.length} runs)` : `Model off by up to ${(worst * 100).toFixed(1)}%: see footnote`;
  const mockNote = srcLabel() === 'model' ? '<div class="note" style="margin-top:8px;color:#FF5C5C">Mock board: its counters ARE the model, so errors are 0 by construction. Real errors come from SIM/HW runs.</div>' : '';
  body.innerHTML = `<div class="kv mono" style="font-size:20px;margin-bottom:6px">last run: ${esc(r.gemm.name)} M=${r.gemm.M} ${r.gemm.mode}</div>
    <table class="err"><tr><th>counter</th><th>measured</th><th>model</th><th>err</th><th>max |err|, all</th></tr>${rows}</table>
    <div class="note" style="margin-top:8px">Values over 5% are red and need an explanation, not a tweak (SPEC §7).${S.latencies_provisional ? ' Latencies are provisional until RTL confirms them.' : ''}</div>${mockNote}`;
}

// ---------- Scene 3 ----------
function renderBlock() {
  const svg = $('block-svg'); if (svg.childNodes.length) return;
  const box = (x, y, w, h, label, col, sub) => {
    svg.appendChild(el('rect', { x, y, width: w, height: h, rx: 8, fill: '#1a1f26', stroke: col || '#5b6672', 'stroke-width': 2 }));
    svg.appendChild(el('text', { x: x + w / 2, y: y + h / 2 + (sub ? -4 : 7), 'text-anchor': 'middle', 'font-size': 20 }, label));
    if (sub) svg.appendChild(el('text', { x: x + w / 2, y: y + h / 2 + 22, 'text-anchor': 'middle', 'font-size': 18, fill: C.muted }, sub));
  };
  const arrow = (x1, y1, x2, y2, col) => {
    svg.appendChild(el('line', { x1, y1, x2, y2, stroke: col || '#8a939d', 'stroke-width': 2.5 }));
    const a = Math.atan2(y2 - y1, x2 - x1), s = 10;
    svg.appendChild(el('polygon', { points: `${x2},${y2} ${x2 - s * Math.cos(a - 0.4)},${y2 - s * Math.sin(a - 0.4)} ${x2 - s * Math.cos(a + 0.4)},${y2 - s * Math.sin(a + 0.4)}`, fill: col || '#8a939d' }));
  };
  box(10, 10, 210, 60, 'host (laptop)', null, 'USB-UART');
  box(260, 10, 300, 60, 'packet: framing + CRC-8', null, 'regfile / mem mux');
  box(600, 10, 340, 60, 'sequencer FSM', null, 'setup→stream→flush→drain');
  arrow(220, 40, 258, 40); arrow(560, 40, 598, 40);
  box(10, 130, 210, 66, 'WBUF 128 b', null, '16 lanes · BRAM');
  box(10, 230, 210, 66, 'ABUF 64 b', null, '8 lanes · BRAM');
  box(260, 130, 170, 66, 'W skew', null, 'lane j: base+j');
  box(260, 230, 170, 66, 'A skew', null, 'row r: r');
  arrow(220, 163, 258, 163); arrow(220, 263, 258, 263);
  // array
  const ax = 480, ay = 120, aw = 300, ah = 300;
  svg.appendChild(el('rect', { x: ax, y: ay, width: aw, height: ah, rx: 8, fill: '#1a1f26', stroke: '#8a939d', 'stroke-width': 2 }));
  svg.appendChild(el('line', { x1: ax + aw / 2, y1: ay, x2: ax + aw / 2, y2: ay + ah, stroke: '#5b6672', 'stroke-width': 2 }));
  svg.appendChild(el('line', { x1: ax, y1: ay + ah / 2, x2: ax + aw, y2: ay + ah / 2, stroke: C.strip, 'stroke-width': 3, 'stroke-dasharray': '10 6' }));
  [['Q0', 0, 0], ['Q1', 1, 0], ['Q2', 0, 1], ['Q3', 1, 1]].forEach(([q, i, j]) => svg.appendChild(el('text', { x: ax + aw / 4 + i * aw / 2, y: ay + ah / 4 + j * ah / 2 + 8, 'text-anchor': 'middle', 'font-size': 24, fill: C.muted }, q)));
  svg.appendChild(el('text', { x: ax + aw / 2, y: ay - 10, 'text-anchor': 'middle', 'font-size': 20 }, '8×8 PE array · 64 DSP48E1'));
  arrow(430, 163, 478, 163, C.full);
  svg.appendChild(el('text', { x: 432, y: 148, 'font-size': 18, fill: C.full }, 'W0–7'));
  arrow(430, 263, 478, 263);
  // STRIP mux at row 4
  box(260, 330, 170, 76, 'row-4 W mux', C.strip, 'STRIP only');
  svg.appendChild(el('path', { d: `M 345 196 L 345 214 L 240 214 L 240 368 L 258 368`, fill: 'none', stroke: C.strip, 'stroke-width': 2.5 }));
  svg.appendChild(el('text', { x: 140, y: 350, 'font-size': 18, fill: C.strip }, 'W 8–15'));
  svg.appendChild(el('text', { x: 140, y: 372, 'font-size': 18, fill: C.strip }, 'delay 4 + c'));
  arrow(430, 368, ax + 6, ay + ah / 2 + 6, C.strip);
  // drain path
  box(820, 150, 126, 66, 'drain mux', null, '8 / 4 rows');
  box(820, 250, 126, 66, 'requant', null, '×16 lanes');
  box(820, 350, 126, 66, 'OBUF', null, '128 b');
  arrow(780, 183, 818, 183); arrow(883, 216, 883, 248); arrow(883, 316, 883, 348);
  box(10, 430, 420, 66, 'perf counters (12 × 48 b)', null, 'cycles · mac_useful · per quadrant');
  box(480, 440, 300, 56, 'board I/O · self-test ROM', null);
  svg.appendChild(el('text', { x: 820, y: 460, 'font-size': 18, fill: C.muted }, 'source:'));
  svg.appendChild(el('text', { x: 820, y: 484, 'font-size': 18, fill: C.muted }, 'SPEC §2, §3.2'));
}
function renderRoof() {
  const svg = $('roof-svg'); svg.innerHTML = '';
  const R = S.roofline;
  const L = 80, T = 16, W = 700, H = 410;
  const xmin = 0.05, xmax = 20, ymin = 0.5, ymax = 100;
  const lx = (v) => L + (Math.log10(v / xmin) / Math.log10(xmax / xmin)) * W;
  const ly = (v) => T + H - (Math.log10(v / ymin) / Math.log10(ymax / ymin)) * H;
  svg.appendChild(el('rect', { x: L, y: T, width: W, height: H, fill: 'none', stroke: C.line }));
  [0.1, 1, 10].forEach((v) => { svg.appendChild(el('line', { x1: lx(v), x2: lx(v), y1: T, y2: T + H, stroke: C.line })); svg.appendChild(el('text', { x: lx(v), y: T + H + 28, 'text-anchor': 'middle', 'font-size': 20, fill: C.muted, class: 'mono' }, v)); });
  [1, 4, 16, 64].forEach((v) => { svg.appendChild(el('line', { x1: L, x2: L + W, y1: ly(v), y2: ly(v), stroke: C.line })); svg.appendChild(el('text', { x: L - 10, y: ly(v) + 7, 'text-anchor': 'end', 'font-size': 20, fill: C.muted, class: 'mono' }, v)); });
  svg.appendChild(el('text', { x: L + W / 2, y: T + H + 60, 'text-anchor': 'middle', 'font-size': 20, fill: C.muted }, 'arithmetic intensity (MACs per operand byte, log)'));
  svg.appendChild(el('text', { x: 18, y: T + H / 2, 'text-anchor': 'middle', 'font-size': 20, fill: C.muted, transform: `rotate(-90 18 ${T + H / 2})` }, 'MACs / cycle (log)'));
  if (!R) { svg.appendChild(el('text', { x: L + W / 2, y: T + H / 2, 'text-anchor': 'middle', 'font-size': 22, fill: C.fail }, 'results/model/roofline.json missing')); $('roof-title').textContent = 'Roofline: model pending'; setBadge('roof-badge', 'model'); return; }
  const peak = R.roofs.compute_macs_per_cycle;
  for (const bwr of R.roofs.bandwidth) {
    const col = modeColor(bwr.mode), b = bwr.bytes_per_cycle;
    const xk = peak / b;
    svg.appendChild(el('polyline', { points: `${lx(xmin)},${ly(Math.max(ymin, xmin * b))} ${lx(xk)},${ly(peak)} ${lx(xmax)},${ly(peak)}`, fill: 'none', stroke: col, 'stroke-width': 2.5, 'stroke-dasharray': '8 5' }));
    const ax0 = 0.09;
    svg.appendChild(el('text', { x: lx(ax0), y: ly(ax0 * b) + (bwr.mode === 'STRIP' ? -40 : 34), 'font-size': 19, fill: col }, `${b} B/cycle (${bwr.mode})`));
  }
  svg.appendChild(el('text', { x: L + W - 6, y: ly(peak) - 10, 'text-anchor': 'end', 'font-size': 19 }, `compute roof ${peak} MACs/cycle`));
  for (const p of R.points) {
    if (p.M !== 1 && p.M !== 64) continue;
    svg.appendChild(el('circle', { cx: lx(p.arithmetic_intensity), cy: ly(p.macs_per_cycle), r: 7, fill: 'none', stroke: modeColor(p.mode), 'stroke-width': 2.5 }));
  }
  let nm = 0;
  for (const r of runs()) {
    const d = r.derived;
    if (!d.arithmetic_intensity || !d.macs_per_cycle) continue;
    nm++;
    svg.appendChild(el('circle', { cx: lx(d.arithmetic_intensity), cy: ly(d.macs_per_cycle), r: 5, fill: modeColor(r.gemm.mode) }));
  }
  const lg = [[`hollow: model (M=1, 64)`, 'ring'], [`filled: measured (${nm})`, 'fill']];
  lg.forEach(([t, k], i) => {
    const x = L + 16, y = T + 26 + i * 28;
    svg.appendChild(k === 'ring' ? el('circle', { cx: x, cy: y - 6, r: 7, fill: 'none', stroke: '#fff', 'stroke-width': 2.5 }) : el('circle', { cx: x, cy: y - 6, r: 6, fill: '#fff' }));
    svg.appendChild(el('text', { x: x + 16, y, 'font-size': 19, fill: C.muted }, t));
  });
  $('roof-title').textContent = 'Decode GEMMs sit far below the roof: few MACs per operand byte';
  setBadge('roof-badge', nm ? srcLabel() : 'model', nm ? undefined : 'MODEL');
}
function renderTrade() {
  const c = S.cost;
  const lut = c && c.full_only && c.split ? `${c.split.lut - c.full_only.lut >= 0 ? '+' : '−'}${int(Math.abs(c.split.lut - c.full_only.lut))} LUTs` : null;
  const f1 = measuredPoint('ffn_up', 1, 'FULL'), s1 = measuredPoint('ffn_up', 1, 'STRIP');
  const gain = f1 && s1 && f1.derived.util > 0 ? `${(s1.derived.util / f1.derived.util).toFixed(2)}×` : null;
  $('trade-title').textContent = `STRIP buys utilization on skinny matmuls for +25% operand bandwidth${lut ? ' and ' + lut : ''}`;
  setBadge('trade-badge', gain ? srcLabel() : 'model', gain ? undefined : 'SPEC');
  $('trade-body').innerHTML = `
    <div><b>Gain:</b> ${gain ? `<span class="mono strip">${gain}</span> utilization on ffn_up at M=1 (STRIP vs FULL, ${badgeFor(srcLabel())[0]})` : '<span class="muted">pending a FULL and a STRIP run of ffn_up at M=1</span>'}</div>
    <div><b>Bandwidth:</b> <span class="mono">20</span> vs <span class="mono">16</span> operand bytes per stream cycle (4 A + 16 W vs 8 A + 8 W).</div>
    <div><b>Area:</b> ${lut ? `<span class="mono">${lut}</span>, <span class="mono">+${c.split.dsp - c.full_only.dsp}</span> DSPs (Vivado, BUILD), incl. the 16-wide requant; both builds meet timing at ${c.clock_mhz || 100} MHz` : '<span class="muted">pending build (results/cost.json)</span>'}</div>
    <div><b>When it pays:</b> M ≤ 4 (decode). At M ≥ 8 both shapes fill the array.</div>`;
}

// ---------- render / data ----------
function renderAll() {
  if (!S) return;
  renderChrome();
  if (!anim) renderGrid(null);
  renderRun(); renderVerified(); renderCost(); renderTicker();
  renderSweep(); renderBench(); renderErrTable();
  renderBlock(); renderRoof(); renderTrade();
}
async function refresh() {
  try {
    const r = await fetch('/api/state', { cache: 'no-store' });
    S = await r.json();
    renderAll();
  } catch (e) {
    if (S) { S.board = Object.assign({}, S.board, { status: 'disconnected' }); renderChrome(); }
    else setTimeout(refresh, 500); // first load failed: retry until the server answers
  }
}
function post(path, body) {
  return fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) }).then(() => refresh()).catch(() => {});
}
function connectEvents() {
  const es = new EventSource('/events');
  es.addEventListener('run_started', (e) => { const info = JSON.parse(e.data); if (S) S.running = info; if (S && S.scene === 1) startAnimation(info); });
  es.addEventListener('run_finished', () => { if (anim) pendingFinish = true; else refresh(); });
  es.addEventListener('run_failed', () => refresh());
  es.addEventListener('board_status', () => refresh());
  es.addEventListener('scene', (e) => { const d = JSON.parse(e.data); if (S) { S.scene = d.scene; renderChrome(); } refresh(); });
  es.addEventListener('state', () => refresh());
  es.onerror = () => { if (S) { S.board = Object.assign({}, S.board, { status: 'disconnected', error: 'dashboard server unreachable' }); renderChrome(); } };
  es.onopen = () => refresh();
}
document.addEventListener('keydown', (e) => {
  if (e.key === '1' || e.key === '2' || e.key === '3') post('/api/scene', { scene: +e.key });
  else if (e.key === ' ') { e.preventDefault(); post('/api/demo'); }
  else if (e.key === 'a' || e.key === 'A') post('/api/attract');
  else if (e.key === 'r' || e.key === 'R') post('/api/replay');
  else if (e.key === 'f' || e.key === 'F') { if (document.documentElement.requestFullscreen) document.documentElement.requestFullscreen().catch(() => {}); }
});
document.querySelectorAll('.tab').forEach((t) => t.addEventListener('click', () => post('/api/scene', { scene: +t.dataset.s })));
// ?scene=N (screenshots) shows a scene locally without changing the server's scene.
const qs = new URLSearchParams(location.search);
const forcedScene = +qs.get('scene') || 0;
const _origRender = renderChrome;
renderChrome = function () { if (forcedScene && S) S.scene = forcedScene; _origRender(); };
// ?shot=1 (headless screenshots): no SSE / polling, so the page reaches network idle.
const shot = qs.has("shot");
fit();
if (shot) {
  // Headless screenshots use a virtual-time budget that can expire before an async
  // fetch resolves; a synchronous request pins the state before the first paint.
  try {
    const x = new XMLHttpRequest();
    x.open('GET', '/api/state', false);
    x.send();
    S = JSON.parse(x.responseText);
    renderAll();
  } catch (e) { refresh(); }
} else {
  refresh();
  connectEvents();
  setInterval(refresh, 5000);
}
