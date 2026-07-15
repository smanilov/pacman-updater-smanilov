const Applet = imports.ui.applet;
const Gio = imports.gi.Gio;
const Main = imports.ui.main;
const Mainloop = imports.mainloop;
const PopupMenu = imports.ui.popupMenu;
const St = imports.gi.St;
const Util = imports.misc.util;

class PacmanUpdater extends Applet.IconApplet {
////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                         CONSTRUCTOR/DESTRUCTOR                             //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////
    constructor(metadata, orientation, panel_height, instance_id) {
        super(orientation, panel_height, instance_id);

        log("pacman updater started -- version 1.2.0");

        /** @type {PopupMenu.PopupMenuManager|null} */
        this.menuManager = null;
        /** @type {PopupMenu.PopupMenu|null} */
        this.menu = null;
        /** @type {PopupMenu.PopupSwitchMenuItem|null} */
        this.toggleLoopItem = null;

        // bool
        this._loopEnabled = true;
        // bool
        this._hasNetwork = false;

        /** @type {number|null} */
        this._networkWatcherId = null;
        /** @type {string|null} usage message when package-managers.json is missing */
        this._configError = null;
        /** @type {object|null} */
        this._depgraph = null;
        /** @type {string} */
        this._appletPath = metadata.path;
        /** @type {string} */
        this._depgraphPath = metadata.path + '/depgraph.json';

        /**
         * Package managers from package-managers.json, each extended with
         * runtime state: _timeout, _updateCount, _impactedCount, _checking,
         * _error.
         * @type {object[]}
         */
        this._managers = this.loadPackageManagers();

        this.set_applet_icon_name("face-smile");

        this.buildMenu(orientation);

        if (this._configError) {
            // without a config no check will run, so show the usage
            // message in the tooltip right away
            this._updateTooltip();
        }

        this.updateDepgraph();
    }

    // destructor
    on_applet_removed_from_panel() {
        if (this.isNetworkWatcherEnabled()) {
            this.disableNetworkWatcher();
        }
        if (this.isLoopRunning()) {
            this.stopLoop();
        }
        this.menu.destroy();
        log("pacman updater stopped");
    }

    // event handler; not called explicitly in this file
    on_applet_clicked(event) {
        log("applet clicked");
        this.menu.toggle();
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                           PACKAGE MANAGERS                                 //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    loadPackageManagers() {
        let path = this._appletPath + '/package-managers.json';
        let parsed;
        try {
            // Gio and JSON.parse throw on missing/unreadable/invalid input
            let file = Gio.File.new_for_path(path);
            let [ok, contents] = file.load_contents(null);
            parsed = JSON.parse(new TextDecoder().decode(contents));
        } catch (e) {
            logError(`failed to load ${path}: ${e}`);
            log("usage: copy example-package-managers.json to " +
                "package-managers.json and adjust it for this system");
            this._configError = "no package-managers.json found;\n" +
                "copy example-package-managers.json to package-managers.json";
            return [];
        }
        let configs = parsed.package_managers;
        if (!Array.isArray(configs) || configs.length === 0) {
            logError(`${path}: no package_managers entries`);
            this._configError = "package-managers.json has no " +
                "package_managers entries;\nsee example-package-managers.json";
            return [];
        }
        let managers = configs.map(config => Object.assign({}, config, {
            _timeout: null,
            _updateCount: 0,
            _impactedCount: 0,
            _checking: false,
            _error: null,
        }));
        log(`package managers loaded: ${managers.map(m => m.name).join(', ')}`);
        return managers;
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                                SETTERS                                     //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    setLoopEnabled(loopEnabled) {
        this._loopEnabled = loopEnabled;
        this._updateTooltip();
    }

    setManagerUpdateCounts(manager, updateCount, impactedCount) {
        manager._updateCount = updateCount;
        manager._impactedCount = impactedCount;
        this._updateTooltip();
    }

    setManagerChecking(manager, checking) {
        manager._checking = checking;
        this._updateTooltip();
    }

    setManagerError(manager, errorMessage) {
        manager._error = errorMessage;
        this._updateTooltip();
    }

    setHasNetwork(hasNetwork) {
        this._hasNetwork = hasNetwork;
        this._updateTooltip();
    }

