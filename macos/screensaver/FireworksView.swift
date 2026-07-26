import AppKit
import ScreenSaver
import os.log

private let log = OSLog(subsystem: "com.jasonslay.fireworks.screensaver", category: "main")

/// Thin ScreenSaverView that launches the bundled Fireworks binary fullscreen.
///
/// Preview mode draws a static title instead of starting Bevy (too heavy for the
/// System Settings thumbnail). Only the main display starts the engine so
/// multi-monitor setups don't spawn duplicate processes.
@objc(FireworksView)
public final class FireworksView: ScreenSaverView {
    private static let newInstanceNotification =
        Notification.Name("com.jasonslay.fireworks.screensaver.NewInstance")

    private var engine: Process?
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
        stopEngine()
        NotificationCenter.default.removeObserver(self)
        DistributedNotificationCenter.default().removeObserver(self)
    }

    public override var hasConfigureSheet: Bool { false }

    public override func startAnimation() {
        super.startAnimation()
        guard !lameDuck else { return }
        guard !isPreviewMode else { return }

        // One engine for the whole desktop; secondary displays stay black.
        if let screen = window?.screen, let main = NSScreen.main, screen != main {
            return
        }

        startEngine()
    }

    public override func stopAnimation() {
        stopEngine()
        super.stopAnimation()
    }

    public override func draw(_ dirtyRect: NSRect) {
        NSColor.black.setFill()
        bounds.fill()

        // Preview thumbnail, or secondary displays / failed launch.
        if engine == nil {
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
        if engine == nil {
            setNeedsDisplay(bounds)
        }
    }

    @objc private func willStop(_ notification: Notification) {
        stopAnimation()
    }

    @objc private func neuter(_ notification: Notification) {
        guard (notification.object as AnyObject?) !== self else { return }
        lameDuck = true
        stopEngine()
        removeFromSuperview()
        NotificationCenter.default.removeObserver(self)
        DistributedNotificationCenter.default().removeObserver(self)
    }

    private func engineURL() -> URL? {
        let bundle = Bundle(for: FireworksView.self)
        let bundled = bundle.bundleURL
            .appendingPathComponent("Contents/MacOS/fireworks", isDirectory: false)
        if FileManager.default.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        return bundle.url(forResource: "fireworks", withExtension: nil)
    }

    private func startEngine() {
        guard engine == nil else { return }
        guard let url = engineURL() else {
            os_log("fireworks binary missing from screensaver bundle", log: log, type: .error)
            return
        }

        let process = Process()
        process.executableURL = url
        var env = ProcessInfo.processInfo.environment
        env["FIREWORKS_SCREENSAVER"] = "1"
        env["FIREWORKS_NATIVE"] = "1"
        process.environment = env
        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                self?.engine = nil
                self?.setNeedsDisplay(self?.bounds ?? .zero)
            }
        }

        do {
            try process.run()
            engine = process
            os_log("started fireworks engine pid=%d", log: log, type: .info, process.processIdentifier)
        } catch {
            os_log(
                "failed to start fireworks: %{public}@",
                log: log,
                type: .error,
                String(describing: error)
            )
        }
    }

    private func stopEngine() {
        guard let process = engine else { return }
        engine = nil
        if process.isRunning {
            process.terminate()
            let pid = process.processIdentifier
            DispatchQueue.global().asyncAfter(deadline: .now() + 2.0) {
                if kill(pid, 0) == 0 {
                    kill(pid, SIGKILL)
                }
            }
        }
    }
}
