# NinjaCrawler Companion

Chrome extension package for adding and syncing supported profile tabs with the local NinjaCrawler desktop app.

The V1 extension intentionally stays small: it detects the active profile tab, shows whether the profile already exists, sends new `provider + handle` seeds to NinjaCrawler, and queues sync for existing profiles. Provider-specific configuration remains in NinjaCrawler.

## Local Development

1. Build and run NinjaCrawler.
2. Open `chrome://extensions`.
3. Enable Developer mode.
4. Select **Load unpacked** and choose this `NinjaCrawler.Companion` folder.

The extension calls the desktop API at:

```text
http://127.0.0.1:47219/ninjacrawler-companion/v1
```

## Supported Profile URLs

- Instagram: `https://www.instagram.com/<handle>/`
- X / Twitter: `https://x.com/<handle>` or `https://twitter.com/<handle>`
- TikTok: `https://www.tiktok.com/@<handle>`
- Reddit: `https://www.reddit.com/user/<handle>` or `/u/<handle>`

The extension badge shows:

- `✓` when the current profile already exists in NinjaCrawler.
- `+` when the current profile is supported and can be added.
- `!` when the desktop API is unavailable.
