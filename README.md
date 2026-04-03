  <p align="center">
    <img src="marmoset_logo_red.png" alt="mviewer icon" width="128">
  </p>

# mviewer

Rust-native exporter for Marmoset `.mview` scenes.

![vivfox export preview](docs/images/sample.gif)

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

## Feature Parity

### Marmoset `.mview` / JS Parity

- [x] `.mview` archive parsing
- [x] `scene.json` parsing
- [x] static glTF scene export
- [x] animated glTF scene export
- [x] skinning export
- [x] camera export
- [x] light export
- [x] primary UV export
- [x] corrected UV orientation parity
- [x] packed normal decoding
- [x] vertex color export
- [x] material texture extraction
- [x] alpha texture merge
- [x] metallic-roughness texture packing
- [x] embedded Marmoset runtime preview in the desktop GUI
- [x] direct OBJ export in the desktop GUI
- [ ] full plain-glTF parity for all Marmoset-only runtime behavior
- [x] `.glb` output
- [ ] direct `FBX` export

### Project Features

- [x] Rust-native command-line tool
- [x] Native desktop GUI with embedded Marmoset preview
- [x] Windows, Linux, and macOS builds
- [x] GitHub Actions CI and release packaging
- [x] GitHub Pages project site
- [x] sample animated fixtures in-repo

## Quick Start

```powershell
mviewer input.mview output_dir
```

Example:

```powershell
mviewer test_data\vivfox.mview test_output\vivfox
```

This writes for glTF:

- `<name>.gltf`
- `<name>.bin`
- copied texture files used by the scene
- merged `*_rgba.png` textures when the source scene uses a separate alpha map

For GLB:

```powershell
mviewer test_data\vivfox.mview test_output\vivfox_glb --format glb
```

This writes:

- `<name>.glb`

## GUI Workflow

Launch the desktop app with:

```text
mviewer
```

The current GUI supports:

- opening a local `.mview` file
- loading a supported URL directly, including ArtStation artwork pages that expose a public `.mview`
- previewing the scene through the embedded Marmoset runtime
- exporting to glTF, GLB, or OBJ/MTL
- choosing export options per format
- remembering the last export defaults and recent files

## Output Formats

mviewer exports `.mview` scenes to glTF and GLB directly.

The desktop GUI can also export selected meshes to OBJ/MTL with source textures.

If you need other formats:

- `mview to fbx`: export to glTF first, then convert with Blender or another downstream tool
- `mview to obj`: export to glTF first, then convert if you only need static geometry/materials
- `mview to blender`: import the generated glTF into Blender

glTF is the primary interchange format because it preserves modern materials, skins, animation, cameras, and lights better than the old script-based workflow.

## Build From Source

Requirements:

- Rust stable toolchain

Build:

```powershell
cargo build --release
```

Run GUI:

```powershell
cargo run
```

Run CLI:

```powershell
cargo run --release -- <input.mview> [output_dir]
```

## Workflow Notes

mviewer is a Rust-native Marmoset `.mview` viewer, exporter, and converter focused on `.mview` to glTF export.

If you are looking for an `mview viewer`, `mview editor`, `mview to gltf`, `mview to fbx`, or `mview to obj` workflow, the current project path is:

`.mview` -> `glTF` -> optional conversion to `FBX`, `OBJ`, Blender, or other formats

This repository no longer uses the old Python extractors or the Noesis plugin as its primary workflow. The current implementation reads `.mview` archives directly, exports with a Rust backend, and uses the embedded Marmoset runtime in the desktop GUI for preview parity.

## Current Limitations

- stock third-party glTF viewers only see the standard glTF export
- direct `FBX` export is not implemented yet

## Reverse Engineering References

These files are still kept in the repo as format references:

- `docs/reverse-engineering/marmoset-d3f745560e47d383adc4f6a322092030.js`

The newer bundled Marmoset JavaScript is the primary reference for format and runtime parity work.
