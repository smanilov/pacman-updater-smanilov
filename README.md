# Installation

1. git clone this-repo ~/.local/share/cinnamon/applets/pacman-updater@smanilov
2. Alt+F2, r to restart Cinnamon
3. add pacman-updater@smanilov to Applets

# Usage

1. this applet will run `checkupdates` (you need to install it separately)
   repeatedly and count the number of returned lines
2. if the count is positive, a notification will be shown informing the user
   there are updates available
3. from the taskbar, the user can click the smiley and run the updater; this
   will open a new terminal and prompt for sudo password
4. after the update completes successfully, `depgraph.json` is refreshed
   automatically; it is also refreshed on applet startup

# Data

depgraph.json is written by the applet and contains the dependency graph between
pacman managed packages. It is updated on startup and after each successful
`sudo pacman -Syu`. An example follows:

```
{
  "last_updated": 1774870313,
  "packages": {
    "firefox": {
      "name": "firefox",
      "reason": "explicit",
      "version": "123.0-1",
      "depends_on": ["nss", "gtk3"]
    },
    "nss": {
      "name": "nss",
      "reason": "dependency",
      "version": "3.98-1",
      "depends_on": ["nspr"]
    }
  }
}
```
