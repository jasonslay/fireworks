import AppKit
import ScreenSaver
import WebKit
import os.log

private let log = OSLog(subsystem: "com.jasonslay.fireworks.screensaver", category: "main")

/// ScreenSaverView that hosts the Fireworks WebAssembly build in a WKWebView.
///
/// The legacyScreenSaver sandbox blocks launching the native binary as a child
/// process, so the screensaver embeds the same web bundle used on jtslay.com.
@objc(FireworksView)
public final class FireworksView: ScreenSaverView {
    private static let newInstanceNotification =
        Notification.Name("com.jasonslay.fireworks.screensaver.NewInstance")

    private var webView: WKWebView?
    private var isPreviewMode = false
    private var lameDuck = false

    public override init?(frame: NSRect, isPreview: Bool) {
        super.init(frame: frame, isPreview: isPreview)
        isPreviewMode = isPreview
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        animationTimeInterval = 1.0 / 30.0

        NotificationCenter.default.post(name: Self.newInstanceNotification, object: self)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(neuter(_:)),
            name: Self.newInstanceNotification,
            object: nil
        )
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(willStop(_:)),
            name: Notification.Name("com.apple.screensaver.willstop"),
            object: nil
        )
    }

    public required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    deinit {
        teardownWebView()
        NotificationCenter.default.removeObserver(self)
        DistributedNotificationCenter.default().removeObserver(self)
    }

    public override var hasConfigureSheet: Bool { false }

    public override func startAnimation() {
        super.startAnimation()
        guard !lameDuck else { return }

        // Preview thumbnail stays lightweight; full-screen loads the WASM app.
        guard !isPreviewMode else { return }

        ensureFullSize()
        installWebView()
        webView?.frame = webViewTargetFrame()
    }

    public override func stopAnimation() {
        teardownWebView()
        super.stopAnimation()
    }

    public override func draw(_ dirtyRect: NSRect) {
        NSColor.black.setFill()
        bounds.fill()

        if webView == nil {
            let title = "Fireworks"
            let fontSize = max(12.0, min(bounds.width, bounds.height) * 0.12)
            let attrs: [NSAttributedString.Key: Any] = [
                .foregroundColor: NSColor.white.withAlphaComponent(0.85),
                .font: NSFont.systemFont(ofSize: fontSize, weight: .light),
            ]
            let size = title.size(withAttributes: attrs)
            title.draw(
                at: NSPoint(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2),
                withAttributes: attrs
            )
        }
    }

    public override func animateOneFrame() {
        if webView == nil {
            setNeedsDisplay(bounds)
        }
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        webView?.frame = webViewTargetFrame()
    }

    public override func layout() {
        super.layout()
        webView?.frame = webViewTargetFrame()
    }

    public override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        webView?.frame = webViewTargetFrame()
    }

    public override func resizeSubviews(withOldSize oldSize: NSSize) {
        super.resizeSubviews(withOldSize: oldSize)
        webView?.frame = webViewTargetFrame()
    }

    @objc private func willStop(_ notification: Notification) {
        stopAnimation()
    }

    @objc private func neuter(_ notification: Notification) {
        guard (notification.object as AnyObject?) !== self else { return }
        lameDuck = true
        teardownWebView()
        removeFromSuperview()
        NotificationCenter.default.removeObserver(self)
        DistributedNotificationCenter.default().removeObserver(self)
    }

    private func ensureFullSize() {
        if bounds.width > 1, bounds.height > 1 { return }
        let target = NSScreen.main?.frame.size ?? NSSize(width: 1920, height: 1080)
        setFrameSize(target)
    }

    /// Clamp against screen points — legacyScreenSaver can hand over backing pixels.
    private func webViewTargetFrame() -> NSRect {
        if isPreviewMode, bounds.width > 1, bounds.height > 1 {
            return bounds
        }
        let screenSize = window?.screen?.frame.size
            ?? NSScreen.main?.frame.size
            ?? .zero
        if screenSize.width > 1, screenSize.height > 1 {
            if bounds.width > 1, bounds.height > 1 {
                return NSRect(
                    x: 0,
                    y: 0,
                    width: min(bounds.width, screenSize.width),
                    height: min(bounds.height, screenSize.height)
                )
            }
            return NSRect(origin: .zero, size: screenSize)
        }
        return bounds
    }

    private func webRootURL() -> URL? {
        let bundle = Bundle(for: FireworksView.self)
        if let index = bundle.url(forResource: "index", withExtension: "html", subdirectory: "web") {
            return index.deletingLastPathComponent()
        }
        if let resources = bundle.resourceURL?.appendingPathComponent("web", isDirectory: true) {
            let index = resources.appendingPathComponent("index.html")
            if FileManager.default.fileExists(atPath: index.path) {
                return resources
            }
        }
        return nil
    }

    private func installWebView() {
        guard webView == nil else { return }
        guard let root = webRootURL() else {
            os_log("web bundle missing from screensaver Resources/web", log: log, type: .error)
            return
        }
        let index = root.appendingPathComponent("index.html")

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.preferences.setValue(true, forKey: "allowFileAccessFromFileURLs")
        config.setValue(true, forKey: "allowUniversalAccessFromFileURLs")

        // Screen saver host reports document.visibilityState === "hidden", which
        // pauses rAF / WebGL. Spoof visible so Bevy keeps rendering.
        let visibilityOverride = """
        try {
          Object.defineProperty(Document.prototype, 'hidden', {
            configurable: true, get: function() { return false; }
          });
          Object.defineProperty(Document.prototype, 'visibilityState', {
            configurable: true, get: function() { return 'visible'; }
          });
          document.dispatchEvent(new Event('visibilitychange'));
        } catch (e) {}
        """
        config.userContentController.addUserScript(
            WKUserScript(
                source: visibilityOverride,
                injectionTime: .atDocumentStart,
                forMainFrameOnly: false
            )
        )

        let wv = WKWebView(frame: webViewTargetFrame(), configuration: config)
        if #available(macOS 13.0, *) {
            wv.underPageBackgroundColor = .black
        }
        wv.setValue(false, forKey: "drawsBackground")
        addSubview(wv)
        webView = wv

        wv.loadFileURL(index, allowingReadAccessTo: root)
        os_log("loading fireworks web bundle from %{public}@", log: log, type: .info, root.path)
    }

    private func teardownWebView() {
        guard let wv = webView else { return }
        wv.stopLoading()
        wv.navigationDelegate = nil
        wv.removeFromSuperview()
        webView = nil
    }
}