    // Whether this manager's updates get impact counts from the depgraph:
    // either it sources the depgraph (pacman) or its updated packages are
    // installed via pacman and thus present in it (yay/AUR).
    _usesDepgraph(manager) {
        return !!(manager.provides_depgraph || manager.uses_depgraph);
    }

    // e.g. "5 (12)" for depgraph-using managers, "5" otherwise
    _formatManagerCount(manager) {
        if (this._usesDepgraph(manager)) {
            return `${manager._updateCount} (${manager._impactedCount})`;
        }
        return `${manager._updateCount}`;
    }

    _managerStatusLine(manager) {
        if (manager.self_managed) {
            return `${manager.name}: self-managed`;
        }
        if (manager._checking) {
            return `${manager.name}: checking for updates...`;
        }
        if (manager._error) {
            return `${manager.name}: ${manager._error}`;
        }
        if (manager._updateCount === 0) {
            return `${manager.name}: no updates`;
        }
        return `${manager.name}: ${this._formatManagerCount(manager)}`;
    }

    _updateTooltip() {
        let loopState = this._loopEnabled ? "running" : "not running";
        let lines = [`loop is ${loopState}`];
        if (this._configError) {
            lines.push(this._configError);
        } else if (!this._hasNetwork) {
            lines.push("no network connection");
        } else {
            for (let manager of this._managers) {
                lines.push(this._managerStatusLine(manager));
            }
        }
        this.set_applet_tooltip(lines.join('\n'));
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                              MENU LOGIC                                    //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    buildMenu(orientation) {
        this.menuManager = new PopupMenu.PopupMenuManager(this);
        this.menu = new Applet.AppletPopupMenu(this, orientation);
        this.menuManager.addMenu(this.menu);

        this.toggleLoopItem = new PopupMenu.PopupSwitchMenuItem(
            "Enable update loop",
            this._loopEnabled
        );

        this.toggleLoopItem.connect('toggled', (item, state) => {
            this.setLoopEnabled(state);
            if (state) {
                log("Update loop enabled");
                this.startLoop();
            } else {
                log("Update loop disabled");
                this.stopLoop();
            }
        });
        this.menu.addMenuItem(this.toggleLoopItem);

        this.menu.addAction(_("Run pacman-update-viewer"), () => {
            this.launchUpdateTerminal();
        });

        log("menu built");
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                            LOOP MANAGEMENT                                 //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    isLoopRunning() {
        return this._managers.some(m => m._timeout);
    }

    startLoop() {
        if (this.isLoopRunning()) {
            log("warning: corrupt state; already running");
            return;
        }
        for (let manager of this._managers) {
            if (manager.self_managed) continue;
            this.checkUpdates(manager);
            let intervalSeconds = (manager.check_interval_minutes || 10) * 60;
            manager._timeout = Mainloop.timeout_add_seconds(intervalSeconds, () => {
                this.checkUpdates(manager);
                return true;
            });
        }

        log("loop started");
    }

    stopLoop() {
        if (!this.isLoopRunning()) {
            log("warning: corrupt state; not running");
            return;
        }

        for (let manager of this._managers) {
            if (manager._timeout) {
                Mainloop.source_remove(manager._timeout);
                manager._timeout = null;
            }
        }

        log("loop stopped");
    }

