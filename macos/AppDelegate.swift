import Cocoa
import WebKit

class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate {
    var window: NSWindow!
    var webView: WKWebView!
    var retryTimer: Timer?
    var dashboardLoaded = false
    var directProcess: Process?

    static let loadingHTML = """
    <!DOCTYPE html>
    <html>
    <head>
      <meta charset="UTF-8">
      <style>
        body {
          margin: 0;
          height: 100vh;
          display: flex;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          background: #0b0f19;
          color: #f8fafc;
          font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", sans-serif;
          gap: 14px;
        }
        .spinner {
          width: 36px;
          height: 36px;
          border: 3px solid rgba(59, 130, 246, 0.25);
          border-top-color: #3b82f6;
          border-radius: 50%;
          animation: spin 0.8s linear infinite;
        }
        @keyframes spin { to { transform: rotate(360deg); } }
        h2 { font-size: 1.1rem; font-weight: 600; margin: 0; }
        p { font-size: 0.85rem; color: #94a3b8; margin: 0; }
      </style>
    </head>
    <body>
      <div class="spinner"></div>
      <h2>Starting PXE Server Engine…</h2>
      <p id="status">Initializing DHCP (UDP 67/4011), TFTP (UDP 69), and HTTP (TCP 8080)</p>
    </body>
    </html>
    """

    func applicationDidFinishLaunching(_ notification: Notification) {
        setupMenu()

        let darkBg = NSColor(red: 0.043, green: 0.059, blue: 0.098, alpha: 1.0)

        // 1. Setup Native Dark macOS Window
        let rect = NSRect(x: 0, y: 0, width: 1200, height: 800)
        window = NSWindow(
            contentRect: rect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.center()
        window.title = "PXE Server Dashboard"
        window.appearance = NSAppearance(named: .darkAqua)
        window.backgroundColor = darkBg
        window.minSize = NSSize(width: 850, height: 550)
        window.delegate = self
        window.contentView?.wantsLayer = true
        window.contentView?.layer?.backgroundColor = darkBg.cgColor

        // 2. Setup WebKit View (disable white background flash)
        let config = WKWebViewConfiguration()
        webView = WKWebView(frame: window.contentView!.bounds, configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = self
        webView.setValue(false, forKey: "drawsBackground")
        window.contentView!.addSubview(webView)

        // Render immediate dark-mode splash so window is never white
        webView.loadHTMLString(Self.loadingHTML, baseURL: nil)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        // 3. Launch backend daemon in background if not already running, and poll until ready
        startDaemonInBackground()
        scheduleReadinessPoll()
    }

    func scheduleReadinessPoll() {
        retryTimer?.invalidate()
        retryTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.checkBackendAndLoad()
        }
    }

    func checkBackendAndLoad() {
        guard let checkUrl = URL(string: "http://127.0.0.1:8080/api/status") else { return }
        var req = URLRequest(url: checkUrl)
        req.timeoutInterval = 0.5
        req.cachePolicy = .reloadIgnoringLocalCacheData

        URLSession.shared.dataTask(with: req) { [weak self] _, resp, _ in
            guard let self = self else { return }
            if let http = resp as? HTTPURLResponse, http.statusCode == 200 {
                DispatchQueue.main.async {
                    if !self.dashboardLoaded {
                        self.dashboardLoaded = true
                        self.retryTimer?.invalidate()
                        self.retryTimer = nil
                        self.loadDashboard()
                    }
                }
            }
        }.resume()
    }

    func loadDashboard() {
        if let url = URL(string: "http://127.0.0.1:8080/dashboard") {
            let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 5.0)
            self.webView.load(request)
        }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        dashboardLoaded = false
        scheduleReadinessPoll()
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        dashboardLoaded = false
        scheduleReadinessPoll()
    }

