// ParticleWall shared script: devicePixelRatio / viewport clamp.
// Injected at document-start with __PW_CAP__ replaced by the host
// (renderScale setting; 2.0 = native retina).
(function () {
  var orig = window.devicePixelRatio || 1;
  try {
    Object.defineProperty(window, 'devicePixelRatio', {
      get: function () { return Math.min(orig, __PW_CAP__); }
    });
  } catch (e) {}
  var s = Math.min(1.0, __PW_CAP__ / 2.0);
  if (s < 1) {
    try {
      Object.defineProperty(window, 'innerWidth', {
        get: function () { return Math.round(document.documentElement.clientWidth * s); }
      });
      Object.defineProperty(window, 'innerHeight', {
        get: function () { return Math.round(document.documentElement.clientHeight * s); }
      });
    } catch (e) {}
  }
})();