    restartLoop() {
        if (this.isLoopRunning()) {
            this.stopLoop();
        }
        this.startLoop();
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                               RUN UPDATE                                   //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    launchUpdateTerminal() {
        log("opening terminal...");

        let viewerPath = this._appletPath +
            '/pacman-update-viewer/target/release/pacman-update-viewer';

        // --wait makes gnome-terminal block until the shell exits, so the
        // spawnCommandLineAsync callback fires only after the update finishes.
        Util.spawnCommandLineAsync(
            `gnome-terminal --wait -- ${viewerPath}`,
            () => {
                log("update terminal exited successfully");
                this.updateDepgraph();
            },
            () => {
                log("update terminal exited with error");
            }
        );

        log("terminal opened");
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                             CHECK UPDATES                                  //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    checkUpdates(manager) {
        if (!this.hasNetwork()) {
            log("no network connection, skipping update check");
            this.setHasNetwork(false);
            if (!this.isNetworkWatcherEnabled()) {
                this.enableNetworkWatcher();
            }
            return;
        } else {
            log("network connection detected");
            this.setHasNetwork(true);
        }

        log(`checking for updates (${manager.name})...`);
        this.setManagerChecking(manager, true);

        let proc = new Gio.Subprocess({
            argv: ['bash', '-c', manager.check_cmd],
            flags: Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE
        });

        proc.init(null);

        proc.communicate_utf8_async(null, null, (proc, res) => {
            try {
                let [ok, stdout, stderr] = proc.communicate_utf8_finish(res);
                log(`check complete (${manager.name})`);
                this.setManagerChecking(manager, false);
                let status = proc.get_exit_status();
                if (status === 127) {
                    // bash exits 127 when the command is not found
                    logError(`error: ${stderr}`);
                    let hint = manager.check_cmd.startsWith('checkupdates') ?
                        "pacman-contrib not installed" :
                        `'${manager.check_cmd.split(/\s+/)[0]}' not installed`;
                    this.setManagerError(manager, `error: ${hint}`);
                } else if (status === manager.check_no_updates_exit_code) {
                    this.setManagerError(manager, null);
                    this.handleCheckOutput(manager, "");
                } else if (status !== 0 || stderr) {
                    logError(`error: ${stderr}`);
                    let message = stderr ? stderr.trim() : `exit code ${status}`;
                    this.setManagerError(manager, `error: ${message}`);
                } else {
                    this.setManagerError(manager, null);
                    this.handleCheckOutput(manager, stdout.trim());
                }
            } catch (e) {
                logError(`exception during update check (${manager.name}): ${e}`);
                this.setManagerChecking(manager, false);
                this.setManagerError(manager, `error: ${e.message || e}`);
            }
        });
    }

    // Count pending updates from check_cmd output according to check_parse,
    // returning [count, pkgNames]. pkgNames is only populated for "lines"
    // parsing, where each line starts with a package name.
    _parseCheckOutput(manager, cmdOutput) {
        let lines = cmdOutput ? cmdOutput.split('\n').filter(l => l.trim()) : [];
        let parse = manager.check_parse || "lines";
        if (parse === "lines") {
            return [lines.length, lines.map(l => l.split(/\s+/)[0])];
        }
        if (parse.count_regex) {
            let regex = new RegExp(parse.count_regex);
            return [lines.filter(l => regex.test(l)).length, []];
        }
        logError(`unknown check_parse for ${manager.name}; assuming "lines"`);
        return [lines.length, lines.map(l => l.split(/\s+/)[0])];
    }

    handleCheckOutput(manager, cmdOutput) {
        let [total, pkgNames] = this._parseCheckOutput(manager, cmdOutput);
        let impacted = this._usesDepgraph(manager) ?
            this._allImpactedOf(pkgNames).size : 0;
        this.setManagerUpdateCounts(manager, total, impacted);
        log(`updates available: ${this._managerStatusLine(manager)}`);
        if (total > 0) {
            this._notifyUpdates();
        }
    }

    // Notification shows the per-manager breakdown of pending updates,
    // e.g. "updates available: pacman: 5 (12), aur: 2".
    _notifyUpdates() {
        let parts = this._managers
            .filter(m => !m.self_managed && m._updateCount > 0)
            .map(m => `${m.name}: ${this._formatManagerCount(m)}`);
        if (parts.length === 0) return;
        Main.notify("Pacman Updater", `updates available: ${parts.join(', ')}`);
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                            NETWORK WATCHER                                 //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    hasNetwork() {
        let monitor = Gio.NetworkMonitor.get_default();
        return monitor.get_network_available();
    }

    getNetworkConnectivity() {
        let monitor = Gio.NetworkMonitor.get_default();
        let connectivity = monitor.get_connectivity();
        let label = "";
        switch (connectivity) {
            case Gio.NetworkConnectivity.NONE:
                label = "none";
                break;
            case Gio.NetworkConnectivity.LOCAL:
                label = "local";
                break;
            case Gio.NetworkConnectivity.LIMITED:
                label = "limited";
                break;
            case Gio.NetworkConnectivity.FULL:
                label = "full";
                break;
            default:
                label = "unknown";
                break;
        }
        log(`network connectivity: ${label}`);
        return connectivity;
    }