    func resolvePaths() -> (String, String) {
        let bundlePath = Bundle.main.bundlePath
        let bundleParent = (bundlePath as NSString).deletingLastPathComponent
        var pxeBin = (bundlePath as NSString).appendingPathComponent("Contents/MacOS/pxe")
        if !FileManager.default.fileExists(atPath: pxeBin) {
            let candidateBins = [
                "/Volumes/remove/pxe/target/release/pxe",
                "/Volumes/laptopcard/pxe/target/release/pxe",
                (NSHomeDirectory() as NSString).appendingPathComponent("pxe/target/release/pxe"),
                "/usr/local/bin/pxe"
            ]
            for bin in candidateBins where FileManager.default.fileExists(atPath: bin) {
                pxeBin = bin
                break
            }
        }

        let candidateDirs = [
            bundleParent,
            "/Volumes/remove/pxe",
            "/Volumes/laptopcard/pxe",
            (NSHomeDirectory() as NSString).appendingPathComponent("pxe"),
            "/Users/yocan/pxe"
        ]
        var pxeDir = "/Volumes/remove/pxe"
        for dir in candidateDirs {
            let tftpCheck = (dir as NSString).appendingPathComponent("tftpboot")
            if FileManager.default.fileExists(atPath: tftpCheck) {
                pxeDir = dir
                break
            }
        }
        return (pxeBin, pxeDir)
    }

    func startDaemonInBackground(mode: String = "standalone", forceRestart: Bool = false) {
        if forceRestart {
            dashboardLoaded = false
            webView.loadHTMLString(Self.loadingHTML, baseURL: nil)
            scheduleReadinessPoll()
            launchDaemon(mode: mode, forceRestart: true)
            return
        }

        guard let checkUrl = URL(string: "http://127.0.0.1:8080/api/status") else { return }
        var req = URLRequest(url: checkUrl)
        req.timeoutInterval = 0.5

        URLSession.shared.dataTask(with: req) { [weak self] _, resp, _ in
            if let http = resp as? HTTPURLResponse, http.statusCode == 200 {
                DispatchQueue.main.async {
                    self?.dashboardLoaded = true
                    self?.retryTimer?.invalidate()
                    self?.loadDashboard()
                }
                return
            }
            self?.launchDaemon(mode: mode, forceRestart: false)
        }.resume()
    }

    func launchDaemon(mode: String, forceRestart: Bool) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let (pxeBin, pxeDir) = self.resolvePaths()
            let modeVal = (mode == "standalone") ? "standalone" : "proxy"

            if forceRestart {
                if let url = URL(string: "http://127.0.0.1:8080/api/shutdown") {
                    var req = URLRequest(url: url)
                    req.httpMethod = "POST"
                    req.timeoutInterval = 0.5
                    let sem = DispatchSemaphore(value: 0)
                    URLSession.shared.dataTask(with: req) { _, _, _ in sem.signal() }.resume()
                    _ = sem.wait(timeout: .now() + 0.6)
                }
                self.directProcess?.terminate()
                usleep(200_000)
            }

