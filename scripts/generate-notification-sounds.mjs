import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

const sampleRate = 44_100;

function envelope(t, start, duration, attack = 0.018, release = 0.24) {
  const local = t - start;
  if (local < 0 || local >= duration) {
    return 0;
  }

  const attackGain = Math.min(1, local / attack);
  const releaseStart = Math.max(attack, duration - release);
  const releaseGain =
    local <= releaseStart
      ? 1
      : Math.max(
          0,
          (duration - local) / Math.max(0.001, duration - releaseStart)
        );

  return attackGain * releaseGain * releaseGain;
}

function tone(t, note) {
  const gain = envelope(
    t,
    note.start,
    note.duration,
    note.attack,
    note.release
  );
  if (!gain) {
    return 0;
  }

  const phase = 2 * Math.PI * note.frequency * (t - note.start);
  const harmonics =
    Math.sin(phase) +
    0.22 * Math.sin(phase * 2) +
    0.08 * Math.sin(phase * 3);

  return gain * harmonics * note.gain;
}

function writeWav(filePath, duration, notes) {
  const frameCount = Math.ceil(duration * sampleRate);
  const samples = new Float64Array(frameCount);
  let peak = 0;

  for (let index = 0; index < frameCount; index += 1) {
    const time = index / sampleRate;
    let sample = 0;
    for (const note of notes) {
      sample += tone(time, note);
    }
    samples[index] = sample;
    peak = Math.max(peak, Math.abs(sample));
  }

  const normalization = peak > 0 ? 0.78 / peak : 0;
  const dataSize = frameCount * 2;
  const wav = Buffer.alloc(44 + dataSize);

  wav.write("RIFF", 0, "ascii");
  wav.writeUInt32LE(36 + dataSize, 4);
  wav.write("WAVE", 8, "ascii");
  wav.write("fmt ", 12, "ascii");
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(sampleRate, 24);
  wav.writeUInt32LE(sampleRate * 2, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36, "ascii");
  wav.writeUInt32LE(dataSize, 40);

  for (let index = 0; index < frameCount; index += 1) {
    const sample = Math.max(-1, Math.min(1, samples[index] * normalization));
    wav.writeInt16LE(Math.round(sample * 32_767), 44 + index * 2);
  }

  writeFileSync(filePath, wav);
}

const outputDirectory = path.resolve("public", "sounds");
mkdirSync(outputDirectory, { recursive: true });

writeWav(path.join(outputDirectory, "mod-install-success.wav"), 0.92, [
  {
    frequency: 523.251,
    start: 0.03,
    duration: 0.46,
    gain: 0.72,
    attack: 0.014,
    release: 0.25
  },
  {
    frequency: 659.255,
    start: 0.18,
    duration: 0.48,
    gain: 0.78,
    attack: 0.014,
    release: 0.27
  },
  {
    frequency: 783.991,
    start: 0.34,
    duration: 0.54,
    gain: 0.86,
    attack: 0.016,
    release: 0.34
  },
  {
    frequency: 1567.982,
    start: 0.35,
    duration: 0.25,
    gain: 0.12,
    attack: 0.01,
    release: 0.18
  }
]);

writeWav(path.join(outputDirectory, "mod-install-failed.wav"), 0.82, [
  {
    frequency: 329.628,
    start: 0.03,
    duration: 0.38,
    gain: 0.72,
    attack: 0.012,
    release: 0.2
  },
  {
    frequency: 261.626,
    start: 0.22,
    duration: 0.4,
    gain: 0.82,
    attack: 0.012,
    release: 0.23
  },
  {
    frequency: 195.998,
    start: 0.42,
    duration: 0.36,
    gain: 0.68,
    attack: 0.014,
    release: 0.25
  }
]);
