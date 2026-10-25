export const directions = [
  { label: "右", symbol: "→" }, { label: "左", symbol: "←" },
  { label: "下", symbol: "↓" }, { label: "上", symbol: "↑" }
];
const protocols = { slow: "低速4方向", fast: "高速4方向", rest: "静止5秒", free: "自由5秒" };

export function quantile(values, fraction = 0.5) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const index = (sorted.length - 1) * fraction;
  const low = Math.floor(index);
  return sorted[low] + (sorted[Math.ceil(index)] - sorted[low]) * (index - low);
}

export function parseSensorLine(line) {
  const start = line.indexOf("RTB1,");
  if (start < 0 || (start > 0 && !/\s/.test(line[start - 1]))) return null;
  const fields = line.slice(start).trim().split(",");
  if (fields.length !== 8 || !fields.slice(1).every(value => /^-?\d+$/.test(value))) return null;
  const [seq, micros, dx, dy, squal, shutter, motion] = fields.slice(1).map(Number);
  const bounded = (value, min, max) => Number.isSafeInteger(value) && value >= min && value <= max;
  if (!bounded(seq, 0, 0xffffffff) || !bounded(micros, 0, Number.MAX_SAFE_INTEGER)
      || !bounded(dx, -2048, 2047) || !bounded(dy, -2048, 2047)
      || !bounded(squal, 0, 255) || !bounded(shutter, 0, 65535) || !bounded(motion, 0, 255)
      || (!(motion & 0x80) && (dx !== 0 || dy !== 0))) return null;
  return { seq, t: micros / 1000, dx, dy, squal, shutter, motion };
}

export class SensorLines {
  pending = "";
  skipping = false;
  invalid = 0;
  push(text) {
    const lines = (this.pending + text).split("\n");
    this.pending = lines.pop();
    const samples = [];
    for (const line of lines) {
      if (this.skipping) { this.skipping = false; continue; }
      const sample = line.length <= 1024 ? parseSensorLine(line) : null;
      if (sample) samples.push(sample);
      else if (line.includes("RTB1,") || line.length > 1024) this.invalid++;
    }
    if (this.pending.length > 1024) { this.pending = ""; this.skipping = true; this.invalid++; }
    return samples;
  }
}

export function summarizeStroke(samples, durationMs) {
  if (!Number.isFinite(durationMs) || durationMs <= 0) throw new Error("Invalid duration");
  let x = 0, y = 0, path = 0, maxRadius = 0, radiusSquared = 0, largestStep = 0;
  let previousTime = 0, lastInput = 0, gapMs = 0, lost = 0;
  const positions = [];
  for (const sample of samples) {
    if (![sample.t, sample.dx, sample.dy].every(Number.isFinite)
        || sample.t < previousTime || sample.t < 0 || sample.t > durationMs + 1) throw new Error("Invalid sample time");
    previousTime = sample.t;
    x += sample.dx; y += sample.dy;
    const step = Math.hypot(sample.dx, sample.dy);
    path += step; largestStep = Math.max(largestStep, step);
    const radius = Math.hypot(x, y);
    maxRadius = Math.max(maxRadius, radius); radiusSquared += radius * radius;
    if (step > 0) { gapMs = Math.max(gapMs, sample.t - lastInput); lastInput = sample.t; }
    lost += sample.lost || 0;
    positions.push([x, y]);
  }
  gapMs = Math.max(gapMs, durationMs - lastInput);
  const net = Math.hypot(x, y);
  let crossSquared = 0, backward = 0, along = 0;
  if (net > 0) {
    for (let i = 0; i < samples.length; i++) {
      const cross = (positions[i][0] * y - positions[i][1] * x) / net;
      crossSquared += cross * cross;
      const projection = (samples[i].dx * x + samples[i].dy * y) / net;
      along += Math.abs(projection); backward += Math.max(0, -projection);
    }
  }
  return {
    durationMs, sampleCount: samples.length, net, path, x, y, maxRadius, largestStep, lost,
    rms: samples.length ? Math.sqrt(radiusSquared / samples.length) : 0,
    crossRms: net && samples.length ? Math.sqrt(crossSquared / samples.length) : null,
    reversePercent: along ? backward / along * 100 : null,
    efficiency: path ? net / path * 100 : null, gapMs,
    squalP10: quantile(samples.map(s => s.squal), 0.1),
    squalMedian: quantile(samples.map(s => s.squal)),
    squalMin: quantile(samples.map(s => s.squal), 0),
    shutterMedian: quantile(samples.map(s => s.shutter)),
    shutterP90: quantile(samples.map(s => s.shutter), 0.9)
  };
}

export function summarizeRun(run) {
  const valid = run.strokes.filter(stroke => !stroke.invalidReason);
  const groups = directions.map((direction, index) => {
    const strokes = valid.filter(stroke => stroke.direction === index);
    const amounts = strokes.map(s => s.metrics.net);
    const mean = amounts.reduce((a, b) => a + b, 0) / amounts.length;
    const variance = amounts.length > 1 ? amounts.reduce((sum, n) => sum + (n - mean) ** 2, 0) / (amounts.length - 1) : null;
    return {
      label: direction.label, count: strokes.length, net: quantile(amounts),
      cv: mean > 0 && variance !== null ? Math.sqrt(variance) / mean * 100 : null,
      crossRms: quantile(strokes.map(s => s.metrics.crossRms)),
      reversePercent: quantile(strokes.map(s => s.metrics.reversePercent)),
      gapMs: quantile(strokes.map(s => s.metrics.gapMs), 1),
      squalP10: quantile(strokes.map(s => s.metrics.squalP10))
    };
  });
  const amounts = groups.filter(g => g.count === 3).map(g => g.net);
  const max = Math.max(...amounts);
  return {
    groups, validCount: valid.length,
    balance: amounts.length === 4 && max > 0 ? Math.min(...amounts) / max * 100 : null,
    net: quantile(valid.map(s => s.metrics.net)),
    rms: quantile(valid.map(s => s.metrics.rms)),
    crossRms: quantile(valid.map(s => s.metrics.crossRms)),
    reversePercent: quantile(valid.map(s => s.metrics.reversePercent)),
    gapMs: quantile(valid.map(s => s.metrics.gapMs), 1),
    squalP10: quantile(valid.map(s => s.metrics.squalP10)),
    shutterMedian: quantile(valid.map(s => s.metrics.shutterMedian)),
    lost: run.strokes.reduce((sum, s) => sum + s.metrics.lost, 0)
  };
}

export function comparisonIssues(a, b) {
  const issues = [];
  if (!a.complete || !b.complete) issues.push("未完了・通信欠落のある測定は比較できません");
  if (a.source === "sensor" && b.source === "sensor" && (a.transport || "usb") !== (b.transport || "usb")) issues.push("接続方式が異なります");
  if (a.source !== b.source || a.unadjusted !== b.unadjusted) issues.push("入力元が異なります");
  if (a.conditions.cpi !== b.conditions.cpi) issues.push("記録したCPIが異なります");
  if (a.conditions.travel !== b.conditions.travel) issues.push("回した量の目安が異なります");
  if (a.protocol !== b.protocol && ![a.protocol, b.protocol].every(p => p === "slow" || p === "fast")) issues.push("テストの種類が異なります");
  return issues;
}

