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

        log("pacman updater started -- version 1.0.0");

        /** @type {number|null} ID of the Mainloop timeout */
        this._timeout = null;
        /** @type {PopupMenu.PopupMenuManager|null} */
        this.menuManager = null;
        /** @type {PopupMenu.PopupMenu|null} */
        this.menu = null;
        /** @type {PopupMenu.PopupSwitchMenuItem|null} */
        this.toggleLoopItem = null;

        // bool
        this._loopEnabled = true;
        // int
        this._updateCount = 0;
        // bool
        this._checkingForUpdates = false;
        // bool
        this._hasNetwork = false;

        /** @type {number|null} */
        this._networkWatcherId = null;
        /** @type {string} */
        this._depgraphPath = metadata.path + '/depgraph.json';

        this.set_applet_icon_name("face-smile");

        this.buildMenu(orientation);

        this.updateDepgraph();

        this.startLoop();
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
//                                SETTERS                                     //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    setLoopEnabled(loopEnabled) {
        this._loopEnabled = loopEnabled;
        this._updateTooltip();
    }

    setUpdateCount(updateCount) {
        this._updateCount = updateCount;
        this._updateTooltip();
    }

    setCheckingForUpdates(checkingForUpdates) {
        this._checkingForUpdates = checkingForUpdates;
        this._updateTooltip();
    }

    setHasNetwork(hasNetwork) {
        this._hasNetwork = hasNetwork;
        this._updateTooltip();
    }

    _updateTooltip() {
        let loopState = this._loopEnabled ? "running" : "not running";
        if (!this._hasNetwork) {
            this.set_applet_tooltip(`loop is ${loopState}\nno network connection`);
        } else if (this._checkingForUpdates) {
            this.set_applet_tooltip(`loop is ${loopState}\nchecking for updates...`);
        } else {
            let countMessage = this._updateCount == 0 ?
                "no updates available" :
                `updates available: ${this._updateCount}`;
            this.set_applet_tooltip(`loop is ${loopState}\n${countMessage}`);
        }
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

        this.menu.addAction(_("Run pacman update..."), () => {
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
        return this._timeout;
    }

    startLoop() {
        if (this.isLoopRunning()) {
            log("warning: corrupt state; already running");
            return;
        }
        this.checkUpdates();
        this._timeout = Mainloop.timeout_add_seconds(10 * 60, () => {
            this.checkUpdates();
            return true;
        });

        log("loop started");
    }

    stopLoop() {
        if (!this.isLoopRunning()) {
            log("warning: corrupt state; not running");
            return;
        }

        Mainloop.source_remove(this._timeout);
        this._timeout = null;

        log("loop stopped");
    }

////////////////////////////////////////////////////////////////////////////////
//                                                                            //
//                               RUN UPDATE                                   //
//                                                                            //
////////////////////////////////////////////////////////////////////////////////

    launchUpdateTerminal() {
        log("opening terminal...");

        // --wait makes gnome-terminal block until the shell exits, so the
        // spawnCommandLineAsync callback fires only after the update finishes.
        Util.spawnCommandLineAsync(
            'gnome-terminal --wait -- bash -c "sudo pacman -Syu && fc-cache -fv"',
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

    checkUpdates() {
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

        log("checking for updates...");
        this.setCheckingForUpdates(true);

        let proc = new Gio.Subprocess({
            argv: ['bash', '-c', 'checkupdates'],
            flags: Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE
        });

        proc.init(null);

        proc.communicate_utf8_async(null, null, (proc, res) => {
            try {
                let [ok, stdout, stderr] = proc.communicate_utf8_finish(res);
                // ok is ignored, because checkupdates returns "not ok" if there
                // are no updates; stdout is used to check if there are updates
                log("check complete");
                this.setCheckingForUpdates(false);
                if (stderr) {
                    logError(`error: ${stderr}`);
                } else {
                    this.setUpdateMessage(stdout.trim());
                }
            } catch (e) {
                logError(`exception: parseInt failed? ${e}`);
            }
        });
    }

    setUpdateMessage(cmdOutput) {
        let count = cmdOutput ? cmdOutput.split('\n').length : 0;
        log(`updates available: ${count}`);
        this.setUpdateCount(count);
        if (count > 0) {
            let notification = `updates available: ${count}`;
            if (count <= 10) {
                notification += `\n${cmdOutput}`;
            }
            Main.notify("Pacman Updater", notification);
        }
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
                    if (this.isLoopRunning()) {
                        log(`restarting loop...`);
                        this.stopLoop();
                    }
                    this.startLoop();
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
                    return;
                }
                let graph = this._parsePacmanQi(stdout);
                this._writeDepgraph(graph);
            } catch (e) {
                logError(`depgraph update failed: ${e}`);
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
            if (pkg.name) packages[pkg.name] = pkg;
        }
        return { last_updated: Math.floor(Date.now() / 1000), packages };
    }

    _writeDepgraph(graph) {
        let file = Gio.File.new_for_path(this._depgraphPath);
        let bytes = new TextEncoder().encode(JSON.stringify(graph, null, 2));
        file.replace_contents(
            bytes, null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null
        );
        log(`depgraph updated: ${Object.keys(graph.packages).length} packages`);
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
