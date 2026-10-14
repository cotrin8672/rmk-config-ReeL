import assert from "node:assert/strict";
import { test } from "node:test";
import { SensorPackets, BleSensorConnection, bleService, frameMetrics, frameRgba, SensorLines, parseSensorLine, summarizeStroke, summarizeRun, comparisonIssues, quantile, csvCell, rawCsv } from "./trackball-diagnostics.mjs";

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

const telemetry = new Uint8Array([84, 1, 7, 0, 0, 0, 160, 15, 0, 0, 255, 255, 0, 248, 42, 52, 18, 128, 64, 6]);
function status(id, phase) { const b = new Uint8Array(20); b.set([83, 1, id, 0, phase]); return b; }
function chunks(id, pixels) { return Array.from({ length: 35 }, (_, i) => {
  const b = new Uint8Array(20), offset = i * 14; b.set([70, 1, id, 0, offset & 255, offset >>> 8]);
  b.set(pixels.slice(offset, offset + 14), 6); return b;
}); }

test("BLE preserves the firmware wire fixture and unwraps microsecond clocks", () => {
  const parser = new SensorPackets();
  const padded = new Uint8Array(40); padded.set(telemetry, 10);
  assert.deepEqual(parser.push(new DataView(padded.buffer, 10, 20)).sample,
    { seq: 7, t: 4, dx: -1, dy: -2048, squal: 42, shutter: 4660, motion: 128, cpi: 1600 });
  const next = telemetry.slice(), view = new DataView(next.buffer);
  view.setUint32(2, 8, true); view.setUint32(6, 0xffffff00, true);
  const a = parser.push(next).sample;
  view.setUint32(2, 9, true); view.setUint32(6, 0x100, true);
  const b = parser.push(next).sample;
  assert.ok(Math.abs(b.t - a.t - 0.512) < 1e-6);
  view.setUint32(2, 1, true); view.setUint32(6, 1000, true);
  assert.equal(parser.push(next).sample.t, 1);
  next[17] = 0; assert.equal(parser.push(next), null);
  assert.equal(parser.push(new Uint8Array(19)), null);
  next[17] = 128; view.setUint16(18, 201, true); assert.equal(parser.push(next), null);
});

test("only complete matching 484-pixel frames survive BLE chunk loss or corruption", () => {
  const pixels = Uint8Array.from({ length: 484 }, (_, i) => i % 128), parser = new SensorPackets();
  const packets = chunks(23, pixels);
  parser.push(status(23, 1));
  for (const packet of packets) assert.equal(parser.push(packet).kind, "progress");
  const frame = parser.push(status(23, 2));
  assert.equal(frame.kind, "frame"); assert.deepEqual(frame.pixels, pixels);
  parser.push(status(23, 1)); for (const packet of packets.slice(1)) parser.push(packet);
  assert.equal(parser.push(status(23, 2)).kind, "error");
  parser.push(status(23, 1)); parser.push(packets[0]); assert.equal(parser.push(packets[0]), null);
  assert.equal(parser.push(status(23, 2)).kind, "error");
  parser.push(status(23, 1)); const corrupt = packets[0].slice(); corrupt[6] = 128;
  assert.equal(parser.push(corrupt), null); assert.equal(parser.push(status(23, 2)).kind, "error");
  parser.push(status(23, 1)); assert.equal(parser.push(status(23, 3)).kind, "error");
  parser.push(status(23, 1)); assert.equal(parser.push(chunks(24, pixels)[0]), null);
  assert.equal(parser.push(status(24, 2)).kind, "error");
});

test("frame contrast uses raw values and rendering follows the datasheet column map", () => {
  const flat = new Uint8Array(484).fill(64);
  assert.deepEqual(frameMetrics(flat), { mean: 64, sd: 0, gradient: 0, dark: 0, bright: 0 });
  assert.equal(frameMetrics(new Uint8Array(484)).dark, 100);
  const pixels = new Uint8Array(484); pixels[0] = 127; pixels[462] = 64; pixels[21] = 32; pixels[483] = 16;
  const rgba = frameRgba(pixels);
  assert.deepEqual(Array.from(rgba.slice(21 * 4, 22 * 4)), [255, 255, 255, 255]);
  assert.equal(rgba[0], 129); assert.equal(rgba[(21 * 22 + 21) * 4], 64); assert.equal(rgba[21 * 22 * 4], 32);
  assert.ok(frameMetrics(pixels).sd > 0); assert.ok(frameMetrics(pixels).gradient > 0);
  assert.equal(frameRgba(flat, true)[0], 129);
  assert.throws(() => frameMetrics([1])); assert.throws(() => frameRgba(new Uint8Array(484).fill(255)));
});

test("BLE client subscribes before starting, sends capture, and stops cleanly", async () => {
  const writes = [], listeners = new Map(); let subscribed = false, seen = 0, disconnected = 0;
  const command = { async writeValueWithResponse(b) { assert.ok(subscribed); writes.push(Array.from(b)); } };
  const data = { addEventListener(k, fn) { listeners.set(k, fn); }, removeEventListener(k) { listeners.delete(k); }, async startNotifications() { subscribed = true; } };
  const device = { addEventListener(k, fn) { listeners.set(k, fn); }, removeEventListener(k) { listeners.delete(k); },
    gatt: { connected: false, async connect() { this.connected = true; return { async getPrimaryService(uuid) { assert.equal(uuid, bleService); return { async getCharacteristic(id) { return id.includes("8a01") ? command : data; } }; } }; }, disconnect() { this.connected = false; listeners.get("gattserverdisconnected")?.(); } } };
  const connection = new BleSensorConnection(() => seen++, () => disconnected++);
  await connection.connect({ async requestDevice(options) { assert.equal(options.filters[0].namePrefix, "ReeL"); assert.deepEqual(options.optionalServices, [bleService]); return device; } });
  listeners.get("characteristicvaluechanged")({ target: { value: new DataView(telemetry.buffer) } });
  await connection.setMode(2); await connection.close();
  assert.equal(seen, 1); assert.equal(disconnected, 0); assert.equal(device.gatt.connected, false);
  assert.deepEqual(writes, [[82, 84, 1, 1], [82, 84, 1, 2], [82, 84, 1, 0]]);
  assert.equal(listeners.size, 0); await assert.rejects(() => connection.setMode(2));
  const a = run(), b = run(); b.transport = "ble"; assert.equal(comparisonIssues(a, b).length, 1);
});
