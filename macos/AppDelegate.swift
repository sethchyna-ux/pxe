import Cocoa
import WebKit

class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate {
    var window: NSWindow!
    var webView: WKWebView!
    var retryTimer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        setupMenu()

        // 1. Setup Native Dark macOS Window
        let rect = NSRect(x: 0, y: 0, width: 1150, height: 780)
        window = NSWindow(
            contentRect: rect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.center()
        window.title = "PXE Server Dashboard"
        window.appearance = NSAppearance(named: .darkAqua)
        window.minSize = NSSize(width: 850, height: 550)
        window.delegate = self

        // 2. Setup WebKit View
        let config = WKWebViewConfiguration()
        webView = WKWebView(frame: window.contentView!.bounds, configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = self
        window.contentView!.addSubview(webView)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        // 3. Immediately begin loading dashboard (auto-retries until server is up)
        loadDashboard()

        // 4. Launch backend daemon in background if not already running
        startDaemonInBackground()
    }

    func loadDashboard() {
        if let url = URL(string: "http://127.0.0.1:8080/dashboard") {
            let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 3.0)
            self.webView.load(request)
        }
    }

    // Auto-retry loading whenever connection is refused (e.g. server booting / Touch ID prompt active)
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        retryTimer?.invalidate()
        retryTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: false) { [weak self] _ in
            self?.loadDashboard()
        }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        retryTimer?.invalidate()
        retryTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: false) { [weak self] _ in
            self?.loadDashboard()
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        retryTimer?.invalidate()
        retryTimer = nil
    }

    func startDaemonInBackground() {
        // Check if port 8080 is already alive
        guard let checkUrl = URL(string: "http://127.0.0.1:8080/api/status") else { return }
        var req = URLRequest(url: checkUrl)
        req.timeoutInterval = 0.8

        URLSession.shared.dataTask(with: req) { _, resp, _ in
            if let http = resp as? HTTPURLResponse, http.statusCode == 200 {
                return // Server is already alive and running!
            }

            // Launch daemon with admin privileges on background thread
            DispatchQueue.global(qos: .userInitiated).async {
                let bundlePath = Bundle.main.bundlePath
                var pxeBin = (bundlePath as NSString).appendingPathComponent("Contents/MacOS/pxe")
                if !FileManager.default.fileExists(atPath: pxeBin) {
                    let devPath = "/Users/yocan/pxe/target/release/pxe"
                    pxeBin = FileManager.default.fileExists(atPath: devPath) ? devPath : "/usr/local/bin/pxe"
                }

                let pxeDir = "/Users/yocan/pxe"
                let cmd = "killall -9 pxe 2>/dev/null || true; cd '\(pxeDir)' && '\(pxeBin)' serve --mode standalone --dhcp-range 192.168.1.200,192.168.1.240 --dhcp-router 192.168.1.1 --dhcp-dns 1.1.1.1 --no-tui"
                let script = "do shell script \"\(cmd) > /tmp/pxe_app.log 2>&1 &\" with administrator privileges"

                var error: NSDictionary?
                if let appleScript = NSAppleScript(source: script) {
                    appleScript.executeAndReturnError(&error)
                    if let err = error {
                        print("Admin authorization notice: \(err)")
                    }
                }
            }
        }.resume()
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

    @objc func openInBrowser() {
        if let url = URL(string: "http://127.0.0.1:8080/dashboard") {
            NSWorkspace.shared.open(url)
        }
    }

    @objc func openImagesFolder() {
        let cardPath = "/Volumes/laptopcard/pxe/httpboot"
        let fallbackPath = "/Users/yocan/pxe/httpboot"
        let target = FileManager.default.fileExists(atPath: cardPath) ? cardPath : fallbackPath
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