    // register a watcher to 'network-changed' events and store it in
    // this._networkWatcherId
    enableNetworkWatcher() {
        log("enabling network watcher...");
        if (this.isNetworkWatcherEnabled()) {
            log("warning: corrupt state; network watcher already enabled");
            return;
        }
        let monitor = Gio.NetworkMonitor.get_default();
        this._networkWatcherId = monitor.connect('network-changed', (monitor, available) => {
            log(`network available: ${available}`);
            if (available) {
                let connectivity = this.getNetworkConnectivity();
                if (connectivity == Gio.NetworkConnectivity.FULL) {
                    this.restartLoop();
                    this.disableNetworkWatcher();
                }
            }
        });
    }

    disableNetworkWatcher() {
        log("disabling network watcher...");
        if (!this.isNetworkWatcherEnabled()) {
            log("warning: corrupt state; network watcher not enabled");
            return;
        }

        let monitor = Gio.NetworkMonitor.get_default();
        monitor.disconnect(this._networkWatcherId);
        this._networkWatcherId = null;
    }

    isNetworkWatcherEnabled() {
        return this._networkWatcherId;
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                               DEPGRAPH                                     //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    updateDepgraph() {
        log("updating depgraph...");
        let proc = new Gio.Subprocess({
            argv: ['pacman', '-Qi'],
            flags: Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE
        });
        proc.init(null);
        proc.communicate_utf8_async(null, null, (proc, res) => {
            try {
                let [ok, stdout, stderr] = proc.communicate_utf8_finish(res);
                if (stderr && stderr.trim()) {
                    logError(`depgraph: pacman -Qi error: ${stderr}`);
                    // still start the update loop; impact counts will just
                    // miss the depgraph until the next successful rebuild
                    this.restartLoop();
                    return;
                }
                let graph = this._parsePacmanQi(stdout);
                this._writeDepgraph(graph);
            } catch (e) {
                logError(`depgraph update failed: ${e}`);
                this.restartLoop();
            }
        });
    }

    _parsePacmanQi(output) {
        let packages = {};
        for (let block of output.split('\n\n')) {
            let pkg = {};
            for (let line of block.split('\n')) {
                let match = line.match(/^(\S[^:]*?)\s*:\s*(.*)/);
                if (!match) continue;
                let [, key, val] = match;
                if (key.trim() === 'Name') {
                    pkg.name = val.trim();
                } else if (key.trim() === 'Version') {
                    pkg.version = val.trim();
                } else if (key.trim() === 'Install Reason') {
                    pkg.reason = val.includes('Explicitly') ? 'explicit' : 'dependency';
                } else if (key.trim() === 'Depends On') {
                    let v = val.trim();
                    pkg.depends_on = v === 'None' ? [] :
                        v.split(/\s+/).map(d => d.replace(/[><=].*/,''));
                }
            }
            if (pkg.name) {
                pkg.required_by = [];
                packages[pkg.name] = pkg;
            }
        }
        for (let [name, pkg] of Object.entries(packages)) {
            for (let dep of pkg.depends_on) {
                if (packages[dep]) packages[dep].required_by.push(name);
            }
        }
        return { last_updated: Math.floor(Date.now() / 1000), packages };
    }

    // Follow required_by edges upward from each package in pkgNames,
    // collecting every reachable package (including the starting packages).
    // Returns the full set of all transitively impacted installed packages.
    _allImpactedOf(pkgNames) {
        let visited = new Set();

        const visit = (name) => {
            if (visited.has(name)) return;
            visited.add(name);
            let pkg = this._depgraph && this._depgraph.packages[name];
            if (!pkg) return;
            for (let parent of pkg.required_by) visit(parent);
        };

        for (let name of pkgNames) visit(name);
        return visited;
    }

    _writeDepgraph(graph) {
        this._depgraph = graph;
        let file = Gio.File.new_for_path(this._depgraphPath);
        let bytes = new TextEncoder().encode(JSON.stringify(graph, null, 2));
        file.replace_contents(
            bytes, null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null
        );
        log(`depgraph updated: ${Object.keys(graph.packages).length} packages`);
        this.restartLoop();
    }
}

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                                   MAIN                                     //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

function main(metadata, orientation, panel_height, instance_id) {
    return new PacmanUpdater(metadata, orientation, panel_height, instance_id);
}

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                                 LOGGING                                    //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

function log(msg) {
    global.log(`[pacman-updater@smanilov] ${msg}`);
}

function logError(msg) {
    global.logError(`[pacman-updater@smanilov] ${msg}`);
}
