// ParticleWall shared script: requestAnimationFrame gate.
// Source of truth for both macOS (WKWebView) and Linux (WebKitGTK).
// Injected at document-start by each host. Contract:
//   window.__pwPaused       bool    freeze rendering
//   window.__pwFPSCap       int     0 = unlimited
//   window.__pwAdaptiveEnabled bool
//   window.__pwAdaptiveCap  int     set by adaptive quality logic
(function () {
  if (window.__pwInstalled) return;
  window.__pwInstalled = true;
  window.__pwPaused = false;
  window.__pwFPSCap = 0;
  window.__pwAdaptiveEnabled = true;
  window.__pwAdaptiveCap = 0;
  window.__pwQualityLevel = 1;
  window.__pwAverageFrameCost = 0;
  window.__pwErrors = [];
  function recordError(message) {
    window.__pwErrors.push(String(message).slice(0, 2048));
    if (window.__pwErrors.length > 100) window.__pwErrors.shift();
  }
  window.addEventListener('error', function (e) {
    recordError(e.message || e.error || 'unknown error');
  });
  window.addEventListener('unhandledrejection', function (e) {
    recordError('unhandled rejection: ' + String(e.reason));
  });
  var raf = window.requestAnimationFrame.bind(window);
  // Throttle per display frame, not per callback: rAF callbacks within one
  // frame share the same timestamp, so `allowedT` lets every callback of an
  // allowed frame through (otherwise concurrent loops starve each other).
  // Skipped ticks sleep via setTimeout instead of re-queueing rAF, so the
  // engine wakes ~cap times/s instead of once per display refresh.
  var lastPass = -1e9;
  var allowedT = -1;
  var adaptiveSamples = 0;
  var stableWindows = 0;
  var averageCost = 0;
  var previousAllowedT = 0;
  var adaptiveCapSteps = [60, 30, 24, 20, 15];

  function updateAdaptiveQuality(callbackCost, timestamp) {
    if (!window.__pwAdaptiveEnabled) {
      window.__pwAdaptiveCap = 0;
      window.__pwQualityLevel = 1;
      return;
    }
    averageCost = averageCost ? averageCost * 0.94 + callbackCost * 0.06 : callbackCost;
    window.__pwAverageFrameCost = averageCost;
    adaptiveSamples++;
    if (adaptiveSamples < 90) return;
    adaptiveSamples = 0;

    var requested = window.__pwFPSCap > 0 ? window.__pwFPSCap : 60;
    var current = window.__pwAdaptiveCap > 0 ? window.__pwAdaptiveCap : requested;
    var budget = 1000 / Math.max(1, current);
    var actualInterval = previousAllowedT > 0 ? timestamp - previousAllowedT : budget;
    var overloaded = averageCost > budget * 0.72 || actualInterval > budget * 1.45;

    if (overloaded) {
      stableWindows = 0;
      var lower = adaptiveCapSteps.find(function (step) { return step < current; });
      if (lower) window.__pwAdaptiveCap = Math.min(requested, lower);
    } else {
      stableWindows++;
      // Recovery is intentionally slow to avoid oscillating quality.
      if (stableWindows >= 8 && current < requested) {
        var candidates = adaptiveCapSteps.filter(function (step) {
          return step > current && step <= requested;
        });
        window.__pwAdaptiveCap = candidates.length ? candidates[candidates.length - 1] : 0;
        stableWindows = 0;
      }
    }
    var effective = window.__pwAdaptiveCap || requested;
    window.__pwQualityLevel = Math.max(0.5, Math.min(1, effective / requested));
    window.dispatchEvent(new CustomEvent('particlewallqualitychange', {
      detail: { fpsCap: window.__pwAdaptiveCap, quality: window.__pwQualityLevel }
    }));
  }

  window.requestAnimationFrame = function (cb) {
    function gate(t) {
      if (window.__pwPaused) {
        setTimeout(function () { raf(gate); }, 250);
        return;
      }
      var requestedCap = window.__pwFPSCap;
      var adaptiveCap = window.__pwAdaptiveEnabled ? window.__pwAdaptiveCap : 0;
      var cap = requestedCap > 0 && adaptiveCap > 0
        ? Math.min(requestedCap, adaptiveCap)
        : (requestedCap || adaptiveCap);
      if (t !== allowedT) {
        var min = cap > 0 ? 1000 / cap : 0;
        if (cap > 0 && t - lastPass < min - 0.5) {
          var wait = Math.max(0, min - (t - lastPass) - 2);
          setTimeout(function () { raf(gate); }, wait);
          return;
        }
        lastPass = t;
        allowedT = t;
        window.__pwFrameCount = (window.__pwFrameCount | 0) + 1;
      }
      var callbackStart = performance.now();
      cb(t);
      var callbackCost = performance.now() - callbackStart;
      updateAdaptiveQuality(callbackCost, t);
      previousAllowedT = t;
    }
    return raf(gate);
  };
})();
