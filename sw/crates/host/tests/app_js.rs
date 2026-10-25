//! Dashboard logic in demo/app.js, run under node (the file exports its pure
//! helpers when there is no DOM). Skipped with a message if node is not on PATH.

use std::path::PathBuf;
use std::process::Command;

const SCRIPT: &str = r#"
const A = require(process.argv[1]);
const assert = require('assert');
const rec = (name, mode, M, K, N, requant, source, build_id) =>
  ({ gemm: { name, mode, M, K, N, requant }, source, build_id, derived: { util: 0.5 } });

// 3. Mode pairing is by (M, K, N, requant), not by name.
const full = rec('gemm_full_s1_99', 'FULL', 44, 133, 234, true, 'hw', '52a66216');
const strip = rec('gemm_strip_s1_99', 'STRIP', 44, 133, 234, true, 'hw', '38a3950a');
assert.strictEqual(A.otherModeRun([full, strip], strip), full);
assert.strictEqual(A.otherModeRun([full, strip], full), strip);
// Latest of several candidates wins.
const full2 = rec('gemm_full_s1_99', 'FULL', 44, 133, 234, true, 'hw', '38a3950a');
assert.strictEqual(A.otherModeRun([full, full2, strip], strip), full2);
// Any differing dimension, requant flag, or source: no pair. Same mode: no pair.
for (const x of [
  rec('a', 'FULL', 45, 133, 234, true, 'hw', 'b'),
  rec('a', 'FULL', 44, 134, 234, true, 'hw', 'b'),
  rec('a', 'FULL', 44, 133, 235, true, 'hw', 'b'),
  rec('a', 'FULL', 44, 133, 234, false, 'hw', 'b'),
  rec('a', 'FULL', 44, 133, 234, true, 'sim', 'b'),
  rec('a', 'STRIP', 44, 133, 234, true, 'hw', 'b'),
]) assert.strictEqual(A.otherModeRun([x, strip], strip), null, JSON.stringify(x.gemm) + x.source);
// Same name still pairs when the shape matches (ffn_up FULL vs STRIP).
const fu = rec('ffn_up', 'FULL', 1, 64, 256, true, 'hw', 'x'), su = rec('ffn_up', 'STRIP', 1, 64, 256, true, 'hw', 'x');
assert.strictEqual(A.otherModeRun([fu, su], su), fu);

// Header link label: port and rate from the transport string; nothing for tcp / mock.
assert.strictEqual(A.linkLabel('serial:COM3@1562500'), 'COM3 @ 1.5625 Mbaud');
assert.strictEqual(A.linkLabel('serial:COM3@115200'), 'COM3 @ 115200 baud');
assert.strictEqual(A.linkLabel('serial:/dev/ttyUSB1@2000000'), '/dev/ttyUSB1 @ 2 Mbaud');
assert.strictEqual(A.linkLabel('tcp:127.0.0.1:5555 (co-sim)'), '');
assert.strictEqual(A.linkLabel('mock (in-process model board)'), '');
assert.strictEqual(A.linkLabel(undefined), '');

// 2. Two decimals on every ratio.
assert.strictEqual(A.ratio(2.0709), '2.07×');
assert.strictEqual(A.ratio(1), '1.00×');
assert.strictEqual(A.ratio(2.1), '2.10×');

// 1. Builds line computed from the records, first-appearance order.
const ids = [].concat(Array(255).fill('52a66216'), Array(184).fill('38a3950a'));
assert.strictEqual(A.buildsLine('HW', ids), 'HW: builds 52a66216 (255) + 38a3950a (184)');
assert.strictEqual(A.buildsLine('REPLAY', ['38a3950a', '38a3950a']), 'REPLAY: build 38a3950a (2)');
assert.strictEqual(A.buildsLine('HW', []), '');
assert.strictEqual(A.buildsLineCounts('HW', [{ build_id: 'a', n: 3 }, { build_id: 'b', n: 1 }]), 'HW: builds a (3) + b (1)');

// 4. Ticker label: REPLAY in replay view, else the record's own source.
assert.strictEqual(A.recordLabel(full, 'replay'), 'REPLAY');
assert.strictEqual(A.recordLabel(full, 'hw'), 'HW');
assert.strictEqual(A.recordLabel(rec('a', 'FULL', 1, 1, 1, true, 'sim', 'b'), 'sim'), 'SIM');
// 5. Auto panel: rows where the model says STRIP loses, worst ratio first; ties excluded.
const ar = (M, f, s) => ({ M, K: 1, N: 1, requant: true, model_cycles_full: f, model_cycles_strip: s });
const lost = A.autoRowsWhereStripLost([ar(1, 100, 90), ar(2, 100, 110), ar(3, 100, 132), ar(4, 100, 100)]);
assert.deepStrictEqual(lost.map((r) => r.M), [3, 2]);
assert.strictEqual(A.shapeStr({ M: 22, K: 250, N: 17, requant: true }), '22×250×17');
assert.strictEqual(A.shapeStr({ M: 8, K: 17, N: 8, requant: false }), '8×17×8 i32');
console.log('app.js helpers ok');
"#;

#[test]
fn app_js_helpers() {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../demo/app.js");
    assert!(app.exists(), "{}", app.display());
    let out = match Command::new("node").arg("-e").arg(SCRIPT).arg(&app).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP app_js_helpers: node not runnable ({e})");
            return;
        }
    };
    let so = String::from_utf8_lossy(&out.stdout);
    let se = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "node failed:\n{so}\n{se}");
    assert!(so.contains("app.js helpers ok"), "{so}\n{se}");
}
