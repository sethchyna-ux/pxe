import Cocoa
import WebKit

class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var window: NSWindow!
    var webView: WKWebView!
    var pxeProcess: Process?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Configure App Menu
        setupMenu()

        // Setup Main Window
        let rect = NSRect(x: 0, y: 0, width: 1150, height: 780)
        window = NSWindow(
            contentRect: rect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.center()
        window.title = "PXE Server Dashboard"
        window.titlebarAppearsTransparent = true
        window.appearance = NSAppearance(named: .darkAqua)
        window.minSize = NSSize(width: 850, height: 550)
        window.delegate = self

        // Setup WebKit View
        let config = WKWebViewConfiguration()
        webView = WKWebView(frame: window.contentView!.bounds, configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.setValue(false, forKey: "drawsBackground") // Transparent background until loaded
        window.contentView!.addSubview(webView)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        // Ensure Server is running, then load dashboard
        ensureServerRunning { [weak self] success in
            DispatchQueue.main.async {
                self?.loadDashboard()
            }
        }
    }

    func loadDashboard() {
        if let url = URL(string: "http://127.0.0.1:8080/dashboard") {
            let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 10.0)
            self.webView.load(request)
        }
    }

    func ensureServerRunning(completion: @escaping (Bool) -> Void) {
        // 1. Check if server already active on 8080
        checkServerActive { active in
            if active {
                print("PXE server already active on port 8080.")
                completion(true)
                return
            }

            // 2. Locate pxe binary
            let bundlePath = Bundle.main.bundlePath
            var pxeBin = (bundlePath as NSString).appendingPathComponent("Contents/MacOS/pxe")
            if !FileManager.default.fileExists(atPath: pxeBin) {
                // Fallback to local dev paths
                let devPath = "/Users/yocan/pxe/target/release/pxe"
                if FileManager.default.fileExists(atPath: devPath) {
                    pxeBin = devPath
                } else {
                    pxeBin = "/usr/local/bin/pxe"
                }
            }

            let pxeDir = "/Users/yocan/pxe"
            let cmd = "killall -9 pxe 2>/dev/null || true; cd \(pxeDir) && \(pxeBin) serve --mode standalone --dhcp-range 192.168.1.200,192.168.1.240 --dhcp-router 192.168.1.1 --dhcp-dns 1.1.1.1 --no-tui"

            // Elevate privileges using AppleScript for low UDP ports (67, 69)
            let script = "do shell script \"\(cmd) > /tmp/pxe_app.log 2>&1 &\" with administrator privileges"
            var error: NSDictionary?
            if let appleScript = NSAppleScript(source: script) {
                appleScript.executeAndReturnError(&error)
                if let err = error {
                    print("Admin elevation cancelled or error: \(err)")
                }
            }

            // Wait up to 3 seconds for server to initialize
            DispatchQueue.global().async {
                for _ in 0..<15 {
                    usleep(200_000) // 200ms
                    var isUp = false
                    let sema = DispatchSemaphore(value: 0)
                    self.checkServerActive { up in
                        isUp = up
                        sema.signal()
                    }
                    sema.wait()
                    if isUp {
                        completion(true)
                        return
                    }
                }
                completion(false)
            }
        }
    }

    func checkServerActive(completion: @escaping (Bool) -> Void) {
        guard let url = URL(string: "http://127.0.0.1:8080/api/status") else {
            completion(false)
            return
        }
        var req = URLRequest(url: url)
        req.timeoutInterval = 0.8
        let task = URLSession.shared.dataTask(with: req) { _, resp, _ in
            if let http = resp as? HTTPURLResponse, http.statusCode == 200 {
                completion(true)
            } else {
                completion(false)
            }
        }
        task.resume()
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
        self.webView.reload()
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

    func applicationWillTerminate(_ notification: Notification) {
        // Optional cleanup on quit
    }
}

// Main Entrypoint
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
