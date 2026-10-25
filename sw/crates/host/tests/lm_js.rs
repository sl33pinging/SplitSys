//! Scene 4 logic in demo/lm.js, run under node (the file exports its pure helpers
//! when there is no DOM). Skipped with a message if node is not on PATH.

use std::path::PathBuf;
use std::process::Command;

const SCRIPT: &str = r#"
const L = require(process.argv[1]);
const assert = require('assert');
const call = (layer, name, mode, ok) => ({ layer, name, mode, K: 64, N: 64, bit_exact: ok, counters_eq_model: ok });
const step = (i, phase, piece, calls, wall) => ({ index: i, phase, piece, calls,
  summary: { gemms: calls.length, bit_exact: calls.filter((c) => c.bit_exact).length,
    counters_eq_model: calls.filter((c) => c.counters_eq_model).length, cycles_total: 100 * calls.length,
    bytes_tx: 1000, wall_ms: wall, modes: calls.reduce((m, c) => (m[c.mode] = (m[c.mode] || 0) + 1, m), {}) } });
const rec = (source, mock, steps) => ({ gen_id: 'g1', source, mock, status: 'done', started_utc: '2026-10-25T01:02:03Z',
  prompt: 'Once', text: 'Once upon', generated_text: ' upon', steps, model: { n_layers: 2 } });
const cs = [call(0, 'wq', 'STRIP', true), call(1, 'w2', 'FULL', true), call(null, 'cls0', 'STRIP', true), call(null, 'cls1', 'STRIP', true)];
const hw = rec('hw', false, [step(0, 'prompt', 'x', cs, 20000), step(1, 'generate', ' up', cs, 22000), step(2, 'generate', 'on', cs, 24000)]);

// Source labels: live this session, recorded on disk, replay; mock replays say so.
let p = L.lmPick({ live: hw, live_this_session: true, live_source: 'hw' }, false);
assert.strictEqual(p.kind, 'live'); assert.strictEqual(p.badge, 'hw');
p = L.lmPick({ live: hw, live_this_session: false, live_source: 'hw', live_dir: 'results/hw/lm' }, false);
assert.strictEqual(p.kind, 'recorded'); assert.ok(p.note.includes('recorded 2026-10-25 01:02:03 UTC'), p.note);
p = L.lmPick({ live: null, replay: hw, replay_dir: 'results/hw/lm' }, true);
assert.strictEqual(p.kind, 'replay'); assert.strictEqual(p.badge, 'replay');
assert.ok(p.note.startsWith('REPLAY · recorded') && p.note.includes('not live') && !p.note.includes('MOCK'), p.note);
const mk = rec('model', true, hw.steps);
p = L.lmPick({ replay: mk }, true);
assert.ok(p.note.includes('MOCK BOARD (model) record, not hardware'), p.note);
p = L.lmPick({ live: mk, live_this_session: true }, false);
assert.strictEqual(p.badge, 'model');
assert.strictEqual(L.lmPick({ replay: null, replay_dir: 'd' }, true).rec, null);
assert.strictEqual(L.lmPick(null, false).kind, 'none');

// Text: prompt vs generated, prefix during playback.
assert.deepStrictEqual(L.lmText(hw), { prompt: 'Once', gen: ' upon', tokens: 2 });
assert.deepStrictEqual(L.lmText(hw, 2), { prompt: 'Once', gen: ' up', tokens: 1 });
assert.deepStrictEqual(L.lmText(hw, 0), { prompt: 'Once', gen: '', tokens: 0 });
// Long stories keep the tail, prompt first to go.
assert.ok(L.lmStory({ prompt: 'P'.repeat(10), gen: 'g'.repeat(20) }, 25).includes('…PPPPP'));
assert.ok(!L.lmStory({ prompt: 'P'.repeat(10), gen: 'g'.repeat(40) }, 25).includes('P'));
assert.ok(L.lmStory({ prompt: '<b>', gen: '&' }).includes('&lt;b&gt;'));

