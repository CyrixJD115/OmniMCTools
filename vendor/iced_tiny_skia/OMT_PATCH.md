# OMT patch to iced_tiny_skia 0.14.1 (MIT)

`Engine::draw_image` (raster branch) ignored the image's own
`clip_bounds` — e.g. the bounds of the canvas frame an image was drawn
into — and clipped only to the enclosing layer, which for a canvas is
usually the whole window. Zoomed-in map tiles (much larger than the map
canvas) were therefore painted over the top bar, side panels and status
bar under the software renderer.

Patch (marked `OMT_PATCH` in `src/engine.rs`): intersect the image's
`clip_bounds` with the layer clip, build the mask for that rectangle when
the image is not fully inside it, and restore the layer mask afterwards.
The SVG branch is untouched (OMT draws no SVG). Drop this vendored copy
once upstream fixes it.

## 2. Full-frame repaint (`src/window/compositor.rs`)

The software compositor repaints only the regions its layer diff reports
as damaged (per buffer age). In OMT this intermittently left stale
content: the top bar's "Open world…" button was drawn without its label
and icon in 2 of 6 identical headless runs (Map Viewer, zoom 8), and in
0 of 6 with damage tracking disabled. Patch: `present` always repaints the
whole frame (`last_layers = None`). Cost: one full software repaint per
frame, only when the GPU renderer is unavailable.