export function csvCell(value) {
  if (typeof value === "number") return String(value);
  let text = String(value ?? "");
  if (/^[\s]*[=+@-]/.test(text)) text = "'" + text;
  return '"' + text.replaceAll('"', '""') + '"';
}

export function rawCsv(run) {
  const rows = [["condition", "ball", "position", "cpi_recorded", "travel_mm_recorded", "settings", "source", "transport", "protocol", "stroke", "direction", "t_ms", "seq", "dx", "dy", "squal", "shutter", "motion", "lost", "invalid_reason"]];
  run.strokes.forEach((stroke, index) => {
    for (const sample of stroke.samples || []) rows.push([
      run.conditions.name, run.conditions.ball, run.conditions.position, run.conditions.cpi, run.conditions.travel, run.conditions.note,
      run.source, run.transport || "", run.protocol, index + 1, directions[stroke.direction]?.label || "—",
      sample.t, sample.seq ?? "", sample.dx, sample.dy, sample.squal ?? "", sample.shutter ?? "", sample.motion ?? "", sample.lost || 0, stroke.invalidReason || ""
    ]);
  });
  return "\ufeff" + rows.map(row => row.map(csvCell).join(",")).join("\r\n");
}



export function frameMetrics(pixels) {
  if (pixels.length !== 484 || Array.from(pixels).some(p => !Number.isInteger(p) || p < 0 || p > 127)) throw new Error("Invalid 22x22 frame");
  const mean = pixels.reduce((sum, p) => sum + p, 0) / 484;
  const sd = Math.sqrt(pixels.reduce((sum, p) => sum + (p - mean) ** 2, 0) / 484);
  let gradient = 0, neighbors = 0;
  for (let y = 0; y < 22; y++) for (let x = 0; x < 22; x++) {
    const index = y * 22 + x;
    if (x < 21) { gradient += (pixels[index + 1] - pixels[index]) ** 2; neighbors++; }
    if (y < 21) { gradient += (pixels[index + 22] - pixels[index]) ** 2; neighbors++; }
  }
  return { mean, sd, gradient: Math.sqrt(gradient / neighbors),
    dark: pixels.filter(p => p <= 2).length / 484 * 100,
    bright: pixels.filter(p => p >= 125).length / 484 * 100 };
}


export function frameRgba(pixels, stretch = false) {
  frameMetrics(pixels);
  const low = stretch ? Math.min(...pixels) : 0, high = stretch ? Math.max(...pixels) : 127;
  const rgba = new Uint8ClampedArray(484 * 4);
  for (let y = 0; y < 22; y++) for (let x = 0; x < 22; x++) {
    // Datasheet Figure 17: first pixel at top-right, down columns, right to left.
    const value = pixels[(21 - x) * 22 + y];
    const level = high > low ? Math.round((value - low) / (high - low) * 255) : Math.round(value / 127 * 255);
    const i = (y * 22 + x) * 4; rgba[i] = rgba[i + 1] = rgba[i + 2] = level; rgba[i + 3] = 255;
  }
  return rgba;
}

export class SensorPackets {
  invalid = 0;
  frame = null;
  last = null;
  epoch = 0;
  push(value) {
    const bytes = value instanceof DataView ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : value;
    const bad = () => { this.invalid++; if (this.frame) this.frame.invalid = true; return null; };
    if (!(bytes instanceof Uint8Array) || bytes.length !== 20 || bytes[1] !== 1) return bad();
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes[0] === 84) {
      const seq = view.getUint32(2, true), micros = view.getUint32(6, true);
      const dx = view.getInt16(10, true), dy = view.getInt16(12, true), motion = bytes[17], cpi = view.getUint16(18, true);
      if (dx < -2048 || dx > 2047 || dy < -2048 || dy > 2047 || (!(motion & 128) && (dx || dy)) || cpi < 200 || cpi > 3200 || cpi % 200) return bad();
      if (this.last && ((seq - this.last.seq) >>> 0) > 0x80000000) this.epoch = 0;
      else if (this.last && micros < this.last.micros && this.last.micros - micros > 0x80000000) this.epoch += 2 ** 32;
      this.last = { seq, micros };
      return { kind: "sample", sample: { seq, t: (this.epoch + micros) / 1000, dx, dy, squal: bytes[14], shutter: view.getUint16(15, true), motion, cpi } };
    }
    const id = view.getUint16(2, true);
    if (bytes[0] === 83) {
      if (bytes[4] === 1) {
        this.frame = { id, pixels: new Uint8Array(484), seen: new Set(), invalid: false };
        return { kind: "start", id };
      }
      if (bytes[4] === 2 || bytes[4] === 3) {
        const frame = this.frame; this.frame = null;
        if (bytes[4] === 3) return { kind: "error", message: "センサーの撮影・追跡復帰に失敗しました。電源を入れ直して再接続してください。" };
        if (!frame || frame.id !== id || frame.invalid || frame.seen.size !== 35) return { kind: "error", message: "画像データが欠けています。このフレームは表示・保存しません。" };
        return { kind: "frame", id, pixels: frame.pixels };
      }
      return bad();
    }
    if (bytes[0] === 70) {
      const offset = view.getUint16(4, true), frame = this.frame;
      if (!frame || frame.id !== id || offset >= 484 || offset % 14 || frame.seen.has(offset)) return bad();
      const count = Math.min(14, 484 - offset);
      if (bytes.slice(6, 6 + count).some(p => p > 127) || bytes.slice(6 + count).some(p => p !== 0)) return bad();
      frame.pixels.set(bytes.slice(6, 6 + count), offset); frame.seen.add(offset);
      return { kind: "progress", count: Math.min(484, frame.seen.size * 14) };
    }
    return bad();
  }
}

export function decodeUsbReport(value, tag) {
  const bytes = value instanceof DataView ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : value;
  if (!(bytes instanceof Uint8Array) || bytes.length !== 64 || bytes[0] !== 0xd2 || bytes[1] !== tag
      || (bytes[2] & 0x7f) !== 3 || bytes[3] > 3) throw new Error("USB診断応答が不正です");
  if (bytes.slice(4 + bytes[3] * 20).some(b => b !== 0)) throw new Error("USB診断応答の長さが不正です");
  return { overflow: !!(bytes[2] & 128), packets: Array.from({ length: bytes[3] }, (_, i) => bytes.slice(4 + i * 20, 24 + i * 20)) };
}

