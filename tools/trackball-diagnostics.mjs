export const directions = [
  { label: "右", symbol: "→" }, { label: "左", symbol: "←" },
  { label: "下", symbol: "↓" }, { label: "上", symbol: "↑" }
];
const protocols = { guided: "上下・左右の往復", slow: "低速4方向", fast: "高速4方向", rest: "静止5秒", free: "自由5秒" };

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
  if ((a.protocol === "guided") !== (b.protocol === "guided")) issues.push("測定手順が異なります");
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
  if (run.protocol === "guided") {
    const rows = [["condition","ball","position","height_offset_mm","cpi","settings","source","transport","stage","speed","cue_leg","cue_direction","t_ms","seq","dx","dy","squal","shutter","motion","lost","invalid_reason"]];
    for (const stage of run.stages) for (const s of stage.samples || []) {
      const phase = guidePhase(stage, Math.max(0, s.t));
      rows.push([run.conditions.name,run.conditions.ball,run.conditions.position,run.conditions.offsetMm ?? "",run.conditions.cpi,run.conditions.note,run.source,run.transport || "",stage.id,stage.speed || "rest",phase.countdown ? "countdown" : phase.leg + 1,phase.countdown ? "" : directions[phase.direction]?.label || "静止",s.t,s.seq ?? "",s.dx,s.dy,s.squal ?? "",s.shutter ?? "",s.motion ?? "",s.lost || 0,stage.invalidReason]);
    }
    return "\ufeff" + rows.map(row => row.map(csvCell).join(",")).join("\r\n");
  }
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




