# Changelog

## 2.1.2 - 2026-04-03

- added visible progress/loading feedback for URL-based scene loading in the desktop app
- switched release publishing to GitHub CLI to avoid deprecated Node 20 release actions

## 2.1.1 - 2026-04-03

- refreshed release docs/media assets
- added the animated preview GIF to the published docs assets

## 2.1.0 - 2026-04-03

- unified the project into a single `mviewer` binary: no arguments launch the GUI, arguments run the CLI
- rebuilt the desktop app around an embedded Marmoset preview using `tao + wry`
- added direct URL loading in the GUI, including ArtStation artwork page resolution
- added GUI export options for glTF, GLB, and OBJ/MTL
- added CLI `--format gltf|glb`
- added persistent app settings, recent files/URLs, and appdata-backed WebView storage
- updated icons, packaging, CI, and release assets for the single-binary desktop app

## 2.0.0 - 2026-04-02

- replaced the legacy Python and Noesis workflow with a Rust-native `.mview` exporter
- added glTF export for static and animated scenes
- added runtime sidecar export with generated `viewer.html`
- added sample animated fixtures and updated reverse-engineering references
- added CI, tagged release packaging, and GitHub Pages site