export class UsbSensorConnection {
  constructor(onPacket, onDisconnect, onOverflow = () => {}) {
    this.onPacket = onPacket; this.onDisconnect = onDisconnect; this.onOverflow = onOverflow;
    this.queue = Promise.resolve(); this.tag = 0; this.running = false; this.pending = null;
  }
  async connect(hid) {
    this.hid = hid;
    const devices = await hid.requestDevice({ filters: [{ vendorId: 0x4c4b, productId: 0x524d, usagePage: 0xff62, usage: 0x61 }] });
    if (!devices.length) throw new DOMException("選択をキャンセルしました", "NotFoundError");
    this.device = devices.find(d => d.collections.some(c => c.usagePage === 0xff62 && c.usage === 0x61));
    if (!this.device) throw new Error("USB診断対応のReeLが選ばれていません。ページ内の修正版UF2を書き込み、USBを挿し直してください。");
    this.input = event => {
      if (event.device !== this.device || event.reportId !== 0 || !this.pending) return;
      const bytes = new Uint8Array(event.data.buffer, event.data.byteOffset, event.data.byteLength);
      if (bytes.length < 2 || bytes[0] !== 0xd2 || bytes[1] !== this.pending.tag) return;
      const pending = this.pending; this.pending = null; clearTimeout(pending.timer);
      try { pending.resolve(decodeUsbReport(bytes, pending.tag)); } catch (error) { pending.reject(error); }
    };
    this.removed = event => { if (event.device === this.device) this.fail(new Error("ReeLのUSB接続が切れました")); };
    try {
      if (!this.device.opened) await this.device.open();
      this.device.addEventListener("inputreport", this.input); hid.addEventListener("disconnect", this.removed);
      await this.exchange(0);
      this.running = true; this.pollTask = this.poll();
    } catch (error) { await this.close(); throw error; }
  }
  exchange(command) {
    const task = this.queue.then(() => this.request(command)); this.queue = task.catch(() => {}); return task;
  }
  request(command) {
    if (!this.device?.opened || ![0,1,2,3].includes(command)) return Promise.reject(new Error("USBが接続されていません"));
    const tag = this.tag = this.tag % 255 + 1;
    const packet = new Uint8Array(64); packet.set([82,84,2,command,tag]);
    return new Promise((resolve,reject) => {
      const timer = setTimeout(() => {
        if (this.pending?.tag !== tag) return;
        this.pending = null; reject(new Error("ReeLからUSB診断の応答がありません。修正版UF2とUSB接続を確認してください。"));
      }, 1500);
      this.pending = { tag, resolve, reject, timer };
      Promise.resolve().then(() => this.device.sendReport(0,packet)).catch(error => {
        if (this.pending?.tag !== tag) return;
        clearTimeout(timer); this.pending = null; reject(error);
      });
    });
  }
  async poll() {
    try {
      while (this.running) {
        const reply = await this.exchange(1);
        if (!this.running) break;
        if (reply.overflow) this.onOverflow();
        for (const packet of reply.packets) this.onPacket(packet);
        // Drain queued records immediately; only idle polls need a delay.
        if (!reply.packets.length) await new Promise(resolve => setTimeout(resolve,4));
      }
    } catch (error) { if (this.running) this.fail(error); }
  }
  async capture() { await this.exchange(2); }
  fail(error) {
    this.running = false;
    if (this.pending) { clearTimeout(this.pending.timer); this.pending.reject(error); this.pending = null; }
    this.device?.removeEventListener("inputreport",this.input); this.hid?.removeEventListener("disconnect",this.removed);
    this.onDisconnect(error);
    this.device?.close().catch(() => {});
  }
  async close() {
    this.running = false;
    await this.pollTask;
    if (this.device?.opened) { await this.exchange(3).catch(() => {}); await this.device.close().catch(() => {}); }
    this.device?.removeEventListener("inputreport",this.input); this.hid?.removeEventListener("disconnect",this.removed);
  }
}

if (typeof document !== "undefined") initialize();

