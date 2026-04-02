# mviewer

Rust-native exporter for Marmoset `.mview` scenes.

This repository no longer uses the old Python extractors or the Noesis plugin as its primary workflow. The current implementation reads the `.mview` archive directly, exports a glTF scene, and emits a runtime playback page for preserved Marmoset state.

## Status

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
- source scene preservation in glTF `extras`
- `MVIEWER_marmoset_runtime` extension output with sampled runtime state
- generated `viewer.html` runtime player

Remaining limitations:

- stock third-party glTF viewers will ignore `MVIEWER_marmoset_runtime`
- full behavior parity depends on the generated runtime player, not plain glTF semantics alone
- `.glb` output

## Build

```powershell
cargo build --release
```

## Usage

```powershell
cargo run -- <input.mview> [output_dir]
```

Example:

```powershell
cargo run -- test_data\vivfox.mview test_output\vivfox
```

This writes:

- `<name>.gltf`
- `<name>.bin`
- `viewer.html`
- copied texture files used by the scene
- merged `*_rgba.png` textures when the source scene uses a separate alpha map
- `mviewer_raw/` with all source archive entries

## Runtime Playback

After export, open `viewer.html` from the output directory in a browser. It loads the generated glTF, consumes `MVIEWER_marmoset_runtime`, and applies preserved runtime state such as:

- evaluated object transforms
- inherited visibility
- sampled light and camera properties
- sampled material UV and emissive properties
- preserved fog / sky / shadow-floor scene data

## Reverse Engineering References

These files are still kept in the repo as format references:

- `docs/reverse-engineering/marmoset-d3f745560e47d383adc4f6a322092030.js`

The newer bundled Marmoset JavaScript is the primary reference for format and runtime parity work.
