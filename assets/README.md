# Brand assets

The Ganglion mark is a neural-relay hub: a central ganglion body with signals
radiating to satellite nodes (one hollow node = an endpoint not yet connected;
one dashed link = a relayed circuit). Accent is **teal `#0D9488`**.

| File | Use |
|------|-----|
| `logo.svg` | Color mark (teal). Square icon — avatar, README header icon. |
| `logo-mono.svg` | Monochrome mark; inherits `currentColor`, so it adapts to light/dark. |
| `logo-lockup.svg` | Mark + `ganglion` wordmark, horizontal. Used in the README header. |
| `logo-lockup-mono.svg` | Monochrome lockup (`currentColor`). |
| `logo-512.png` | 512×512 raster of the color mark (avatar upload, slides). |
| `social-preview.svg` / `.png` | 1280×640 GitHub social preview card. |

The wordmark is **outlined to vector paths** (Space Grotesk Bold), so the SVGs
render identically everywhere without a webfont — GitHub strips SVG fonts.

## Setting the GitHub social preview

GitHub has no API for this — upload it by hand once:
**repo → Settings → General → Social preview → Edit → upload
`assets/social-preview.png`**.

## Regenerating

`social-preview.png` / `logo-512.png` are rendered from the SVGs with
`cairosvg`. To rebuild after editing an SVG:

```bash
cairosvg assets/social-preview.svg -o assets/social-preview.png -W 1280 -H 640
cairosvg assets/logo.svg           -o assets/logo-512.png       -W 512  -H 512
```