function initialize() {
  const $ = id => document.getElementById(id);
  const storageKey = "reel-trackball-lab-v1";
  let history = [], storageHealthy = true;
  const state = { source: null, live: [], previous: null, arrival: 0, lost: 0, invalid: 0, unadjusted: null,
    hid: null, transport: null, lastInvalid: 0, demoTimer: null, session: null, capture: null, run: null, preparing: false, breakNext: true };
  const camera = { frame: null, reference: null, working: false, continuous: false, timeout: null, next: null, demo: null, started: 0, requestConditions: null, waitingForSample: false, completedTimes: [] };
  const fmt = (value, suffix = "", digits = 1) => Number.isFinite(value) ? value.toFixed(digits) + suffix : "—";
  const sourceName = source => ({ sensor: "センサー直結", browser: "ブラウザ入力", demo: "デモ・合成データ" }[source] || "未接続");
  const notice = message => { $("notice").textContent = message; $("notice").hidden = !message; };
  const fresh = () => state.source !== "sensor" || (state.arrival > 0 && performance.now() - state.arrival < 750);
  const row = values => { const tr = document.createElement("tr"); for (const value of values) { const td = document.createElement("td"); td.textContent = value; tr.append(td); } return tr; };
  const download = (text, filename, type) => {
    const url = URL.createObjectURL(new Blob([text], { type }));
    const link = document.createElement("a"); link.href = url; link.download = filename; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  function exportRun(run) { download(JSON.stringify({ schema: 1, run }, null, 2), "reel-trackball-result.json", "application/json"); }
  try {
    const saved = localStorage.getItem(storageKey);
    if (saved) {
      history = JSON.parse(saved);
      if (!Array.isArray(history) || history.some(run => !run || typeof run.id !== "string" || !run.conditions
          || typeof run.conditions.name !== "string" || !Array.isArray(run.strokes) || !(run.protocol in protocols)
          || !["sensor", "browser", "demo"].includes(run.source))) throw new Error("Invalid saved history");
    }
  } catch { history = []; storageHealthy = false; notice("保存履歴を読み込めませんでした。既存の保存データは上書きしません。今回の測定はJSONで保存できます。"); }

  function sourceStatus(message, kind = "") {
    $("connectionStatus").textContent = message;
    $("statusDot").className = "dot " + kind;
    $("prepare").disabled = !!state.session || state.preparing || !state.source || (state.source === "sensor" ? !state.hid?.device?.opened : !fresh());
    $("coordinateLabel").textContent = state.source === "sensor" ? "補正前のセンサー counts" : state.source === "demo" ? "合成データ / counts" : state.source === "browser" ? "PC側の移動単位 u" : "入力待ち";
  }
  function resetInput(source) {
    state.source = source; state.preparing = false; state.breakNext = true; state.live = []; state.previous = null; state.arrival = 0; state.lost = 0; state.invalid = 0; state.lastInvalid = 0; state.unadjusted = null;
    $("connectSensor").textContent = state.hid ? "USBを切断" : "USBで接続";
    $("sourceDetail").textContent = source === "sensor" ? "USB HIDで補正前のΔX/ΔY・SQUAL・露光と22×22の実画像を受信します。"
      : source === "browser" ? "PC側の出力を測ります。SQUAL・露光は取得できません。測定中は対象のトラックボールだけを動かしてください。"
      : source === "demo" ? "画面と測定手順を試すための合成データです。実機の評価には使えません。"
      : "USBでセンサーの生データ・実画像を受信できます。";
    $("prompt").textContent = source ? "測定を準備してください。" : "USBで接続してください。";
    renderCamera();
    sourceStatus(source === "sensor" ? "接続済み · センサーデータ待ち" : sourceName(source), source === "demo" ? "demo" : "");
  }
  function ingest(sample) {
    if (state.source === "demo" && camera.working) return;
    if (state.source === "sensor") {
      if (state.previous) {
        const advance = (sample.seq - state.previous.seq) >>> 0;
        if (advance === 0) return;
        if (sample.t < state.previous.t || advance > 0x80000000) {
          stopSession("センサーが再起動しました。測定をやり直してください。");
          state.live = []; state.previous = null;
        } else sample.lost = advance - 1;
      }
      state.previous = sample;
    }
    sample.breakBefore = state.breakNext; state.breakNext = false;
    state.arrival = performance.now(); state.lost += sample.lost || 0;
    state.live.push(sample);
    if (state.live.length > 3000) state.live = state.live.slice(-2500);
    if (state.capture && sample.t >= state.capture.start) state.capture.samples.push({ ...sample, t: sample.t - state.capture.start });
    if (camera.waitingForSample && camera.continuous && !camera.working && !state.session && !state.preparing) {
      camera.waitingForSample = false;
      camera.next = setTimeout(() => { if (camera.continuous) requestFrame(); }, 0);
    }
  }
  async function disconnect() {
    stopSession("入力を切り替えたため中断しました。");
    clearInterval(state.demoTimer); state.demoTimer = null;
    stopCamera(); camera.frame = null; camera.reference = null; camera.completedTimes = []; $("imageRate").textContent = "— fps";
    paintFrame("sensorImage", null); paintFrame("referenceImage", null); renderFrameMetrics();
    if (state.hid) { const hid = state.hid; state.hid = null; await hid.close(); }
    state.transport = null; resetInput(null);
  }
  $("connectSensor").addEventListener("click", async () => {
    if (state.hid) { await disconnect(); return; }
    if (!navigator.hid || !window.isSecureContext) { notice("USB接続はChrome / Edgeでこのページを直接開いてください。"); return; }
    $("connectSensor").disabled = true;
    try {
      await disconnect();
      const parser = new SensorPackets();
      const hid = new UsbSensorConnection(value => {
        if (state.hid !== hid) return;
        const previousInvalid = parser.invalid;
        const event = parser.push(value);
        if (parser.invalid > previousInvalid) { state.invalid += parser.invalid - previousInvalid; state.lastInvalid = performance.now(); }
        if (!event) return;
        if (event.kind === "sample") {
          if (state.live.at(-1)?.cpi && state.live.at(-1).cpi !== event.sample.cpi) stopSession("CPIが変わったため測定を中断しました。");
          ingest(event.sample);
        } else if (event.kind === "start") {
          stopSession("撮影中は移動量を測定できません。"); camera.working = true; state.previous = null; state.breakNext = true;
          $("cameraStatus").textContent = "撮影中";
        } else if (event.kind === "progress") $("cameraStatus").textContent = '受信 ' + event.count + ' / 484 px';
        else if (event.kind === "frame") finishFrame(event.pixels,event.id);
        else if (event.kind === "error") { stopCamera(); notice(event.message); $("cameraStatus").textContent = "撮影失敗 · 再撮影してください"; }
      }, error => {
        if (state.hid !== hid) return;
        state.hid = null; state.arrival = 0; stopCamera(); stopSession("USBが切断されたため中断しました。");
        notice(error.message); sourceStatus("USB切断 · 再接続してください", "error");
        $("connectSensor").textContent = "USBで接続"; renderCamera();
      }, () => { state.invalid++; state.lastInvalid = performance.now(); });
      state.hid = hid; state.transport = "usb-hid"; resetInput("sensor");
      await hid.connect(navigator.hid);
      $("usbFirmwareNote").hidden = true;
      notice(""); setCameraLive(false);
    } catch (error) { await disconnect(); if (error.name !== "NotFoundError") notice("USB接続できませんでした: " + error.message); }
    finally { $("connectSensor").disabled = false; renderCamera(); }
  });
  $("browserInput").addEventListener("click", async () => {
    await disconnect(); resetInput("browser"); notice("");
    $("prompt").textContent = "測定を準備してください。";
  });
  $("demoInput").addEventListener("click", async () => {
    await disconnect(); resetInput("demo"); notice("");
    let seq = 0; let t = 0;
    state.demoTimer = setInterval(() => {
      for (let i = 0; i < 4; i++) {
        seq++; t += 4;
        const mode = state.session?.protocol || "slow";
        const direction = state.session ? Math.floor(state.run.strokes.length / 3) : Math.floor(t / 1500) % 4;
        const sign = direction % 2 ? -1 : 1;
        const amount = mode === "rest" ? (seq % 41 === 0 ? 1 : 0) : (mode === "fast" ? 12 : 3) + Math.round(Math.sin(seq / 7));
        const dx = direction < 2 ? sign * amount : Math.round(Math.sin(seq / 5));
        const dy = direction < 2 ? (mode === "rest" ? 0 : Math.round(Math.sin(seq / 5))) : sign * amount;
        ingest({ seq, t, dx, dy, squal: Math.round(38 + 8 * Math.sin(seq / 35)), shutter: Math.round(120 + 40 * Math.sin(seq / 42)), motion: dx || dy ? 128 : 0 });
      }
    }, 16);
    setCameraLive(true);
  });


  function cameraConditions() {
    return { name: $("conditionName").value.trim(), ball: $("ball").value.trim(), position: $("position").value.trim(),
      cpi: state.transport === "usb-hid" ? state.live.at(-1)?.cpi ?? null : Number($("cpi").value), note: $("settingNote").value.trim() };
  }
  function paintFrame(id, frame, stretch = $("stretchImage").checked) {
    const target = $(id), context = target.getContext("2d");
    context.imageSmoothingEnabled = false;
    context.fillStyle = "#0e2027"; context.fillRect(0, 0, target.width, target.height);
    if (!frame) { context.fillStyle = "#7b979f"; context.font = "14px Consolas, monospace"; context.fillText("NO FRAME", 25, 40); return; }
    const image = document.createElement("canvas"); image.width = image.height = 22;
    image.getContext("2d").putImageData(new ImageData(frameRgba(frame.pixels, stretch), 22, 22), 0, 0);
    context.drawImage(image, 0, 0, target.width, target.height);
  }
  function renderCamera() {
    $("prepare").disabled = !!state.session || state.preparing || !state.source || (state.source === "sensor" ? !state.hid?.device?.opened : !fresh());
    const ready = (state.transport === "usb-hid" && state.hid?.device?.opened) || state.source === "demo";
    $("snapshot").disabled = !ready || !!state.session || camera.working || camera.continuous;
    $("continuous").disabled = !ready || !!state.session || (camera.working && !camera.continuous);
    $("continuous").textContent = camera.continuous ? "ライブ停止" : "ライブ開始";
    $("referenceFrame").disabled = !camera.frame;
    $("exportFrame").disabled = !camera.frame; $("exportPng").disabled = !camera.frame;
    $("imageKind").textContent = state.source === "demo" ? "DEMO / 22×22" : "22×22 / 7 bit";
    $("trackingMode").textContent = camera.continuous || camera.working ? "撮影中 · Escで停止" : "連続追跡";
    if (!camera.frame && !camera.working) $("cameraStatus").textContent = "画像待ち";
  }
  function stopCamera() {
    clearTimeout(camera.timeout); clearTimeout(camera.next); clearTimeout(camera.demo);
    camera.timeout = camera.next = null; camera.working = false; camera.continuous = false; camera.waitingForSample = false;
    renderCamera();
  }
  async function requestFrame() {
    if (camera.working || state.session || state.preparing || !(state.hid?.device?.opened || state.source === "demo")) return;
    camera.working = true; camera.started = performance.now(); camera.requestConditions = cameraConditions();
    state.previous = null; state.breakNext = true;
    $("cameraStatus").textContent = "撮影中"; renderCamera();
    camera.timeout = setTimeout(() => {
      stopCamera(); $("cameraStatus").textContent = "撮影タイムアウト · 最後の画像を保持";
      notice("画像が届きませんでした。診断ファームとUSB接続を確認してください。");
    }, 8000);
    try {
      if (state.source === "demo") {
        const phase = performance.now() / 700;
        const pixels = Uint8Array.from({ length: 484 }, (_, i) => Math.round(64 + 28 * Math.sin(i % 22 / 2 + phase) * Math.cos(Math.floor(i / 22) / 2 - phase)));
        camera.demo = setTimeout(() => finishFrame(pixels, 0), 200);
      } else await state.hid.capture();
    } catch (error) { stopCamera(); notice("撮影を開始できませんでした: " + error.message); }
  }
  function finishFrame(pixels, id) {
    clearTimeout(camera.timeout); camera.timeout = null; camera.demo = null;
    camera.working = false; state.previous = null; state.breakNext = true;
    const completed = performance.now();
    camera.completedTimes.push(completed); camera.completedTimes = camera.completedTimes.filter(t => t >= completed - 3000);
    camera.frame = { id, date: new Date().toISOString(), source: state.source === "demo" ? "demo" : "sensor-usb",
      captureMs: performance.now() - camera.started, conditions: camera.requestConditions || cameraConditions(),
      pixels: Array.from(pixels), metrics: frameMetrics(pixels) };
    const times = camera.completedTimes;
    const fps = times.length > 1 ? (times.length - 1) * 1000 / (times.at(-1) - times[0]) : null;
    $("imageRate").textContent = fmt(fps, " fps", 1) + " / " + fmt(camera.frame.captureMs, " ms", 0);
    $("cameraStatus").textContent = "追跡復帰";
    paintFrame("sensorImage", camera.frame); renderFrameMetrics(); renderCamera();
    camera.waitingForSample = camera.continuous;
  }
  function renderFrameMetrics() {
    const b = camera.frame, a = camera.reference;
    const comparable = a && b && a.source === b.source && a.conditions.cpi === b.conditions.cpi;
    const labels = [["明るさ 平均", "mean", ""], ["コントラスト σ", "sd", ""], ["隣接画素差 RMS", "gradient", ""], ["暗い画素 ≤2", "dark", "%"], ["明るい画素 ≥125", "bright", "%"]];
    $("imageMetrics").replaceChildren(...labels.map(([label, key, suffix]) => row([label, a ? fmt(a.metrics[key], suffix, 2) : "—", b ? fmt(b.metrics[key], suffix, 2) : "—", comparable ? fmt(b.metrics[key] - a.metrics[key], suffix, 2) : "—"])));
    $("frameLabel").textContent = b ? b.conditions.name + ' · ' + b.conditions.ball + ' · ' + b.conditions.position : "撮影待ち";
    $("referenceLabel").textContent = a ? a.conditions.name + ' · ' + a.conditions.ball + ' · ' + a.conditions.position : "—";
    const context = $("imageHistogram").getContext("2d"); context.clearRect(0, 0, 512, 100);
    if (b) {
      const bins = new Array(128).fill(0); for (const pixel of b.pixels) bins[pixel]++;
      const max = Math.max(1, ...bins); context.fillStyle = "#087d77";
      bins.forEach((count, i) => context.fillRect(i * 4, 100 - count / max * 96, 3, count / max * 96));
    }
  }
  $("snapshot").addEventListener("click", requestFrame);
  function setCameraLive(enabled) {
    camera.continuous = enabled; camera.waitingForSample = enabled; clearTimeout(camera.next);
    if (enabled && !camera.working && fresh() && state.live.length) requestFrame();
    renderCamera();
  }
  $("continuous").addEventListener("click", () => setCameraLive(!camera.continuous));
  $("referenceFrame").addEventListener("click", () => { camera.reference = camera.frame; paintFrame("referenceImage", camera.reference); renderFrameMetrics(); });
  $("stretchImage").addEventListener("change", () => { paintFrame("sensorImage", camera.frame); paintFrame("referenceImage", camera.reference); });
  $("exportFrame").addEventListener("click", () => {
    download(JSON.stringify({ schema: "reel-sensor-frame-v1", width: 22, height: 22, bits: 7,
      pixelOrder: "columns-top-to-bottom-right-to-left", frame: camera.frame, reference: camera.reference }, null, 2), "reel-sensor-frame.json", "application/json");
  });
  $("exportPng").addEventListener("click", () => {
    const image = document.createElement("canvas"); image.width = image.height = 22;
    image.getContext("2d").putImageData(new ImageData(frameRgba(camera.frame.pixels), 22, 22), 0, 0);
    const filename = camera.frame.source === "demo" ? "reel-sensor-demo.png" : "reel-sensor-22x22.png";
    image.toBlob(blob => { if (blob) download(blob, filename, "image/png"); else notice("PNGを生成できませんでした。"); }, "image/png");
  });
  document.addEventListener("visibilitychange", () => { if (document.hidden) setCameraLive(false); });
  paintFrame("sensorImage", null); paintFrame("referenceImage", null);

  function lockControls(locked) {
    for (const field of $("conditions").elements) field.disabled = locked;
    renderCamera();
    $("protocol").disabled = locked; $("prepare").disabled = locked; $("cancel").disabled = !locked; $("stroke").disabled = !locked;
  }
  function promptStep() {
    if (!state.session) return;
    const step = state.run.strokes.length;
    const directional = ["slow", "fast"].includes(state.session.protocol);
    $("progress").textContent = `${step} / ${directional ? 12 : 1}`;
    $("directionSymbol").textContent = directional ? directions[Math.floor(step / 3)].symbol : state.session.protocol === "rest" ? "○" : "↔";
    $("prompt").textContent = directional ? `${directions[Math.floor(step / 3)].label}へ ${(step % 3) + 1} / 3回目 · Spaceを押したまま1回回し、離してから指を戻す`
      : state.session.protocol === "rest" ? "Spaceで開始。その後は手を離して5秒間静止してください。" : "Spaceで開始。5秒間、自由に動かしてください。";
    $("stroke").disabled = false; $("stroke").textContent = directional ? "1回測る" : "5秒測る";
    $("captureBar").firstElementChild.style.width = "0%";
  }
  $("prepare").addEventListener("click", async () => {
    if (state.session || state.preparing || !state.source || !$("conditions").reportValidity()) return;
    const input = state.source, hid = state.hid;
    state.preparing = true; setCameraLive(false);
    const deadline = performance.now() + 2500;
    while (camera.working && performance.now() < deadline && state.preparing) await new Promise(resolve => setTimeout(resolve, 10));
    if (input === "sensor") {
      state.arrival = 0;
      while (!fresh() && performance.now() < deadline && state.preparing) await new Promise(resolve => setTimeout(resolve, 10));
    }
    if (!state.preparing || state.source !== input || state.hid !== hid) return;
    state.preparing = false;
    if (camera.working || !fresh()) { notice("追跡データが復帰していません。USBを接続し直してください。"); renderCamera(); return; }
    const previousRun = state.run;
    state.run = {
      id: crypto.randomUUID(), date: new Date().toISOString(), source: state.source, transport: state.transport, protocol: $("protocol").value,
      unadjusted: null, complete: false, strokes: [],
      conditions: { name: $("conditionName").value.trim(), ball: $("ball").value.trim(), position: $("position").value.trim(), cpi: Number($("cpi").value), travel: Number($("travel").value), note: $("settingNote").value.trim() }
    };
    if (state.transport === "usb-hid" && state.live.at(-1)?.cpi) state.run.conditions.cpi = state.live.at(-1).cpi;
    state.session = { protocol: state.run.protocol }; lockControls(true);
    try {
      if (state.source === "browser") {
          try {
            const request = $("captureArea").requestPointerLock({ unadjustedMovement: true });
            state.unadjusted = !!request?.then; await request;
          } catch (error) {
            if (error.name !== "NotSupportedError") throw error;
            state.unadjusted = false; await $("captureArea").requestPointerLock();
          }
          state.run.unadjusted = state.unadjusted;
          notice(state.unadjusted ? "ブラウザ入力を計測します。加速なしの入力を要求済みです。センサー内部値ではありません。" : "加速なしの入力を確認できません。この条件を結果に記録します。同じ環境で比較してください。");
        if (document.pointerLockElement !== $("captureArea")) throw new Error("Pointer Lockを利用できません");
      }
      $("captureArea").focus(); promptStep(); renderResults();
    } catch {
      state.session = null; state.run = previousRun; lockControls(false); renderResults();
      notice("ポインタを固定できませんでした。Chrome / EdgeでこのページのURLを直接開き、もう一度準備してください。");
    }
  });
  function beginCapture() {
    if (!state.session || state.capture || !fresh()) return;
    const first = state.live.at(-1);
    const fixed = !["slow", "fast"].includes(state.session.protocol);
    state.capture = { start: state.source === "browser" ? performance.now() : first?.t || 0, wallStart: performance.now(), samples: [], invalidAtStart: state.invalid };
    state.capture.timer = setTimeout(finishCapture, fixed ? 5000 : 15000);
    $("stroke").disabled = fixed; $("stroke").textContent = fixed ? "計測中…" : "この1回を終了";
    $("prompt").textContent = fixed ? (state.session.protocol === "rest" ? "手を離して静止 · 5秒で自動終了" : "自由に操作 · 5秒で自動終了") : "同じ量を回す → 終了ボタン / Spaceを離す";
  }
  function finishCapture() {
    if (!state.capture || !state.session) return;
    const capture = state.capture; clearTimeout(capture.timer); state.capture = null;
    const duration = state.source === "browser" ? performance.now() - capture.start : (state.live.at(-1)?.t || capture.start) - capture.start;
    let invalidReason = "";
    if (duration < 30) { promptStep(); notice("計測時間が短すぎます。Spaceを押したまま、1回動かしてください。"); return; }
    const metrics = summarizeStroke(capture.samples, duration);
    if (state.source === "sensor" && (!capture.samples.length || !fresh())) invalidReason = "センサーデータが届いていません";
    if (metrics.lost || state.invalid > capture.invalidAtStart) invalidReason = "通信欠落・不正なログがあり比較対象外";
    const wallDuration = performance.now() - capture.wallStart;
    if (state.source !== "browser" && Math.abs(duration - wallDuration) > Math.max(60, wallDuration * 0.25)) invalidReason = "データの時間範囲が操作時間と一致しません";
    state.run.strokes.push({ direction: ["slow", "fast"].includes(state.session.protocol) ? Math.floor(state.run.strokes.length / 3) : -1, metrics, invalidReason, samples: capture.samples });
    renderResults();
    const expected = ["slow", "fast"].includes(state.session.protocol) ? 12 : 1;
    if (state.run.strokes.length === expected) {
      state.run.complete = state.run.strokes.every(s => !s.invalidReason);
      state.run.reason = state.run.complete ? "" : "通信品質の問題があるため比較対象外です。";
      finishSession(); saveHistory();
    } else promptStep();
  }
  function finishSession() {
    state.session = null; state.capture = null; lockControls(false); $("stroke").textContent = "1回測る";
    $("progress").textContent = state.run.complete ? "測定完了" : "未完了・比較対象外";
    $("prompt").textContent = state.run.complete ? "測定完了" : state.run.reason;
    if (document.pointerLockElement === $("captureArea")) document.exitPointerLock();
    sourceStatus(sourceName(state.source), state.source === "demo" ? "demo" : ""); renderResults();
  }
  function stopSession(reason) {
    if (!state.session) return;
    if (state.capture) {
      const capture = state.capture; clearTimeout(capture.timer); state.capture = null;
      const duration = state.source === "browser" ? performance.now() - capture.start : (state.live.at(-1)?.t || capture.start) - capture.start;
      if (duration > 0) state.run.strokes.push({ direction: ["slow", "fast"].includes(state.session.protocol) ? Math.floor(state.run.strokes.length / 3) : -1,
        metrics: summarizeStroke(capture.samples, duration), invalidReason: reason, samples: capture.samples });
    }
    state.run.complete = false; state.run.reason = reason; finishSession(); notice(reason);
  }
  $("stroke").addEventListener("click", () => state.capture ? finishCapture() : beginCapture());
  $("cancel").addEventListener("click", () => stopSession("測定を中断しました。途中のデータはCSV・JSONで保存できます。"));
  document.addEventListener("keydown", event => {
    if (event.code === "Escape" && !state.session) { setCameraLive(false); return; }
    if (!state.session || /^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)) return;
    if (event.code === "Space") { event.preventDefault(); if (!event.repeat) beginCapture(); }
    if (event.code === "Escape") stopSession("Escで測定を終了しました。未完了の結果は比較に使いません。");
    if (event.code === "KeyR" && !event.repeat) {
      if (state.capture) { clearTimeout(state.capture.timer); state.capture = null; }
      else state.run.strokes.pop();
      promptStep(); renderResults();
    }
  });
  document.addEventListener("keyup", event => {
    if (event.code === "Space" && state.session && ["slow", "fast"].includes(state.session.protocol)) { event.preventDefault(); finishCapture(); }
  });
  document.addEventListener("mousemove", event => {
    if (state.source !== "browser" || document.pointerLockElement !== $("captureArea") || !event.isTrusted) return;
    const dx = event.movementX, dy = event.movementY;
    if (Number.isFinite(dx) && Number.isFinite(dy) && (dx || dy)) ingest({ t: performance.now(), dx, dy });
  });
  document.addEventListener("pointerlockchange", () => { if (state.session && state.source === "browser" && document.pointerLockElement !== $("captureArea")) stopSession("ポインタの固定が解除されました。未完了の結果は比較に使いません。"); });
  window.addEventListener("blur", () => stopSession("画面のフォーカスが外れたため中断しました。"));

  function saveHistory() {
    const run = { ...state.run, strokes: state.run.strokes.map(({ samples, ...stroke }) => stroke) };
    history.push(run);
    if (storageHealthy) {
      try { localStorage.setItem(storageKey, JSON.stringify(history)); }
      catch { notice("ブラウザへの履歴保存に失敗しました。画面の結果は残っています。JSONを保存してください。"); }
    }
    renderHistory();
  }
  function renderResults() {
    const run = state.run; if (!run) return;
    const summary = summarizeRun(run); const unit = run.source === "browser" ? " u" : " counts";
    $("resultStatus").textContent = `${sourceName(run.source)} · ${protocols[run.protocol]} · ${summary.validCount}回`;
    $("resultUnit").textContent = unit.trim();
    const cards = run.protocol === "rest" ? [[fmt(summary.rms, unit), "静止偏位 RMS"], [fmt(summary.net, unit), "静止中の正味移動"], [fmt(summary.squalP10, "", 0), "SQUAL P10"], [fmt(summary.shutterMedian, "", 0), "露光 中央値"]]
      : [[fmt(summary.balance, "%"), "4方向の最小 / 最大"], [fmt(summary.crossRms, unit), "横ぶれ RMS 中央値"], [fmt(summary.reversePercent, "%"), "逆走率 中央値"], [fmt(summary.squalP10, "", 0), "SQUAL P10"]];
    $("resultCards").replaceChildren(...cards.map(([value, label]) => { const div = document.createElement("div"); div.className = "summary"; const strong = document.createElement("strong"); strong.textContent = value; const span = document.createElement("span"); span.textContent = label; div.append(strong, span); return div; }));
    $("directionResults").replaceChildren(...summary.groups.map(g => row([g.label, g.count + " / 3", fmt(g.net), fmt(g.cv), fmt(g.crossRms), fmt(g.reversePercent), fmt(g.gapMs, "", 0), fmt(g.squalP10, "", 0)])));
    if (["rest", "free"].includes(run.protocol)) $("directionResults").replaceChildren(row([protocols[run.protocol], summary.validCount + " / 1", fmt(summary.net), "—", fmt(summary.crossRms), fmt(summary.reversePercent), fmt(summary.gapMs, "", 0), fmt(summary.squalP10, "", 0)]));
    $("resultExplanation").textContent = run.reason || (summary.lost ? "通信欠落のある測定は比較対象外です。USB接続を確認して再計測してください。" : "");
    $("exportRaw").disabled = !run.strokes.length; $("exportJson").disabled = !run.strokes.length;
  }
  $("exportRaw").addEventListener("click", () => download(rawCsv(state.run), "reel-trackball-raw.csv", "text/csv;charset=utf-8"));
  $("exportJson").addEventListener("click", () => exportRun(state.run));
  function renderHistory() {
    const previousA = $("compareA").value, previousB = $("compareB").value;
    const eligible = history.filter(run => run.complete);
    for (const id of ["compareA", "compareB"]) {
      $(id).replaceChildren(...eligible.map(run => { const option = document.createElement("option"); option.value = run.id; option.textContent = `${run.conditions.name} · ${protocols[run.protocol]} · ${sourceName(run.source)} · ${new Date(run.date).toLocaleTimeString("ja-JP")}`; return option; }));
    }
    if (eligible.some(run => run.id === previousA)) $("compareA").value = previousA;
    else if (eligible.length > 1) $("compareA").value = eligible.at(-2).id;
    if (eligible.some(run => run.id === previousB) && previousB !== previousA) $("compareB").value = previousB;
    if (eligible.length) $("compareB").value = eligible.at(-1).id;
    $("history").replaceChildren(...history.slice().reverse().map(run => {
      const div = document.createElement("div"); div.className = "history-item";
      const text = document.createElement("div"); text.textContent = `${run.conditions.name} · ${protocols[run.protocol]} · ${run.complete ? "完了" : "比較対象外"}`;
      const span = document.createElement("span"); span.textContent = ` ${sourceName(run.source)} / ${run.conditions.ball} / ${run.conditions.position} / ${run.conditions.cpi} CPI（記録）`;
      text.append(document.createElement("br"), span);
      const button = document.createElement("button"); button.type = "button"; button.textContent = "集計JSON"; button.addEventListener("click", () => exportRun(run));
      div.append(text, button); return div;
    }));
    renderComparison();
  }
  function renderComparison() {
    $("comparisonNote").textContent = "";
    const a = history.find(run => run.id === $("compareA").value), b = history.find(run => run.id === $("compareB").value);
    if (!a || !b || a.id === b.id) { $("comparisonResults").replaceChildren(row(["測定2件を選択", "—", "—", "—"])); return; }
    const issues = comparisonIssues(a, b);
    if (issues.length) { $("comparisonNote").textContent = "比較を停止: " + issues.join(" / "); $("comparisonResults").replaceChildren(row(["条件不一致", "—", "—", "—"])); return; }
    const sa = summarizeRun(a), sb = summarizeRun(b); const unit = a.source === "browser" ? " u" : " counts";
    $("comparisonUnit").textContent = unit.trim();
    const pairs = [["SQUAL P10", sa.squalP10, sb.squalP10, ""], ["露光 中央値", sa.shutterMedian, sb.shutterMedian, ""], ["横ぶれ RMS", sa.crossRms, sb.crossRms, unit], ["逆走率", sa.reversePercent, sb.reversePercent, "%"], ["最大無入力", sa.gapMs, sb.gapMs, " ms"]];
    if (["rest", "free"].includes(a.protocol)) pairs.unshift(["正味移動", sa.net, sb.net, unit], ["偏位 RMS", sa.rms, sb.rms, unit]);
    else pairs.unshift(...directions.map((d, i) => [d.label + " 移動量", sa.groups[i].net, sb.groups[i].net, unit]), ["方向比", sa.balance, sb.balance, "%"]);
    $("comparisonResults").replaceChildren(...pairs.map(([label, va, vb, suffix]) => row([label, fmt(va, suffix === unit ? "" : suffix), fmt(vb, suffix === unit ? "" : suffix), Number.isFinite(va) && va > 0 && Number.isFinite(vb) ? fmt(vb / va, "×", 3) : "—"])));
    $("comparisonNote").textContent = a.protocol !== b.protocol ? "低速 / 高速の比較" : "";
  }
  $("compareA").addEventListener("change", renderComparison); $("compareB").addEventListener("change", renderComparison);

  function canvas(id) {
    const element = $(id); const scale = window.devicePixelRatio || 1;
    const width = element.clientWidth, height = element.clientHeight;
    if (element.width !== Math.round(width * scale) || element.height !== Math.round(height * scale)) { element.width = Math.round(width * scale); element.height = Math.round(height * scale); }
    const context = element.getContext("2d"); context.setTransform(scale, 0, 0, scale, 0, 0); context.clearRect(0, 0, width, height);
    context.font = "10px Consolas, monospace"; context.fillStyle = "#687c82"; context.strokeStyle = "#e0e6db"; context.lineWidth = 1;
    for (let i = 1; i < 4; i++) { context.beginPath(); context.moveTo(30, height * i / 4); context.lineTo(width - 34, height * i / 4); context.stroke(); }
    return { c: context, width, height };
  }
  function plot(context, samples, x, y, color) {
    context.strokeStyle = color; context.lineWidth = 1.6; context.beginPath();
    let started = false;
    for (const sample of samples) {
      const xx = x(sample), yy = y(sample);
      if (!Number.isFinite(xx) || !Number.isFinite(yy) || sample.lost) { started = false; continue; }
      if (sample.breakBefore) started = false;
      if (started) context.lineTo(xx, yy); else context.moveTo(xx, yy);
      started = true;
    }
    context.stroke();
  }
  function renderLive() {
    const latest = state.live.at(-1);
    const windowEnd = state.source === "browser" ? performance.now() : latest?.t;
    const live = latest ? state.live.filter(s => s.t >= windowEnd - 4000) : [];
    const alive = !!latest && fresh();
    const recentlyMoved = state.source !== "browser" || performance.now() - state.arrival < 100;
    $("liveX").textContent = alive ? fmt(recentlyMoved ? latest.dx : 0, "", 0) : "—"; $("liveY").textContent = alive ? fmt(recentlyMoved ? latest.dy : 0, "", 0) : "—";
    $("liveSqual").textContent = alive ? fmt(latest.squal, "", 0) : "—"; $("liveShutter").textContent = alive ? fmt(latest.shutter, "", 0) : "—";
    $("liveP10").textContent = alive ? fmt(quantile(live.map(s => s.squal), 0.1), "", 0) : "—";
    const span = live.length > 1 ? latest.t - live[0].t : 0;
    $("liveRate").textContent = state.source === "browser" && alive ? fmt(live.length / 4, "", 0) : alive && span > 0 ? fmt((live.length - 1) * 1000 / span, "", 0) : "—";
    $("liveLost").textContent = state.source === "sensor" ? `${state.lost} / 不正 ${state.invalid}` : "—";
    if (state.source === "sensor") {
      const recentProblem = live.some(s => s.lost) || (state.lastInvalid > 0 && performance.now() - state.lastInvalid < 4000);
      sourceStatus(camera.working ? "USB HID · 撮影中" : alive ? recentProblem ? "USB受信中 · 直近4秒に通信欠落" : "USB HID · 受信中" : state.hid ? "USBデータ待ち" : "USB切断", camera.working ? "live" : alive ? recentProblem ? "error" : "live" : "error");
      if (!alive && state.session && !camera.working) stopSession("センサーデータが途切れました。未完了の結果は比較に使いません。");
    }
    renderCamera();
    if (state.capture) {
      const elapsed = performance.now() - state.capture.wallStart;
      $("captureBar").firstElementChild.style.width = Math.min(100, elapsed / (["rest", "free"].includes(state.session.protocol) ? 50 : state.session.protocol === "slow" ? 25 : 5)) + "%";
    }
    const trajectory = canvas("trajectory");
    if (!live.length) { trajectory.c.fillText("INPUT WAITING", 20, 25); }
    else {
      let x = 0, y = 0; const positions = live.map(s => { x += s.dx; y += s.dy; return { x, y, lost: s.lost, breakBefore: s.breakBefore }; });
      const xs = positions.map(p => p.x), ys = positions.map(p => p.y);
      const minX = Math.min(0, ...xs), maxX = Math.max(0, ...xs), minY = Math.min(0, ...ys), maxY = Math.max(0, ...ys);
      const factor = Math.min((trajectory.width - 70) / Math.max(20, maxX - minX), (trajectory.height - 60) / Math.max(20, maxY - minY));
      const xx = p => trajectory.width / 2 + (p.x - (minX + maxX) / 2) * factor;
      const yy = p => trajectory.height / 2 + (p.y - (minY + maxY) / 2) * factor;
      plot(trajectory.c, positions, xx, yy, "#087d77");
      trajectory.c.fillStyle = "#d65b45"; trajectory.c.beginPath(); trajectory.c.arc(xx(positions.at(-1)), yy(positions.at(-1)), 3.5, 0, 2 * Math.PI); trajectory.c.fill();
      trajectory.c.fillStyle = "#687c82"; trajectory.c.fillText(`auto scale · ${fmt(1 / factor, " unit/px", 2)}`, 15, 20);
    }
    const quality = canvas("qualityChart"); const optical = live.filter(s => Number.isFinite(s.squal));
    if (!optical.length) quality.c.fillText(state.source === "browser" ? "SQUAL / SHUTTER: unavailable in browser input" : "SQUAL / SHUTTER: waiting", 15, 25);
    else {
      const end = latest.t; const x = s => 30 + (s.t - (end - 4000)) / 4000 * (quality.width - 70);
      const maxShutter = Math.max(1, ...optical.map(s => s.shutter));
      plot(quality.c, optical, x, s => quality.height - 24 - s.squal / 255 * (quality.height - 48), "#087d77");
      plot(quality.c, optical, x, s => quality.height - 24 - s.shutter / maxShutter * (quality.height - 48), "#d65b45");
      quality.c.fillText("255", 5, 20); quality.c.fillText("0", 10, quality.height - 12); quality.c.fillText(String(maxShutter), quality.width - 38, 20);
    }
    const speed = canvas("speedChart");
    if (live.length) {
      const end = windowEnd, start = end - 4000;
      const bins = Array.from({ length: 200 }, (_, i) => ({ t: start + i * 20, value: 0, lost: false }));
      let previous = null;
      for (const sample of live) {
        const index = Math.min(199, Math.max(0, Math.floor((sample.t - start) / 20)));
        bins[index].value += Math.hypot(sample.dx, sample.dy) * 50;
        if (sample.lost || sample.breakBefore) { const first = previous ? Math.max(0, Math.floor((previous.t - start) / 20)) : index; for (let i = first; i <= index; i++) bins[i].lost = true; }
        previous = sample;
      }
      const max = Math.max(1, ...bins.map(b => b.value));
      plot(speed.c, bins, b => 30 + (b.t - start) / 4000 * (speed.width - 65), b => speed.height - 16 - b.value / max * (speed.height - 38), "#087d77");
      speed.c.fillText(fmt(max, state.source === "browser" ? " u/s" : " counts/s", 0), 15, 15);
    }
  }
  $("openGuide").addEventListener("click", () => $("guideDialog").showModal());
  $("openHistory").addEventListener("click", () => $("historyDialog").showModal());
  document.querySelectorAll("[data-close]").forEach(button => button.addEventListener("click", () => button.closest("dialog").close()));
  $("notice").addEventListener("click", () => notice(""));
  renderHistory(); renderLive(); setInterval(renderLive, 100);
}
