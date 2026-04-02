# Reverse Engineering Notes

This folder holds reference material for the `.mview` format.

Current source of truth:

- `marmoset-d3f745560e47d383adc4f6a322092030.js`

Use this file to trace:

- archive parsing and decompression
- mesh buffer layout
- material field meanings
- camera and light scene semantics
- animation, skinning, and matrix-table behavior

The Rust exporter should be updated against this bundled Marmoset script rather than older handwritten notes.
