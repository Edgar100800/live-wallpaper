// Deterministic tests for shared/scripts/raf-patch.js.
// Runs the real script inside a fake DOM environment with a manual clock,
// simulating display vsyncs and setTimeout as discrete events.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const SOURCE = readFileSync(path.join(scriptDir, '..', 'raf-patch.js'), 'utf8');

/**
 * Discrete-event simulator.
 *   vsync(t): deliver all pending rAF callbacks with timestamp t
 *   timers:   sorted queue of {time, fn}
 */
function makeEnv() {
  const env = {
    now: 0,
    rafPending: [],
    timers: [],
    events: {},
    vsyncCount: 0,
  };
  const win = {
    addEventListener: (type, fn) => (env.events[type] ??= []).push(fn),
    dispatchEvent: (event) => {
      for (const fn of env.events[event.type] ?? []) fn(event);
    },
    requestAnimationFrame: (cb) => {
      env.rafPending.push(cb);
      return env.rafPending.length;
    },
    setTimeout: (fn, delay) => {
      const time = env.now + delay;
      const id = env.timers.push({ time, fn });
      env.timers.sort((a, b) => a.time - b.time);
      return id;
    },
    performance: { now: () => 0 }, // zero-cost callbacks
    CustomEvent: class { constructor(type, opts) { this.type = type; Object.assign(this, opts); } },
  };
  win.window = win;
  env.win = win;
  return env;
}

function install(env) {
  vm.createContext(env.win);
  vm.runInContext(SOURCE, env.win);
}

function vsync(env, t) {
  env.now = t;
  env.vsyncCount++;
  const cbs = env.rafPending;
  env.rafPending = [];
  // Same-timestamp callbacks share allowedT; a callback may push new rAFs,
  // which belong to the NEXT vsync, hence snapshot before iterating.
  for (const cb of cbs) cb(t);
}

/** Execute every timer due before `t`, oldest first. */
function flushTimers(env, t) {
  while (env.timers.length && env.timers[0].time <= t) {
    const { time, fn } = env.timers.shift();
    env.now = Math.max(env.now, time);
    fn();
  }
}

/** Advance the simulation to `t`: timers first, then one vsync delivery. */
function advance(env, t) {
  if (t > env.now) flushTimers(env, t);
  vsync(env, t);
}

test('installs once and exposes contract globals', () => {
  const env = makeEnv();
  install(env);
  assert.equal(env.win.__pwInstalled, true);
  assert.equal(env.win.__pwPaused, false);
  assert.equal(env.win.__pwFPSCap, 0);
  install(makeEnv()); // second call on fresh context also installs cleanly
});

test('fps cap 30 halves execution against 60 Hz vsync', () => {
  const env = makeEnv();
  install(env);
  env.win.__pwFPSCap = 30;
  let runs = 0;
  let stop = false;
  const loop = () => { runs++; if (!stop) env.win.requestAnimationFrame(loop); };
  env.win.requestAnimationFrame(loop);

  // Simulate one second of 60 Hz vsyncs with timer-driven catch-up.
  for (let t = 0; t <= 1000; t += 1000 / 60) advance(env, t);
  stop = true;
  // ~30 executions (first pass + ~29 gated), not ~60.
  assert.ok(runs >= 28 && runs <= 32, `expected ~30 runs, got ${runs}`);
});

test('uncapped renders on every vsync', () => {
  const env = makeEnv();
  install(env);
  let runs = 0;
  const loop = () => { runs++; env.win.requestAnimationFrame(loop); };
  env.win.requestAnimationFrame(loop);
  for (let t = 0; t < 500; t += 1000 / 60) advance(env, t);
  assert.equal(runs, 30);
});

test('paused defers via 250ms timeout without executing callbacks', () => {
  const env = makeEnv();
  install(env);
  env.win.__pwPaused = true;
  let runs = 0;
  const loop = () => { runs++; env.win.requestAnimationFrame(loop); };
  env.win.requestAnimationFrame(loop);

  for (let t = 0; t <= 600; t += 1000 / 60) advance(env, t);
  assert.equal(runs, 0, 'no callbacks while paused');

  env.win.__pwPaused = false;
  // Next deferred gate fires at ~750ms; run past it.
  for (let t = 616; t <= 900; t += 1000 / 60) advance(env, t);
  assert.ok(runs >= 1, 'resumes after unpause');
});

test('multiple callbacks share one allowed frame per timestamp', () => {
  const env = makeEnv();
  install(env);
  env.win.__pwFPSCap = 30;
  let aRuns = 0;
  let bRuns = 0;
  const loopA = () => { aRuns++; env.win.requestAnimationFrame(loopA); };
  const loopB = () => { bRuns++; env.win.requestAnimationFrame(loopB); };
  env.win.requestAnimationFrame(loopA);
  env.win.requestAnimationFrame(loopB);

  for (let t = 0; t <= 1000; t += 1000 / 60) advance(env, t);
  assert.equal(aRuns, bRuns, 'both loops starved equally / passed equally');
  assert.ok(aRuns > 20, `loops did not starve each other (${aRuns})`);
});
