import WebKit

/// Blocks any navigation that is not a local file inside the allowed root.
final class LocalOnlyNavigationDelegate: NSObject, WKNavigationDelegate {
    let allowedRoot: URL

    init(allowedRoot: URL) {
        self.allowedRoot = allowedRoot.standardizedFileURL
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
      var last = 0;
      window.requestAnimationFrame = function (cb) {
        function gate(t) {
          window.__pwFrameCount = (window.__pwFrameCount | 0) + 1;
          if (window.__pwPaused) { raf(gate); return; }
          var cap = window.__pwFPSCap;
          if (cap > 0) {
            var min = 1000 / cap;
            if (t - last < min - 0.5) { raf(gate); return; }
            last = t;
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
                      '*{user-select:none !important;-webkit-user-select:none !important;}';
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
