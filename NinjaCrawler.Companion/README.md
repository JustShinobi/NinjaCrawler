# NinjaCrawler Companion

Chrome extension package for detecting supported profiles across all open tabs, adding selected profiles in one batch, syncing the active profile, and importing the signed-in browser account into the local NinjaCrawler desktop app.

The extension deduplicates supported profile tabs across every Chrome window, shows which profiles already exist, and lets the operator select new profiles to add in one batch. Actions tied to a specific URL—sync, story/video download, and account import—continue to use the active tab. On Instagram, X/Twitter, and TikTok, **Import account** captures the current browser session only after an explicit click. The operator then chooses whether to update an existing provider account or create a new one.

Captured cookies and provider authorization metadata are sent only to the loopback desktop API and stored in NinjaCrawler's protected session store. Updating an account preserves its paths, defaults, capabilities, and source bindings. NinjaCrawler keeps one previous Companion import that can be restored from the Accounts window.

The popup supports system, light, and dark themes. Open **Appearance & shortcuts**
to choose a theme, inspect the currently assigned commands, or open Chrome's
native shortcut editor. The available commands are **Sync active profile** for
Instagram, TikTok, and X/Twitter, and **Download active story** when the active
Instagram or TikTok URL identifies a supported story. Command results are shown
on the extension badge, so the popup does not need to remain open.

NinjaCrawler reports the Companion version bundled with its current release and
the minimum compatible version. Download or update it from **Connector
Runtimes** in NinjaCrawler. The desktop app owns the download and keeps the
stable `%LocalAppData%\NinjaCrawler\Companion` folder current.

Chrome requires one manual setup for extensions distributed outside the Chrome
Web Store: enable Developer mode and use **Load unpacked** with that managed
folder. The Companion checks that managed folder through NinjaCrawler and
notifies you when a newer staged version appears. Automatic reload is disabled
by default and can be enabled under **Appearance, updates & shortcuts**. If Chrome is
still running a copy from another folder after an update, the popup detects the
version mismatch, offers **Copy managed folder**, and opens
`chrome://extensions`.

The extension badge uses `↑` for an available update and `!` when an update is
required for compatibility.

## Local Development

1. Build and run NinjaCrawler.
2. Open `chrome://extensions`.
3. Enable Developer mode.
4. Select **Load unpacked** and choose this `NinjaCrawler.Companion` folder
   (or `%LocalAppData%\NinjaCrawler\Companion` after downloading it from
   Connector Runtimes).

## Updating an unpacked installation

Version 0.3.0 introduces a stable extension ID. The one-time installation must
point Chrome at `%LocalAppData%\NinjaCrawler\Companion`; download that folder
from NinjaCrawler's **Connector Runtimes** window. Later downloads replace the
managed files and the extension can reload automatically when that preference
is enabled. The popup guides the
one-time path correction if Chrome is still using another folder.

The extension calls the desktop API at:

```text
http://127.0.0.1:47219/ninjacrawler-companion/v1
```

## Supported Profile URLs

- Instagram: `https://www.instagram.com/<handle>/`
- X / Twitter: `https://x.com/<handle>` or `https://twitter.com/<handle>`
- TikTok: `https://www.tiktok.com/@<handle>`

The extension badge shows:

- `✓` when the current profile already exists in NinjaCrawler.
- `+` when the current profile is supported and can be added.
- `!` when the desktop API is unavailable.
