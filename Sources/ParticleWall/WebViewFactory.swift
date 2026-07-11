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
      window.requestAnimationFrame = function (cb) {
        function gate(t) {
          if (window.__pwPaused) {
            setTimeout(function () { raf(gate); }, 250);
            return;
          }
          var cap = window.__pwFPSCap;
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
          cb(t);
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
