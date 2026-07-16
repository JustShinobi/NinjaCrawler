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
the minimum compatible version. When the installed extension is older, the
popup shows the installed and available versions. With NinjaCrawler running you
can:

1. **Download to AppData** — the desktop app stages the Companion ZIP under
   `%LocalAppData%\NinjaCrawler\Companion`.
2. **Reload extension** — the popup calls `chrome.runtime.reload()` so Chrome
   picks up the staged files without opening `chrome://extensions`.

`chrome.runtime.reload()` only applies files from the folder Chrome already
loaded. For one-click updates, load unpacked from the AppData path above (or
point Load unpacked there once). If you keep a different folder loaded, use
**Open extensions** and reload that install after copying files, or switch Load
unpacked to the AppData path.

The extension badge uses `↑` for an available update and `!` when an update is
required for compatibility.

## Local Development

1. Build and run NinjaCrawler.
2. Open `chrome://extensions`.
3. Enable Developer mode.
4. Select **Load unpacked** and choose this `NinjaCrawler.Companion` folder
   (or `%LocalAppData%\NinjaCrawler\Companion` after staging an update).

## Updating an unpacked installation

Version 0.3.0 introduces a stable extension ID. Prefer the in-popup
**Download to AppData** + **Reload extension** flow when NinjaCrawler is
running. Manual fallback: extract the release ZIP over the loaded Companion
folder and click **Reload** on `chrome://extensions`.

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
