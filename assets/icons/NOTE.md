# icons — raster derivatives

`selora-ide-mono.svg` is the hand-authored source mark. The raster
derivatives are generated from it, never edited directly.

Generated files (committed; regenerate with the script after any SVG change):

- `skarn.ico` — Windows app icon (16/32/48 BMP + 256 PNG entries), used
  by the installer (`installer/skarn.iss`) and available for the window
  exe.
- `skarn-256.png` — 256px raster.
- `skarn-512.png` — 512px raster.

## How they are generated

`generate_icons.py` (this directory) replicates the SVG's construction with
Pillow on a 4x-supersampled canvas — no SVG rasterizer needed:

```sh
python assets/icons/generate_icons.py
```

The IDE chrome itself does **not** import these rasters: the title-bar /
splash / about instances of the mark are painted in code
(`crates/skarn-ui/src/brand.rs`) per DESIGN_TOKENS.md §9.