export const guidedSteps = [
  { id: "rest", label: "静止", symbol: "○", durationMs: 5000 },
  { id: "vertical-slow", label: "上下・ゆっくり", symbol: "↕", axis: "vertical", speed: "slow", moveMs: 2000, holdMs: 400, first: 2, second: 3 },
  { id: "vertical-fast", label: "上下・速く", symbol: "↕", axis: "vertical", speed: "fast", moveMs: 500, holdMs: 400, first: 2, second: 3 },
  { id: "horizontal-slow", label: "左右・ゆっくり", symbol: "↔", axis: "horizontal", speed: "slow", moveMs: 2000, holdMs: 400, first: 0, second: 1 },
  { id: "horizontal-fast", label: "左右・速く", symbol: "↔", axis: "horizontal", speed: "fast", moveMs: 500, holdMs: 400, first: 0, second: 1 }
];
export const GUIDE_COUNTDOWN_MS = 3000;
export function guidePhase(step, elapsedMs) {
  if (elapsedMs < 0 || !Number.isFinite(elapsedMs)) throw new Error("Invalid guide time");
  const t = elapsedMs - GUIDE_COUNTDOWN_MS;
  if (t < 0) return { countdown: Math.ceil(-t / 1000), position: -1, direction: step.first, leg: 0, done: false };
  if (step.id === "rest") return { countdown: 0, position: 0, leg: 0, done: t >= step.durationMs, remaining: Math.max(0, step.durationMs - t) };
  const period = step.moveMs + step.holdMs, leg = Math.min(9, Math.floor(t / period));
  const progress = Math.min(1, Math.max(0, (t - leg * period) / step.moveMs));
  const movement = (1 - Math.cos(Math.PI * progress)) / 2;
  return { countdown: 0, leg, direction: leg % 2 ? step.second : step.first,
    position: leg % 2 ? 1 - movement * 2 : movement * 2 - 1,
    holding: t - leg * period >= step.moveMs, done: t >= period * 10 };
}
export function guideDuration(step) { return GUIDE_COUNTDOWN_MS + (step.durationMs || (step.moveMs + step.holdMs) * 10); }
function cv(values) {
  if (values.length < 2) return null;
  const mean = values.reduce((a, b) => a + b, 0) / values.length;
  return mean > 0 ? Math.sqrt(values.reduce((s, n) => s + (n - mean) ** 2, 0) / (values.length - 1)) / mean * 100 : null;
}
const ratio = (a, b) => Number.isFinite(a) && Number.isFinite(b) && b > 0 ? a / b : null;
export function analyzeGuidedStage(step, samples, invalidReason = "", recordedMs = guideDuration(step)) {
  const durationMs = guideDuration(step), windows = step.id === "rest"
    ? [{ start: GUIDE_COUNTDOWN_MS, end: durationMs, direction: -1 }]
    : Array.from({ length: 10 }, (_, i) => ({ start: GUIDE_COUNTDOWN_MS + i * (step.moveMs + step.holdMs),
      end: GUIDE_COUNTDOWN_MS + (i + 1) * (step.moveMs + step.holdMs), direction: i % 2 ? step.second : step.first }));
  // The hint clock defines direction windows, so missing sensor output cannot erase a stroke.
  const strokes = windows.filter(w => w.start < recordedMs).map((w, index) => {
    w = { ...w, end: Math.min(w.end, recordedMs) };
    const local = samples.filter(s => s.t >= w.start && s.t < w.end).map(s => ({ ...s, t: s.t - w.start }));
    const metrics = summarizeStroke(local, w.end - w.start);
    const moving = step.id === "rest" ? local : local.filter(s => s.t < step.moveMs);
    const activeMetrics = summarizeStroke(moving, step.durationMs || step.moveMs);
    const late = step.id === "rest" ? [] : local.filter(s => s.t >= step.moveMs);
    return { stageId: step.id, speed: step.speed || "rest", direction: w.direction, index,
      startMs: w.start, endMs: w.end, invalidReason, metrics,
      movingGapMs: activeMetrics.gapMs,
      outputWindowMs: local.filter(s => s.dx || s.dy).reduce((range, s) => {
        range[0] ??= s.t; range[1] = s.t; return range;
      }, [null, null]),
      latePath: late.reduce((sum, s) => sum + Math.hypot(s.dx, s.dy), 0),
      crossPercent: ratio(metrics.crossRms, metrics.net) === null ? null : metrics.crossRms / metrics.net * 100 };
  });
  const measured = samples.filter(s => s.t >= GUIDE_COUNTDOWN_MS && s.t < Math.min(recordedMs, durationMs));
  const total = summarizeStroke(measured.map(s => ({ ...s, t: s.t - GUIDE_COUNTDOWN_MS })), Math.max(1, Math.min(recordedMs, durationMs) - GUIDE_COUNTDOWN_MS));
  // 128 time bins preserve min/max output and optical extrema in saved history; CSV keeps all samples.
  const binMs = (durationMs - GUIDE_COUNTDOWN_MS) / 128;
  const plot = Array.from({ length: 128 }, (_, i) => ({ t: i * binMs, x: 0, y: 0, path: 0, peak: 0, squal: null, shutter: null, lost: 0, samples: 0 }));
  for (const s of measured) {
    const bin = plot[Math.min(127, Math.floor((s.t - GUIDE_COUNTDOWN_MS) / binMs))];
    bin.samples++; bin.x += s.dx; bin.y += s.dy; bin.path += Math.hypot(s.dx, s.dy); bin.peak = Math.max(bin.peak, Math.hypot(s.dx, s.dy));
    if (Number.isFinite(s.squal)) bin.squal = bin.squal === null ? s.squal : Math.min(bin.squal, s.squal);
    if (Number.isFinite(s.shutter)) bin.shutter = bin.shutter === null ? s.shutter : Math.max(bin.shutter, s.shutter);
    bin.lost += s.lost || 0;
  }
  return { ...step, durationMs, recordedMs, invalidReason, strokes, total, plot, binMs,
    received: measured.length, lost: total.lost, targetLegs: windows.length };
}
export function summarizeGuidedRun(run) {
  const strokes = run.strokes.filter(s => s.direction >= 0 && !s.invalidReason);
  const groups = directions.map((d, direction) => {
    const bySpeed = speed => {
      const group = strokes.filter(s => s.direction === direction && s.speed === speed);
      return { count: group.length, net: quantile(group.map(s => s.metrics.net)), cv: cv(group.map(s => s.metrics.net)),
        crossPercent: quantile(group.map(s => s.crossPercent)), reversePercent: quantile(group.map(s => s.metrics.reversePercent)),
        squalP10: quantile(group.map(s => s.metrics.squalP10)), shutterMedian: quantile(group.map(s => s.metrics.shutterMedian)),
        gapMs: quantile(group.map(s => s.movingGapMs), 1), durationMs: quantile(group.map(s => s.metrics.durationMs)) };
    };
    const slow = bySpeed("slow"), fast = bySpeed("fast");
    return { label: d.label, direction, slow, fast, fastRatio: slow.count === 5 && fast.count === 5 ? ratio(fast.net, slow.net) : null };
  });
  const balance = (a, b) => groups[a].slow.count === 5 && groups[b].slow.count === 5
    ? ratio(Math.min(groups[a].slow.net, groups[b].slow.net), Math.max(groups[a].slow.net, groups[b].slow.net)) : null;
  const rest = run.stages.find(s => s.id === "rest");
  const issues = [];
  if (!run.complete) issues.push(run.reason || "測定未完了");
  if (run.stages.some(s => s.invalidReason)) issues.push("通信または記録範囲に異常あり");
  if (strokes.some(s => s.metrics.path === 0)) issues.push("移動指示中に出力0の片道あり");
  return { groups, verticalBalance: balance(2, 3), horizontalBalance: balance(0, 1),
    minFastRatio: quantile(groups.map(g => g.fastRatio), 0), restRms: rest && !rest.invalidReason ? rest.total.rms : null,
    squalP10: quantile(strokes.map(s => s.metrics.squalP10)),
    lost: run.stages.reduce((n, s) => n + s.lost, 0), issues };
}
function reportEscape(value) { return String(value ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])); }
export function guidedReport(run) {
  const f = (n, suffix = "", digits = 2) => Number.isFinite(n) ? n.toFixed(digits) + suffix : "—";
  const tr = values => "<tr>" + values.map(v => "<td>" + reportEscape(v) + "</td>").join("") + "</tr>";
  const table = (headers, rows) => "<table><thead><tr>" + headers.map(h => '<th scope="col">' + reportEscape(h) + "</th>").join("") + "</tr></thead><tbody>" + rows.map(tr).join("") + "</tbody></table>";
  const summary = summarizeGuidedRun(run), c = run.conditions;
  const source = run.source === "sensor" ? "USB・センサー読取値（ファームの角度・感度補正前）" : run.source === "demo" ? "DEMO・合成データ" : "PC入力（ファーム・OS処理後）";
  let body = "<h1>ReeL ケース位置比較レポート</h1><p>" + reportEscape(c.name) + " · " + reportEscape(new Date(run.date).toLocaleString("ja-JP")) + "</p>";
  body += table(["入力", "球", "ケース位置・高さ", "高さ差 mm", "CPI", "支持・補正メモ"], [[source, c.ball, c.position, c.offsetMm ?? "未記録", c.cpi, c.note]]);
  body += "<p>" + reportEscape(run.complete ? "全5段階終了" : "未完了：" + (run.reason || "")) + " · 手の可動域の端から端を5往復 · 単位：" + (run.source === "browser" ? "u" : "counts") + " · 球の実回転量・速度は未計測</p>";
  if (summary.issues.length) body += "<p class=warning>" + reportEscape(summary.issues.join(" / ")) + "</p>";
  body += "<h2>方向別の集計</h2>" + table(["方向", "低速 回数", "低速 中央値", "低速 CV %", "高速 回数", "高速 中央値", "高速 CV %", "高速/低速", "低速 横ぶれ %", "高速 横ぶれ %", "SQUAL P10 低/高"], [2, 3, 0, 1].map(i => summary.groups[i]).map(g => [g.label, g.slow.count + "/5", f(g.slow.net), f(g.slow.cv), g.fast.count + "/5", f(g.fast.net), f(g.fast.cv), f(g.fastRatio, "×"), f(g.slow.crossPercent), f(g.fast.crossPercent), f(g.slow.squalP10) + " / " + f(g.fast.squalP10)]));
  body += table(["上下の最小/最大", "左右の最小/最大", "高速/低速 最小", "静止偏位 RMS", "通信欠落"], [[f(summary.verticalBalance === null ? null : summary.verticalBalance * 100, "%"), f(summary.horizontalBalance === null ? null : summary.horizontalBalance * 100, "%"), f(summary.minFastRatio, "×"), f(summary.restRms), summary.lost]]);
  function chart(stage) {
    const points = stage.plot, width = 760, height = 140, pad = 25;
    const maxPath = Math.max(1, ...points.map(p => p.path / stage.binMs * 1000));
    const line = (key, scale, color) => {
      let path = "", open = false;
      points.forEach((p, i) => {
        if (!p.samples || !Number.isFinite(key(p)) || p.lost) { open = false; return; }
        const x = pad + i / 127 * (width - pad * 2), y = height - pad - key(p) / scale * (height - pad * 2);
        path += (open ? "L" : "M") + x.toFixed(1) + "," + y.toFixed(1); open = true;
      });
      return '<path d="' + path + '" stroke="' + color + '" fill="none" stroke-width="1.5"/>';
    };
    let markers = "";
    if (stage.moveMs) for (let i = 1; i < 10; i++) {
      const x = pad + i / 10 * (width - pad * 2);
      markers += '<line x1="' + x + '" x2="' + x + '" y1="10" y2="120" stroke="#d7dfdc"/>';
    }
    return '<svg viewBox="0 0 760 140" role="img" aria-label="出力量とSQUAL">' + markers +
      line(p => p.path / stage.binMs * 1000, maxPath, "#087d77") + line(p => p.squal, 255, "#c95841") +
      '<text x="25" y="13" font-size="10">出力量 0–' + f(maxPath, "/s", 0) + '（緑） / SQUAL 0–255（橙）</text></svg>';
  }
  for (const stage of run.stages) {
    body += "<h2>" + reportEscape(stage.label) + "</h2><p>" + reportEscape(stage.id === "rest" ? "手を離して5秒" : "片道 " + stage.moveMs / 1000 + " 秒 + 端点保持 " + stage.holdMs / 1000 + " 秒、10区間") + " · 受信 " + reportEscape(stage.received) + " 件 / 欠落 " + reportEscape(stage.lost) + " · 案内描画の最大間隔 " + f(stage.maxGuideGapMs, " ms") + "</p>";
    if (stage.invalidReason) body += "<p class=warning>" + reportEscape(stage.invalidReason) + "</p>";
    body += chart(stage);
    body += "<p class=muted>グラフは " + f(stage.binMs) + " ms集計。SQUALは各区間の最小値。折り返しの線は指示時刻。</p>";
    body += table(["回", "指示方向", "開始 s", "終了 s", "正味出力", "移動総量", "横ぶれ RMS", "横ぶれ %", "逆走 %", "移動指示中の最大無出力 ms", "端点保持中の出力総量", "SQUAL P10", "露光 中央", "最大1サンプル出力"], stage.strokes.map(s => [s.index + 1, directions[s.direction]?.label || "静止", f((s.startMs - GUIDE_COUNTDOWN_MS) / 1000), f((s.endMs - GUIDE_COUNTDOWN_MS) / 1000), f(s.metrics.net), f(s.metrics.path), f(s.metrics.crossRms), f(s.crossPercent), f(s.metrics.reversePercent), f(s.movingGapMs), f(s.latePath), f(s.metrics.squalP10), f(s.metrics.shutterMedian), f(s.metrics.largestStep)]));
  }
  body += "<h2>測定条件・計算方法</h2><ul><li>案内点は同じ幅を往復し、低速は片道2秒、高速は0.5秒。各端点で0.4秒保持。各段階の最初に3秒の準備時間を置く。</li><li>集計は案内点の時刻で片道ずつ分割。球の応答が0でも、その指示区間を残す。正味出力は片道のΔX・ΔYの合計ベクトル長。</li><li>端点保持も片道の出力量に含め、遅れて出た出力を別記する。最大無出力時間は移動指示の時間内だけで計算し、意図した端点停止を除く。</li><li>方向別の値は5回の中央値。CVは標本標準偏差÷平均。横ぶれは各片道の始点・終点を結ぶ線からのRMSで、%は正味出力で割った値。</li><li>速く／ゆっくりは指示速度。球の実回転量・速度と手の追従遅れは測れていない。高速/低速の比は取りこぼし率ではなく、総合合否や最適距離は自動判定しない。</li><li>USBの生データとPC入力は比較しない。CPI・球・手の置き方をそろえ、距離・位置を一つずつ変更する。</li></ul>";
  return { body, document: '<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>ReeL ケース位置比較レポート</title><style>body{font:14px/1.6 system-ui,sans-serif;margin:24px;color:#18343e}h1{font-size:22px}h2{font-size:18px;margin-top:28px}table{border-collapse:collapse;width:100%;font-size:11px;margin:12px 0}th,td{border:1px solid #d7dfdc;padding:4px;text-align:right}thead{background:#edf1ee;font-weight:bold}td:first-child{text-align:left}svg{display:block;width:100%;max-height:180px}.warning{color:#a52d22}.muted{color:#63777d;font-size:11px}@media print{body{margin:0}h2,svg,tr{break-inside:avoid}}</style><body>' + body + '</body></html>' };
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
  function exportRun(run) { download(JSON.stringify({ schema: run.protocol === "guided" ? 2 : 1, run }, null, 2), "reel-trackball-result.json", "application/json"); }
  try {
    const saved = localStorage.getItem(storageKey);
    if (saved) {
      history = JSON.parse(saved);
      if (!Array.isArray(history) || history.some(run => !run || typeof run.id !== "string" || !run.conditions
          || typeof run.conditions.name !== "string" || !Array.isArray(run.strokes) || !(run.protocol in protocols)
          || !["sensor", "browser", "demo"].includes(run.source) || (run.protocol === "guided" && !Array.isArray(run.stages)))) throw new Error("Invalid saved history");
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
    $("cpi").readOnly = source === "sensor";
    $("prompt").textContent = source ? "条件を記録して測定を開始してください。" : "USBで接続してください。";
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
    if (state.source === "sensor") $("cpi").value = sample.cpi;
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
    $("prompt").textContent = "条件を記録して測定を開始してください。";
  });
  $("demoInput").addEventListener("click", async () => {
    await disconnect(); resetInput("demo"); notice("");
    let seq = 0, previousCapture = null, previousX = 0, previousY = 0, previousDemoTime = -Infinity;
    state.demoTimer = setInterval(() => {
      for (let i = 0; i < 4; i++) {
        seq++; const t = Math.max(previousDemoTime + 0.001, performance.now() - (3 - i) * 4); previousDemoTime = t;
        const step = state.session ? guidedSteps[state.session.index] : null;
        const phase = step && state.capture ? guidePhase(step, Math.max(0, t - state.capture.wallStart)) : null;
        const position = phase ? phase.position * 500 : Math.sin(t / 700) * 500;
        const x = step?.axis === "vertical" ? position * 0.6 : position * 0.8;
        const y = step?.axis === "vertical" ? position * 0.8 : position * -0.6;
        if (previousCapture !== state.capture) { previousCapture = state.capture; previousX = x; previousY = y; }
        let dx = Math.round(x - previousX), dy = Math.round(y - previousY); previousX = x; previousY = y;
        if (step?.id === "rest") { dx = seq % 113 === 0 ? 1 : 0; dy = 0; }
        ingest({ seq, t, dx, dy, squal: Math.round(40 + 5 * Math.sin(seq / 35)), shutter: Math.round(120 + 20 * Math.sin(seq / 42)), motion: dx || dy ? 128 : 0 });
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
  document.addEventListener("visibilitychange", () => { if (document.hidden) { setCameraLive(false); stopSession("案内画面が非表示になったため中断しました。"); } });
  paintFrame("sensorImage", null); paintFrame("referenceImage", null);

  function lockControls(locked) {
    for (const field of $("conditions").elements) field.disabled = locked;
    renderCamera(); $("prepare").disabled = locked; $("cancel").disabled = !locked;
    $("openGuide").disabled = locked; $("openHistory").disabled = locked;
    $("prepare").hidden = locked; $("stroke").hidden = !locked; $("cancel").hidden = !locked;
  }
  let guideAnimation = null, reportRun = null;
  function renderGuide(now) {
    if (!state.session) return;
    const step = guidedSteps[state.session.index];
    const elapsed = state.capture ? Math.max(0, now - state.capture.wallStart) : GUIDE_COUNTDOWN_MS + (now - state.session.previewAt) % (step.durationMs || (step.moveMs + step.holdMs) * 2);
    const phase = guidePhase(step, elapsed);
    $("paceGuide").className = "pace-guide " + (step.axis || "rest");
    $("paceDot").style.transform = "translate(-50%,-50%) translate(" + (step.axis === "horizontal" ? phase.position * 62 : 0) + "px," + (step.axis === "vertical" ? phase.position * 25 : 0) + "px)";
    $("directionSymbol").textContent = step.id === "rest" ? "○" : directions[phase.direction]?.symbol || step.symbol;
    $("paceLabel").textContent = state.capture ? phase.countdown ? phase.countdown + "秒後に開始" : step.id === "rest" ? "静止 · 残り " + Math.ceil(phase.remaining / 1000) + "秒" : (phase.holding ? "端で止める" : directions[phase.direction].label + "へ") + " · " + (phase.leg + 1) + "/10" : step.id === "rest" ? "手を離す" : "片道 " + step.moveMs / 1000 + "秒 · 案内";
    if (state.capture) {
      state.capture.maxGuideGap = Math.max(state.capture.maxGuideGap, now - state.capture.lastGuideFrame);
      state.capture.lastGuideFrame = now;
    }
    guideAnimation = requestAnimationFrame(renderGuide);
  }
  function promptStep() {
    if (!state.session) return;
    const step = guidedSteps[state.session.index]; state.session.previewAt = performance.now();
    $("progress").textContent = (state.session.index + 1) + "/5 · " + step.label;
    $("guideSteps").replaceChildren(...guidedSteps.map((s, i) => { const li = document.createElement("li"); li.textContent = s.label; li.className = i === state.session.index ? "active" : i < state.session.index ? "done" : ""; return li; }));
    $("prompt").textContent = step.id === "rest" ? "球から手を離してください。" : (step.axis === "vertical" ? "上端" : "左端") + "へ準備。Spaceで開始。点に合わせて5往復。";
    $("stroke").disabled = false; $("stroke").textContent = "開始（Space）"; $("stroke").ariaLabel = step.label + "を開始";
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
    state.run = { id: crypto.randomUUID(), date: new Date().toISOString(), source: state.source, transport: state.transport,
      protocol: "guided", methodVersion: 1, unadjusted: null, complete: false, stages: [], strokes: [],
      conditions: { name: $("conditionName").value.trim(), ball: $("ball").value.trim(), position: $("position").value.trim(),
        cpi: state.transport === "usb-hid" ? state.live.at(-1)?.cpi : Number($("cpi").value), offsetMm: Number($("heightOffset").value), travel: null, note: $("settingNote").value.trim() } };
    state.session = { index: 0, previewAt: performance.now() }; lockControls(true); notice("");
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
        if (document.pointerLockElement !== $("captureArea")) throw new Error("Pointer Lockを利用できません");
      }
      $("captureArea").focus(); promptStep(); renderResults();
      guideAnimation = requestAnimationFrame(renderGuide); beginCapture();
    } catch {
      state.session = null; state.run = previousRun; lockControls(false); renderResults();
      notice("ポインタを固定できませんでした。Chrome / Edgeで直接開いてください。");
    }
  });
  function beginCapture() {
    if (!state.session || state.capture || !fresh()) return;
    const now = performance.now(), step = guidedSteps[state.session.index];
    const anchor = state.source === "browser" ? now : (state.live.at(-1)?.t || 0) + Math.max(0, now - state.arrival);
    state.capture = { start: anchor, wallStart: now, samples: [], invalidAtStart: state.invalid,
      maxGuideGap: 0, lastGuideFrame: now };
    state.capture.timer = setTimeout(() => finishCapture(), guideDuration(step));
    $("stroke").disabled = true; $("stroke").textContent = "計測中…";
    $("prompt").textContent = step.id === "rest" ? "手を離して静止" : "点の往復に合わせて端から端へ。各端で止める。";
  }
  function finishCapture(reason = "") {
    if (!state.capture || !state.session) return;
    const capture = state.capture, step = guidedSteps[state.session.index]; clearTimeout(capture.timer); state.capture = null;
    const wallDuration = performance.now() - capture.wallStart;
    if (!reason && state.source === "sensor" && (!capture.samples.length || !fresh())) reason = "センサーデータが届いていません";
    if (!reason && (state.invalid > capture.invalidAtStart || capture.samples.some(s => s.lost))) reason = "通信欠落・不正データあり";
    if (!reason && state.source !== "browser" && (capture.samples[0]?.t > 100 || (capture.samples.at(-1)?.t || 0) < guideDuration(step) - 100)) reason = "記録時間が案内の時間を覆っていません";
    if (!reason && (wallDuration > guideDuration(step) + 250 || capture.maxGuideGap > 250)) reason = "案内表示が遅延しました。この段階を再測定してください";
    let stage;
    try { stage = analyzeGuidedStage(step, capture.samples, reason, Math.min(wallDuration, guideDuration(step))); }
    catch (error) {
      reason = "記録を集計できませんでした: " + error.message;
      stage = analyzeGuidedStage(step, [], reason, 0); stage.received = capture.samples.length;
    }
    stage.samples = capture.samples; stage.wallDurationMs = wallDuration; stage.maxGuideGapMs = capture.maxGuideGap;
    stage.clockAnchor = "sensor time aligned to the latest arrival at browser cue start";
    state.run.stages.push(stage); state.run.strokes.push(...stage.strokes);
    renderResults();
    if (reason) { state.run.reason = reason; finishSession(); saveHistory(); notice(reason); return; }
    state.session.index++;
    if (state.session.index === guidedSteps.length) {
      state.run.complete = true; finishSession(); saveHistory(); showReport(state.run);
    } else promptStep();
  }
  function finishSession() {
    cancelAnimationFrame(guideAnimation); guideAnimation = null;
    state.session = null; state.capture = null; lockControls(false);
    $("stroke").disabled = true; $("stroke").textContent = "次へ";
    $("progress").textContent = state.run.complete ? "5/5 完了" : "中断";
    $("prompt").textContent = state.run.complete ? "測定完了" : state.run.reason || "中断しました";
    $("paceLabel").textContent = state.run.complete ? "レポート作成済み" : "中断";
    if (document.pointerLockElement === $("captureArea")) document.exitPointerLock();
    renderResults();
  }
  function stopSession(reason) {
    if (!state.session) return;
    if (state.capture) { finishCapture(reason); return; }
    state.run.reason = reason; finishSession(); saveHistory(); notice(reason);
  }
  $("stroke").addEventListener("click", beginCapture);
  $("cancel").addEventListener("click", () => stopSession("測定を中断しました。途中の記録はレポート・CSVで保存できます。"));
  document.addEventListener("keydown", event => {
    if (event.code === "Escape" && !state.session) { setCameraLive(false); return; }
    if (!state.session || /^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)) return;
    if (event.code === "Space") { event.preventDefault(); if (!event.repeat) beginCapture(); }
    if (event.code === "Escape") stopSession("Escで測定を中断しました。");
    if (event.code === "KeyR" && !event.repeat) {
      if (state.capture) { clearTimeout(state.capture.timer); state.capture = null; }
      else if (state.run.stages.length) { state.run.stages.pop(); state.session.index--; }
      state.run.strokes = state.run.stages.flatMap(s => s.strokes); promptStep(); renderResults();
    }
  });
  document.addEventListener("mousemove", event => {
    if (state.source !== "browser" || document.pointerLockElement !== $("captureArea") || !event.isTrusted) return;
    const dx = event.movementX, dy = event.movementY;
    if (Number.isFinite(dx) && Number.isFinite(dy) && (dx || dy)) ingest({ t: performance.now(), dx, dy });
  });
  document.addEventListener("pointerlockchange", () => { if (state.session && state.source === "browser" && document.pointerLockElement !== $("captureArea")) stopSession("ポインタ固定が解除されました。"); });
  window.addEventListener("blur", () => stopSession("画面のフォーカスが外れたため中断しました。"));
  function showReport(run) {
    reportRun = run; $("reportContent").innerHTML = guidedReport(run).body;
    if (!$("reportDialog").open) $("reportDialog").showModal();
  }
  $("openReport").addEventListener("click", () => showReport(state.run));
  $("downloadReport").addEventListener("click", () => download(guidedReport(reportRun).document, "reel-case-report.html", "text/html;charset=utf-8"));

  function saveHistory() {
    const run = { ...state.run, stages: state.run.stages.map(({ samples, ...stage }) => stage) };
    history.push(run);
    if (storageHealthy) {
      try { localStorage.setItem(storageKey, JSON.stringify(history)); }
      catch { notice("履歴を保存できませんでした。レポートHTML・JSONを保存してください。"); }
    }
    renderHistory();
  }
  function renderResults() {
    const run = state.run; if (!run) return;
    const summary = summarizeGuidedRun(run), unit = run.source === "browser" ? "u" : "counts";
    $("resultStatus").textContent = sourceName(run.source) + " · " + run.stages.length + "/5段階";
    $("resultUnit").textContent = unit;
    const percent = n => fmt(n === null ? null : n * 100, "%");
    const cards = [[percent(summary.verticalBalance), "上下 最小/最大"], [percent(summary.horizontalBalance), "左右 最小/最大"], [fmt(summary.minFastRatio, "×", 3), "高速/低速 最小"], [fmt(summary.restRms), "静止偏位 RMS"]];
    $("resultCards").replaceChildren(...cards.map(([value, label]) => { const div = document.createElement("div"); div.className = "summary"; const strong = document.createElement("strong"); strong.textContent = value; const span = document.createElement("span"); span.textContent = label; div.append(strong, span); return div; }));
    const ordered = [summary.groups[2], summary.groups[3], summary.groups[0], summary.groups[1]];
    $("directionResults").replaceChildren(...ordered.map(g => row([g.label, fmt(g.slow.net), fmt(g.fast.net), fmt(g.fastRatio, "×", 3), fmt(g.slow.cv) + "/" + fmt(g.fast.cv), fmt(g.slow.crossPercent) + "/" + fmt(g.fast.crossPercent), g.slow.count + "/" + g.fast.count])));
    $("resultExplanation").textContent = run.reason || (run.complete ? summary.issues.join(" / ") : "");
    $("exportRaw").disabled = !run.stages.some(s => s.samples?.length);
    $("exportJson").disabled = !run.stages.length; $("openReport").disabled = !!state.session || !run.stages.length;
  }
  $("exportRaw").addEventListener("click", () => download(rawCsv(state.run), "reel-trackball-raw.csv", "text/csv;charset=utf-8"));
  $("exportJson").addEventListener("click", () => exportRun(state.run));
  function renderHistory() {
    const previousA = $("compareA").value, previousB = $("compareB").value;
    const eligible = history.filter(run => run.complete && run.protocol === "guided");
    for (const id of ["compareA", "compareB"]) {
      $(id).replaceChildren(...eligible.map(run => { const option = document.createElement("option"); option.value = run.id; option.textContent = run.conditions.name + " · " + run.conditions.position + " · " + (run.conditions.offsetMm ?? "—") + "mm · " + sourceName(run.source); return option; }));
    }
    $("compareA").value = eligible.some(run => run.id === previousA) ? previousA : eligible.length > 1 ? eligible.at(-2).id : eligible[0]?.id || "";
    $("compareB").value = eligible.at(-1)?.id || "";
    if (eligible.some(run => run.id === previousB) && previousB !== previousA && !eligible.some(run => run.id === previousA)) $("compareB").value = previousB;
    $("history").replaceChildren(...history.slice().reverse().map(run => {
      const div = document.createElement("div"); div.className = "history-item";
      const text = document.createElement("div"); text.textContent = run.conditions.name + " · " + protocols[run.protocol] + " · " + (run.complete ? "完了" : "中断");
      const span = document.createElement("span"); span.textContent = sourceName(run.source) + " / " + run.conditions.ball + " / " + run.conditions.position + " / " + run.conditions.cpi + " CPI";
      text.append(document.createElement("br"), span);
      const actions = document.createElement("div"); actions.className = "actions";
      if (run.protocol === "guided") { const report = document.createElement("button"); report.type = "button"; report.textContent = "詳細レポート"; report.addEventListener("click", () => { $("historyDialog").close(); showReport(run); }); actions.append(report); }
      const button = document.createElement("button"); button.type = "button"; button.textContent = "集計JSON"; button.addEventListener("click", () => exportRun(run)); actions.append(button);
      div.append(text, actions); return div;
    }));
    renderComparison();
  }
  function renderComparison() {
    $("comparisonNote").textContent = "";
    const a = history.find(run => run.id === $("compareA").value), b = history.find(run => run.id === $("compareB").value);
    if (!a || !b || a.id === b.id) { $("comparisonResults").replaceChildren(row(["測定2件を選択", "—", "—", "—"])); return; }
    const issues = comparisonIssues(a, b);
    if (issues.length) { $("comparisonNote").textContent = issues.join(" / "); $("comparisonResults").replaceChildren(row(["条件不一致", "—", "—", "—"])); return; }
    const sa = summarizeGuidedRun(a), sb = summarizeGuidedRun(b);
    $("comparisonUnit").textContent = a.source === "browser" ? "u" : "counts";
    const percent = n => n === null ? null : n * 100;
    const pairs = [["上下比", percent(sa.verticalBalance), percent(sb.verticalBalance), "%"], ["左右比", percent(sa.horizontalBalance), percent(sb.horizontalBalance), "%"],
      ...[2,3,0,1].map(i => [directions[i].label + " 高/低", sa.groups[i].fastRatio, sb.groups[i].fastRatio, "×"]),
      ["静止 RMS", sa.restRms, sb.restRms, ""], ["SQUAL P10", sa.squalP10, sb.squalP10, ""], ["欠落", sa.lost, sb.lost, ""]];
    $("comparisonResults").replaceChildren(...pairs.map(([label, va, vb, suffix]) => row([label, fmt(va, suffix, 2), fmt(vb, suffix, 2), fmt(ratio(vb, va), "×", 2)])));
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
      $("captureBar").firstElementChild.style.width = Math.min(100, elapsed / guideDuration(guidedSteps[state.session.index]) * 100) + "%";
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
