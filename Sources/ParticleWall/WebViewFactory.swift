import WebKit

/// Blocks any navigation that is not a local file inside the allowed root.
final class LocalOnlyNavigationDelegate: NSObject, WKNavigationDelegate {
    let allowedRoot: URL
    var onDidFinish: (() -> Void)?

    init(allowedRoot: URL) {
        self.allowedRoot = allowedRoot.standardizedFileURL
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        onDidFinish?()
    }

    func webView(_ webView: WKWebView,
                 decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        if url.scheme == "about" || url.scheme == "blob" || url.scheme == "data" {
            decisionHandler(.allow)
            return
        }
        if url.isFileURL {
            let path = url.standardizedFileURL.path
            if path.hasPrefix(allowedRoot.path) {
                decisionHandler(.allow)
                return
            }
        }
        NSLog("ParticleWall: blocked navigation to \(url)")
        decisionHandler(.cancel)
    }
}

enum WebViewFactory {

    /// Caps the devicePixelRatio wallpapers see, so renderers that call
    /// setPixelRatio(devicePixelRatio) draw fewer pixels on retina screens.
    /// Additionally, below native scale the reported innerWidth/innerHeight
    /// shrink by renderScale/2 — exports that size their canvas from those
    /// (ignoring devicePixelRatio) also render fewer pixels; hardenScript's CSS
    /// stretches the canvas back to full screen.
    static func dprClampScript(cap: Double) -> String {
        let sizeFactor = min(1.0, cap / 2.0)
        return """
        (function () {
          var orig = window.devicePixelRatio || 1;
          try {
            Object.defineProperty(window, 'devicePixelRatio', {
              get: function () { return Math.min(orig, \(cap)); }
            });
          } catch (e) {}
          var s = \(sizeFactor);
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
        """
    }

    /// Patches requestAnimationFrame so any wallpaper honors __pwPaused / __pwFPSCap
    /// without cooperating.
    static let rafPatchScript = """
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
      window.addEventListener('error', function (e) {
        window.__pwErrors.push(String(e.message || e.error || 'unknown error'));
      });
      window.addEventListener('unhandledrejection', function (e) {
        window.__pwErrors.push('unhandled rejection: ' + String(e.reason));
      });
      var raf = window.requestAnimationFrame.bind(window);
      // Throttle per display frame, not per callback: rAF callbacks within one
      // frame share the same timestamp, so `allowedT` lets every callback of an
      // allowed frame through (otherwise concurrent loops starve each other).
      // Skipped ticks sleep via setTimeout instead of re-queueing rAF, so WebKit
      // wakes ~cap times/s (not 120/s) and ProMotion can drop the panel refresh.
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
    """

    static let hardenScript = """
    (function () {
      document.addEventListener('contextmenu', function (e) { e.preventDefault(); }, true);
      document.addEventListener('selectstart', function (e) { e.preventDefault(); }, true);
      document.addEventListener('dragstart', function (e) { e.preventDefault(); }, true);
      var s = document.createElement('style');
      s.textContent = 'html,body{overflow:hidden !important;margin:0;padding:0;}' +
                      '*{user-select:none !important;-webkit-user-select:none !important;}' +
                      // Canvas may render at reduced resolution (innerWidth patch);
                      // stretch it to full screen regardless of its inline size.
                      'body > canvas{width:100vw !important;height:100vh !important;}';
      (document.head || document.documentElement).appendChild(s);
    })();
    """

    static func makeConfiguration() -> WKWebViewConfiguration {
        let config = WKWebViewConfiguration()
        config.suppressesIncrementalRendering = false
        // ES modules under file:// are CORS-blocked without this; wallpapers using
        // <script type="module"> (import-map pipeline) need it.
        config.preferences.setValue(true, forKey: "allowFileAccessFromFileURLs")
        #if DEBUG
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        #endif
        let controller = WKUserContentController()
        let scale = UserDefaults.standard.double(forKey: DefaultsKey.renderScale)
        controller.addUserScript(WKUserScript(source: dprClampScript(cap: scale > 0 ? scale : 2.0),
                                              injectionTime: .atDocumentStart,
                                              forMainFrameOnly: false))
        controller.addUserScript(WKUserScript(source: rafPatchScript,
                                              injectionTime: .atDocumentStart,
                                              forMainFrameOnly: false))
        controller.addUserScript(WKUserScript(source: hardenScript,
                                              injectionTime: .atDocumentEnd,
                                              forMainFrameOnly: true))
        config.userContentController = controller
        return config
    }

    static func makeWebView(frame: CGRect) -> WKWebView {
        let webView = WKWebView(frame: frame, configuration: makeConfiguration())
        webView.autoresizingMask = [.width, .height]
        webView.setValue(false, forKey: "drawsBackground") // transparent until content paints
        webView.allowsMagnification = false
        webView.allowsBackForwardNavigationGestures = false
        return webView
    }
}
