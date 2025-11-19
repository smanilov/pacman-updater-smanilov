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