            // 1. Try direct unprivileged launch first (fast, no Touch ID prompt if UDP 67/69 are available)
            let userLogPath = (NSTemporaryDirectory() as NSString).appendingPathComponent("pxe_user.log")
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: pxeBin)
            proc.currentDirectoryURL = URL(fileURLWithPath: pxeDir)
            proc.arguments = [
                "serve",
                "--bind-ip", "10.77.0.3",
                "--mode", modeVal,
                "--dhcp-range", "10.77.0.2,10.77.0.50",
                "--dhcp-mask", "255.255.0.0",
                "--dhcp-router", "10.77.0.3",
                "--dhcp-dns", "10.77.0.3",
                "--no-tui"
            ]
            try? FileManager.default.removeItem(atPath: userLogPath)
            FileManager.default.createFile(atPath: userLogPath, contents: nil)
            if let logHandle = FileHandle(forWritingAtPath: userLogPath) {
                proc.standardOutput = logHandle
                proc.standardError = logHandle
            }

            do {
                try proc.run()
                self.directProcess = proc
            } catch {
                self.launchPrivilegedDaemon(pxeBin: pxeBin, pxeDir: pxeDir, modeVal: modeVal)
                return
            }

            // Wait 700ms and verify HTTP 8080 is up and DHCP/TFTP did not fail to bind
            usleep(700_000)
            let logText = (try? String(contentsOfFile: userLogPath, encoding: .utf8)) ?? ""
            let bindFailed = !proc.isRunning
                || logText.contains("Failed to bind DHCP")
                || logText.contains("Failed to bind TFTP")
                || logText.contains("Permission denied")
                || logText.contains("Address already in use")

            if bindFailed {
                proc.terminate()
                self.launchPrivilegedDaemon(pxeBin: pxeBin, pxeDir: pxeDir, modeVal: modeVal)
            }
        }
    }

    func launchPrivilegedDaemon(pxeBin: String, pxeDir: String, modeVal: String) {
        // NOTE: Do NOT use 'nohup' inside NSAppleScript 'do shell script' on macOS —
        // macOS /usr/bin/nohup exits with 'nohup: can't detach from console: No such process' when no tty exists.
        let cmd = "launchctl bootout system/com.apple.bootpd 2>/dev/null || true; killall -9 pxe 2>/dev/null || true; cd '\(pxeDir)' && '\(pxeBin)' serve --bind-ip 10.77.0.3 --mode \(modeVal) --dhcp-range 10.77.0.2,10.77.0.50 --dhcp-mask 255.255.0.0 --dhcp-router 10.77.0.3 --dhcp-dns 10.77.0.3 --no-tui </dev/null >/tmp/pxe_app.log 2>&1 &"
        let script = "do shell script \"\(cmd)\" with administrator privileges"

        var error: NSDictionary?
        if let appleScript = NSAppleScript(source: script) {
            appleScript.executeAndReturnError(&error)
            if let err = error {
                print("Admin authorization notice: \(err)")
            }
        }
    }

    func setupMenu() {
        let mainMenu = NSMenu()

        // App Menu
        let appMenuItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About PXE Server", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Hide PXE Server", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Quit PXE Server", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        // Server Menu
        let serverMenuItem = NSMenuItem()
        let serverMenu = NSMenu(title: "Server")
        serverMenu.addItem(withTitle: "Reload Dashboard", action: #selector(reloadDashboard), keyEquivalent: "r")
        serverMenu.addItem(withTitle: "Open in Web Browser", action: #selector(openInBrowser), keyEquivalent: "b")
        serverMenu.addItem(NSMenuItem.separator())
        serverMenu.addItem(withTitle: "Restart in ProxyDHCP Mode (Home Router LAN)", action: #selector(restartProxyMode), keyEquivalent: "p")
        serverMenu.addItem(withTitle: "Restart in Standalone DHCP Mode (Direct Cable)", action: #selector(restartStandaloneMode), keyEquivalent: "s")
        serverMenu.addItem(NSMenuItem.separator())
        serverMenu.addItem(withTitle: "Open Boot Images on Card", action: #selector(openImagesFolder), keyEquivalent: "o")
        serverMenuItem.submenu = serverMenu
        mainMenu.addItem(serverMenuItem)

        // Window Menu
        let windowMenuItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.miniaturize(_:)), keyEquivalent: "m")
        windowMenu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        windowMenuItem.submenu = windowMenu
        mainMenu.addItem(windowMenuItem)

        NSApp.mainMenu = mainMenu
    }

    @objc func reloadDashboard() {
        self.loadDashboard()
    }

    @objc func restartProxyMode() {
        self.startDaemonInBackground(mode: "proxy", forceRestart: true)
    }

    @objc func restartStandaloneMode() {
        self.startDaemonInBackground(mode: "standalone", forceRestart: true)
    }

    @objc func openInBrowser() {
        if let url = URL(string: "http://127.0.0.1:8080/dashboard") {
            NSWorkspace.shared.open(url)
        }
    }

    @objc func openImagesFolder() {
        let candidatePaths = [
            "/Volumes/laptopcard/pxe/httpboot",
            "/Volumes/remove/pxe/httpboot",
            (NSHomeDirectory() as NSString).appendingPathComponent("pxe/httpboot")
        ]
        var target = "/Volumes/remove/pxe/httpboot"
        for path in candidatePaths where FileManager.default.fileExists(atPath: path) {
            target = path
            break
        }
        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: target)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return true
    }
}

// Main Entrypoint
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
