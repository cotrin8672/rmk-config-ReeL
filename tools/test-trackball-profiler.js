"use strict";

const fs = require("fs");

const html = fs.readFileSync("tools/trackball-profiler.html", "utf8");
const scriptMatch = html.match(/<script>([\s\S]*?)<\/script>/);
if (!scriptMatch) throw new Error("inline script not found");
new Function(scriptMatch[1]);

function extractFunction(name) {
  const start = scriptMatch[1].indexOf(`function ${name}`);
  if (start < 0) throw new Error(`missing function ${name}`);
  const bodyStart = scriptMatch[1].indexOf("{", start);
  let depth = 0;
  for (let index = bodyStart; index < scriptMatch[1].length; index += 1) {
    if (scriptMatch[1][index] === "{") depth += 1;
    if (scriptMatch[1][index] === "}" && --depth === 0) {
      return scriptMatch[1].slice(start, index + 1);
    }
  }
  throw new Error(`unterminated function ${name}`);
}

const directionMatch = scriptMatch[1].match(/const directions = (\[[\s\S]*?\n\s*\]);/);
if (!directionMatch) throw new Error("directions not found");
const directions = new Function(`return ${directionMatch[1]}`)();
const expectedDirections = ["右", "左", "下", "上"];
if (JSON.stringify(directions.map(direction => direction.label)) !== JSON.stringify(expectedDirections)) {
  throw new Error(`calibration directions regressed: ${directions.map(direction => direction.label)}`);
}
if (!/const MIN_ROUNDS = 3;/.test(scriptMatch[1]) || !/const MAX_ROUNDS = 10;/.test(scriptMatch[1])) {
  throw new Error("calibration round limits regressed");
}
if (!/const REQUIRED_STABLE_ROUNDS = 2;/.test(scriptMatch[1])) {
  throw new Error("calibration convergence condition regressed");
}

const calibrationFunctions = [
  "applyMatrix",
  "applyLengthPreservingMatrix",
  "scaleMatrix",
  "angleError",
  "robustAxis",
  "fitCorrection",
  "median",
  "classifyInliers",
  "fitReliableCorrection",
  "rmsAngle"
].map(extractFunction).join("\n");

const runCalibration = new Function("directions", `${calibrationFunctions}
  const samples = [];
  const measuredBasis = [[0.98, 0.18], [-0.12, 0.99]];
  for (let round = 0; round < 5; round += 1) {
    directions.forEach((direction, directionIndex) => {
      const noise = (round - 2) * 0.002;
      samples.push({
        dx: measuredBasis[0][0] * direction.x + measuredBasis[0][1] * direction.y + noise,
        dy: measuredBasis[1][0] * direction.x + measuredBasis[1][1] * direction.y - noise,
        tx: direction.x,
        ty: direction.y,
        directionIndex,
        straightness: 0.98
      });
    });
  }
  samples.push({
    dx: -1,
    dy: 0,
    tx: 1,
    ty: 0,
    directionIndex: 0,
    straightness: 0.99
  });

  const assessment = fitReliableCorrection(samples);
  const transformed = applyLengthPreservingMatrix(assessment.matrix, 37, -19);
  return {
    assessment,
    lengthError: Math.abs(Math.hypot(...transformed) - Math.hypot(37, -19))
  };
`);

const calibration = runCalibration(directions);
if (calibration.assessment.excluded !== 1) {
  throw new Error(`expected one excluded outlier, got ${calibration.assessment.excluded}`);
}
if (calibration.assessment.rms > 0.5) {
  throw new Error(`calibration RMS too high: ${calibration.assessment.rms}`);
}
if (calibration.lengthError > 1e-9) {
  throw new Error(`length-preserving transform drifted: ${calibration.lengthError}`);
}

const cpiFunctions = [
  "fnv1a",
  "validateProfileCpi",
  "encodeProfileCpiBlob",
  "decodeProfileCpiBlob"
].map(extractFunction).join("\n");
const testCpi = new Function(`${cpiFunctions}
  const PROFILE_CPI_BLOB_SIZE = 24;
  const PROFILE_CPI_MAGIC = [0x52, 0x43, 0x50, 0x31];
  const PROFILE_CPI_COUNT = 5;
  const PROFILE_CPI_MIN = 200;
  const PROFILE_CPI_MAX = 3200;
  const PROFILE_CPI_STEP = 200;
  const values = [200, 800, 1600, 2400, 3200];
  const blob = encodeProfileCpiBlob(values);
  const decoded = decodeProfileCpiBlob(blob);
  blob[10] ^= 0x80;
  return { decoded, corrupted: decodeProfileCpiBlob(blob) };
`);
const cpi = testCpi();
if (JSON.stringify(cpi.decoded) !== JSON.stringify([200, 800, 1600, 2400, 3200])) {
  throw new Error(`CPI round trip failed: ${cpi.decoded}`);
}
if (cpi.corrupted !== null) throw new Error("corrupted CPI blob was accepted");

console.log(
  `trackball profiler OK: ${directions.length} directions, ` +
  `${calibration.assessment.excluded} outlier excluded, ` +
  `RMS ${calibration.assessment.rms.toFixed(3)}°`
);
