import assert from "node:assert/strict";
import { test } from "node:test";
import { SensorLines, parseSensorLine, summarizeStroke, summarizeRun, comparisonIssues, quantile, csvCell, rawCsv } from "./trackball-diagnostics.mjs";

test("USB fragments preserve signed counts and reject malformed sensor records", () => {
  const parser = new SensorLines();
  assert.deepEqual(parser.push("[      12ms INFO] RTB1,1,1000,-2048,"), []);
  const [sample] = parser.push("2047,42,4660,128\r\nRTB1,2,5000,0,0,0,45,0\n");
  assert.deepEqual(sample, { seq: 1, t: 1, dx: -2048, dy: 2047, squal: 42, shutter: 4660, motion: 128 });
  for (const line of ["RTB1,1,1000,2048,0,42,1,128", "RTB1,1,1000,1,0,42,1,0", "RTB1,1,1000,0,0,256,1,0", "RTB1,1,1000,0,0,42,65536,0", "RTB1,1,1000,NaN,0,42,1,128", "RTB1,1,1000,0,0,42,1,0,extra", "xRTB1,1,1000,0,0,42,1,0"]) assert.equal(parseSensorLine(line), null, line);
  assert.equal(parseSensorLine("RTB1,4294967295,5000,0,0,0,0,0").seq, 0xffffffff);
  assert.equal(parseSensorLine("RTB1,4294967296,5000,0,0,0,0,0"), null);
  parser.push("RTB1," + "0".repeat(2000));
  const recovered = parser.push("tail\nRTB1,3,9000,0,0,10,45,0\n");
  assert.equal(recovered.length, 1); assert.equal(recovered[0].seq, 3); assert.equal(parser.invalid, 1);
});

test("zero motion remains a measurement without claiming 100 percent tracking", () => {
  const samples = Array.from({ length: 51 }, (_, i) => ({ t: i * 100, dx: 0, dy: 0, squal: 0, shutter: 45 }));
  const result = summarizeStroke(samples, 5000);
  assert.equal(result.net, 0); assert.equal(result.rms, 0); assert.equal(result.gapMs, 5000);
  assert.equal(result.squalP10, 0); assert.equal(result.crossRms, null); assert.equal(result.efficiency, null);
  assert.equal(summarizeStroke([], 5000).squalP10, null);
  assert.throws(() => summarizeStroke([{ t: 10, dx: 1, dy: 0 }, { t: 5, dx: 1, dy: 0 }], 30));
  assert.throws(() => summarizeStroke([{ t: 40, dx: 1, dy: 0 }], 30));
});

test("sideways jitter and reverse motion are invariant to sensor mounting rotation", () => {
  const original = [{ t: 10, dx: 10, dy: 2, squal: 40 }, { t: 20, dx: -4, dy: -2, squal: 30 }, { t: 30, dx: 10, dy: 0, squal: 20 }];
  const rotated = original.map(s => ({ ...s, dx: -s.dy, dy: s.dx }));
  const a = summarizeStroke(original, 40), b = summarizeStroke(rotated, 40);
  for (const key of ["net", "path", "crossRms", "reversePercent", "gapMs"]) assert.ok(Math.abs(a[key] - b[key]) < 1e-9, key);
  assert.equal(a.net, 16); assert.ok(a.crossRms > 0); assert.ok(Math.abs(a.reversePercent - 100 / 6) < 1e-9);
  assert.equal(a.gapMs, 10); assert.equal(a.squalP10, 22);
  assert.equal(summarizeStroke(original.map(s => ({ ...s, lost: 2 })), 40).lost, 6);
});

function run(amounts = [100, 50, 80, 75]) {
  return { id: "test", source: "sensor", protocol: "slow", unadjusted: null, complete: true,
    conditions: { name: "紫・現状", ball: "Keychron 紫", position: "現状", cpi: 1600, travel: 10, note: "fixed" },
    strokes: amounts.flatMap((amount, direction) => Array.from({ length: 3 }, () => {
      const samples = [{ t: 0, dx: amount, dy: 0, seq: 1, squal: 40, shutter: 100, motion: 128 }];
      return { direction, samples, invalidReason: "", metrics: summarizeStroke(samples, 100) };
    })) };
}

test("four-direction sensitivity and repeatability expose an imbalance", () => {
  const data = run(); const result = summarizeRun(data);
  assert.equal(result.balance, 50); assert.equal(result.groups[0].cv, 0);
  assert.equal(result.groups[1].net, 50); assert.equal(result.validCount, 12);
  for (const [i, amount] of [9, 10, 11].entries()) data.strokes[i].metrics.net = amount;
  assert.equal(summarizeRun(data).groups[0].cv, 10);
  data.strokes[0].invalidReason = "lost data"; assert.equal(summarizeRun(data).balance, null);
  assert.equal(summarizeRun(run([0, 0, 0, 0])).balance, null);
});

test("comparison refuses mixed input, CPI, travel, and incomplete captures", () => {
  const a = run(); const b = run(); b.protocol = "fast"; assert.deepEqual(comparisonIssues(a, b), []);
  b.conditions.cpi = 800; b.conditions.travel = 20; b.source = "demo"; b.complete = false;
  assert.equal(comparisonIssues(a, b).length, 4);
  b.source = "sensor"; b.complete = true; b.conditions = { ...a.conditions }; b.protocol = "rest";
  assert.equal(comparisonIssues(a, b).length, 1);
  b.protocol = "slow"; b.unadjusted = false; assert.equal(comparisonIssues(a, b).length, 1);
});

test("CSV includes conditions and raw values while neutralizing text formulas", () => {
  const data = run(); data.conditions.name = '=HYPERLINK("https://example.com")'; const csv = rawCsv(data);
  assert.ok(csv.startsWith("\ufeff")); assert.ok(csv.includes("squal")); assert.ok(csv.includes("'="));
  assert.equal(csv.trim().split("\r\n").length, 13);
  assert.equal(csvCell(-2048), "-2048"); assert.equal(csvCell("a,b"), '"a,b"');
  assert.equal(quantile([null, 0, 10, 20, NaN], 0.5), 10);
});