// Grid: layers x 7 projections, classifier row; calls land in their cells.
const g = L.lmGrid(cs, 2);
assert.strictEqual(g.length, 3);
assert.deepStrictEqual(g[0].cells.map((c) => c.name), ['wq', 'wk', 'wv', 'wo', 'w1', 'w3', 'w2']);
assert.strictEqual(g[0].cells[0].call.mode, 'STRIP');
assert.strictEqual(g[0].cells[1].call, null);
assert.strictEqual(g[1].cells[6].call.mode, 'FULL');
assert.deepStrictEqual(g[2].cells.map((c) => c.name), ['cls0', 'cls1']);
assert.strictEqual(L.lmGrid([], 5).length, 6);

// Totals and the wall-time rate (labelled elsewhere as laptop wall time).
const t = L.lmTotals(hw);
assert.strictEqual(t.gemms, 12); assert.strictEqual(t.bit_exact, 12); assert.strictEqual(t.steps, 3);
assert.deepStrictEqual(t.modes, { STRIP: 9, FULL: 3 });
assert.strictEqual(L.lmTotals(hw, 1).gemms, 4);
assert.strictEqual(L.lmRate(22000), '22.0 s per token');
assert.strictEqual(L.lmRate(250), '4.0 tokens/s');
assert.strictEqual(L.lmRate(0), '—');
assert.strictEqual(L.lmModeLine({ STRIP: 1036 }), 'all 1,036 GEMMs in STRIP');
assert.strictEqual(L.lmModeLine({ STRIP: 9, FULL: 3 }), '9 STRIP · 3 FULL');
assert.strictEqual(L.lmModeLine({}), 'none yet');
// Scene 4 controls: stepper clamps to 1..max, token box value, estimate wording.
assert.strictEqual(L.lmStepTokens('8', 1, 200), 9);
assert.strictEqual(L.lmStepTokens('1', -1, 200), 1);
assert.strictEqual(L.lmStepTokens('200', 1, 200), 200);
assert.strictEqual(L.lmStepTokens('', 1, 200), 9);
assert.strictEqual(L.lmStepTokens('999', -1, 200), 200);
assert.strictEqual(L.lmTokensValue(' 12 '), 12);
assert.strictEqual(L.lmTokensValue('2.5'), 2.5);
assert.strictEqual(L.lmTokensValue('abc'), 'abc');
assert.strictEqual(L.lmTokensValue(''), '');
const rate = { s_per_step: 2.17, baud: 1562500, transport: 'serial:COM3@1562500' };
let est = L.lmEstimate(rate, 8, 'hw', 'serial:COM3@1562500');
assert.strictEqual(est, 'Estimate ≈ 17 s laptop wall time (8 × 2.17 s/token, measured at 1,562,500 baud) + ≈ 2.2 s per prompt token');
assert.ok(L.lmEstimate(rate, 200, 'hw').includes('≈ 7 min 14 s'), L.lmEstimate(rate, 200, 'hw'));
assert.ok(L.lmEstimate({ s_per_step: 0.05, transport: 'mock (in-process model board)' }, 8, 'model').includes('MODEL record on mock)'));
assert.ok(L.lmEstimate(null, 8, 'hw', 'serial:COM3@115200').includes('no measured s/token yet on serial:COM3@115200'));
assert.strictEqual(L.lmEstimate(rate, 'abc', 'hw'), '');
assert.strictEqual(L.lmDur(89.4), '89 s');
console.log('lm.js helpers ok');
"#;

#[test]
fn lm_js_helpers() {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../demo/lm.js");
    assert!(f.exists(), "{}", f.display());
    let out = match Command::new("node").arg("-e").arg(SCRIPT).arg(&f).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP lm_js_helpers: node not runnable ({e})");
            return;
        }
    };
    let so = String::from_utf8_lossy(&out.stdout);
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "node failed:\n{so}\n{se}");
    assert!(so.contains("lm.js helpers ok"), "{so}\n{se}");
}

/// No external URLs or network fetches beyond the local server (offline booth).
#[test]
fn lm_js_is_offline() {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../demo/lm.js");
    let s = std::fs::read_to_string(f).unwrap();
    for bad in ["http://", "https://", "//cdn", "@import", "fonts.g"] {
        let hits: Vec<&str> = s.lines().filter(|l| l.contains(bad) && !l.trim_start().starts_with("//")).collect();
        assert!(hits.is_empty(), "{bad}: {hits:?}");
    }
}
