# NinjaCrawler brand assets

The SVG files in this directory are the editable brand masters.

- `ninjacrawler-symbol.svg`: primary transparent symbol for flexible use.
- `ninjacrawler-symbol-micro.svg`: optically simplified symbol for 16-32 px use.
- `ninjacrawler-symbol-mono-*.svg`: one-color applications.
- `ninjacrawler-lockup-horizontal*.svg`: primary light and dark horizontal signatures.
- `ninjacrawler-lockup-compact.svg`: stacked signature for square placements.
- `ninjacrawler-app-icon.svg`: charcoal application tile used to generate executable icons.

Keep clear space around the mark equal to the stroke width. Do not add gradients, shadows, outlines, or alter the symbol proportions. Generate Tauri assets with:

```powershell
npx tauri icon assets/brand/ninjacrawler-app-icon.svg --output src-tauri/icons
```
