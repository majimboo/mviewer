# mviewer

Rust-native exporter for Marmoset `.mview` scenes.

This repository no longer uses the old Python extractors or the Noesis plugin as its primary workflow. The current implementation reads the `.mview` archive directly, exports a glTF scene, and emits a runtime playback page for preserved Marmoset state.

![vivfox export preview](docs/images/vivfox-exporter-screenshot.png)

## Download

Prebuilt binaries are published from GitHub Actions releases:

- Windows x64
- Linux x64
- macOS arm64
- macOS x64

Latest release: [`latest`](https://github.com/majimboo/mviewer/releases/latest)

Download the archive that matches your platform:

- Windows 64-bit: `mviewer-vX.Y.Z-windows-x64.zip`
- Linux 64-bit: `mviewer-vX.Y.Z-linux-x64.tar.gz`
- macOS Apple Silicon: `mviewer-vX.Y.Z-macos-arm64.tar.gz`
- macOS Intel: `mviewer-vX.Y.Z-macos-x64.tar.gz`

After extracting a release archive, run:

```text
mviewer --help
```

## What It Does

Current exporter support:

- `.mview` archive parsing
- `scene.json` parsing
- static and animated export to `.gltf` + external `.bin`
- mesh transforms, skins, and animation export
- primary UVs
- packed normal decoding
- vertex colors when present
- material export with base color, normal, alpha merge, and metallic-roughness packing
- camera and light export with runtime bindings
- raw archive preservation under `mviewer_raw/`
- runtime sidecar export to `mviewer.runtime.json`
- generated `viewer.html` runtime player

Current limitations:

- stock third-party glTF viewers only see the standard glTF export
- full behavior parity depends on the generated runtime player plus `mviewer.runtime.json`, not plain glTF semantics alone
- `.glb` output

## Quick Start

```powershell
mviewer input.mview output_dir
```

Example:

```powershell
mviewer test_data\vivfox.mview test_output\vivfox
```

This writes:

- `<name>.gltf`
- `<name>.bin`
- `viewer.html`
- `mviewer.runtime.json`
- copied texture files used by the scene
- merged `*_rgba.png` textures when the source scene uses a separate alpha map
- `mviewer_raw/` with all source archive entries

## Build From Source

Requirements:

- Rust stable toolchain

Build:

```powershell
cargo build --release
```

Run:

```powershell
cargo run --release -- <input.mview> [output_dir]
```

## Runtime Playback

After export, open `viewer.html` from the output directory in a browser. It loads the generated glTF, reads `mviewer.runtime.json`, and applies preserved runtime state such as:

- evaluated object transforms
- inherited visibility
- sampled light and camera properties
- sampled material UV and emissive properties
- preserved fog / sky / shadow-floor scene data

## Reverse Engineering References

These files are still kept in the repo as format references:

- `docs/reverse-engineering/marmoset-d3f745560e47d383adc4f6a322092030.js`

The newer bundled Marmoset JavaScript is the primary reference for format and runtime parity work.
