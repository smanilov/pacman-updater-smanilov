const Applet = imports.ui.applet;
const Gio = imports.gi.Gio;
const Main = imports.ui.main;
const Mainloop = imports.mainloop;
const PopupMenu = imports.ui.popupMenu;
const St = imports.gi.St;
const Util = imports.misc.util;

class PacmanUpdater extends Applet.IconApplet {
    constructor(metadata, orientation, panel_height, instance_id) {
        super(orientation, panel_height, instance_id);

        log("pacman updater started");

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

        this.set_applet_icon_name("face-smile");

        this.buildMenu(orientation);

        this.startLoop();
    }

    buildMenu(orientation) {
        this.menuManager = new PopupMenu.PopupMenuManager(this);
        this.menu = new Applet.AppletPopupMenu(this, orientation);
        this.menuManager.addMenu(this.menu);

        this.toggleLoopItem = new PopupMenu.PopupSwitchMenuItem(
            "Enable update loop",
            this._loopEnabled
        );

        this.toggleLoopItem.connect('toggled', (item, state) => {
            this._loopEnabled = state;
            this.updateTooltip();
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

    on_applet_clicked(event) {
        log("applet clicked");
        this.menu.toggle();
    }

    startLoop() {
        if (this._timeout) {
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
        if (!this._timeout) {
            log("warning: corrupt state; not running");
            return;
        }
        if (this._timeout) {
            Mainloop.source_remove(this._timeout);
            this._timeout = null;
        }

        log("loop stopped");
    }

    checkUpdates() {
        log("checking for updates...");
        this._checkingForUpdates = true;
        this.updateTooltip();

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
                this._checkingForUpdates = false;
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

    launchUpdateTerminal() {
        log("opening terminal...");

        Util.spawnCommandLine('gnome-terminal -- bash -c "sudo pacman -Syu"');

        log("terminal opened");
    }

    setUpdateMessage(cmdOutput) {
        let count = cmdOutput ? cmdOutput.split('\n').length : 0;
        log(`updates available: ${count}`);
        this._updateCount = count;
        if (count > 0) {
            let notification = `updates available: ${count}`;
            if (count <= 10) {
                notification += `\n${cmdOutput}`;
            }
            Main.notify("Pacman Updater", notification);
        }
        this.updateTooltip();
    }

    updateTooltip() {
        let loopState = this._loopEnabled ? "running" : "not running";
        let countMessage = this._checkingForUpdates ?
            "checking for updates..." :
                this._updateCount == 0 ?
                "no updates available" :
                `updates available: ${this._updateCount}`;
        this.set_applet_tooltip(`loop is ${loopState}\n${countMessage}`);
    }

    on_applet_removed_from_panel() {
        if (this._timeout) {
            this.stopLoop();
        }
        this.menu.destroy();
        log("pacman updater stopped");
    }
}

function main(metadata, orientation, panel_height, instance_id) {
    return new PacmanUpdater(metadata, orientation, panel_height, instance_id);
}

function log(msg) {
    global.log(`[pacman-updater@smanilov] ${msg}`);
}

function logError(msg) {
    global.logError(`[pacman-updater@smanilov] ${msg}`);
}

