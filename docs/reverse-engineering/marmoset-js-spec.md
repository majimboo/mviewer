# Marmoset JS Runtime Spec

This document records the current reverse-engineering findings from:

- [marmoset-d3f745560e47d383adc4f6a322092030.js](/D:/projects/mviewer/docs/reverse-engineering/marmoset-d3f745560e47d383adc4f6a322092030.js)

It is intended to be the implementation reference for the Rust runtime and viewer so the project does not need to rediscover the format repeatedly.

## Scope

This spec covers:

- archive and scene composition
- mesh and material loading
- animated object transform evaluation
- skinning and mesh deformation
- visibility, materials, lights, fog, sky, and cameras

This spec does not yet enumerate every shader uniform bit-by-bit, but it documents the runtime behavior and the important feature flags that the Rust viewer must match.

## Scene Construction

Scene creation happens through `Scene(...)` and `SceneAnimator(...)`.

Important runtime objects:

- `Scene`
- `Material`
- `Mesh`
- `MeshRenderable`
- `SceneAnimator`
- `SkinningRig`
- `SkinningCluster`
- `Sky`
- `Fog`
- `ShadowFloor`

Source references:

- `Scene` loading around JS lines `248-255`
- `SceneAnimator` setup around JS lines `255-264`

## Archive Inputs

Important archive payloads:

- `scene.json`
- `MatTable.bin`
- `SkinRig*.dat`
- mesh payloads such as `mesh*.dat`
- optional `sky.dat` or `sky.png`
- texture assets referenced by materials
- keyframe blob files referenced by animated objects

## Mesh Format

Mesh renderables are created from:

- a `Mesh`
- one or more `subMeshes`
- one `Material` per submesh

Source references:

- `Mesh` constructor around JS lines `200-203`
- `MeshRenderable` around JS lines `203-212`

Vertex layout behavior visible from JS:

- position: `3 x f32`
- primary UV: `2 x f32`
- optional secondary UV: `2 x f32`
- tangent: packed unit vector
- bitangent: packed unit vector
- normal: packed unit vector
- optional vertex color

Exact stride behavior from `Mesh(...)`:

- base stride: `32` bytes
- `+8` bytes if `secondaryTexCoord`
- `+4` bytes if `vertexColor`

Exact attribute order used by `MeshRenderable.draw(...)`:

1. `vPosition`: `3 x float` at offset `0`
2. `vTexCoord`: `2 x float` at offset `12`
3. optional `vTexCoord2`: `2 x float`
4. `vTangent`: `2 x normalized ushort`
5. `vBitangent`: `2 x normalized ushort`
6. `vNormal`: `2 x normalized ushort`
7. optional `vColor`: `4 x normalized ubyte`

Important mesh runtime fields:

- `displayMatrix`
- `modelMatrix`
- `origin`
- `dynamicVertexData` for dynamic/skinned meshes
- `indexBuffer`
- `wireBuffer`

Important note:

- JS rendering uses packed tangent-space vectors directly from the vertex buffer
- exact viewer parity should either preserve the packed format or decode it identically and keep tangent/bitangent/normal aligned

Important renderer expectations:

- `vPosition`
- `vTexCoord`
- `vTangent`
- `vBitangent`
- `vNormal`
- optional `vColor`
- optional `vTexCoord2`

Rust implication:

- decoded meshes must preserve tangent and bitangent data, not only normals
- exact material parity requires the same tangent-space basis the JS uses
- dynamic/skinned meshes must preserve a writable vertex payload equivalent to `dynamicVertexData` if the runtime is ported literally

## Material System

Material construction is centered on `Material(...)`.

Source references:

- material setup around JS lines `171-189`

Textures loaded by the JS runtime:

- `albedo = fromFilesMergeAlpha(albedoTex, alphaTex)`
- `reflectivity = fromFilesMergeAlpha(reflectivityTex, glossTex)`
- `normal = fromFile(normalTex)`
- `extras = fromFilesMergeAlpha(extrasTex, extrasTexA)`

Important detail:

- Marmoset merges paired textures at load time rather than treating them as separate runtime samplers in many cases.
- `TextureCache.fromFilesMergeAlpha(...)` merges RGB from the first texture with alpha from the second texture.
- when merge inputs differ in size, the runtime renders/merges them through an intermediate framebuffer path when possible.

Material runtime properties include:

- `uOffset`
- `vOffset`
- `emissiveIntensity`
- `alphaTest`
- `fresnel`
- `horizonOcclude`
- `horizonSmoothing`

Feature flags seen in the JS shader setup:

- `SKIN`
- `SKIN_VERSION_2`
- `ANISO`
- `MICROFIBER`
- `REFRACTION`
- `VERTEX_COLOR`
- `VERTEX_COLOR_SRGB`
- `VERTEX_COLOR_ALPHA`
- `HORIZON_SMOOTHING`
- `DIFFUSE_UNLIT`
- `EMISSIVE`
- `EMISSIVE_SECONDARY_UV`
- `AMBIENT_OCCLUSION`
- `AMBIENT_OCCLUSION_SECONDARY_UV`
- `TSPACE_ORTHOGONALIZE`
- `TSPACE_RENORMALIZE`
- `TSPACE_COMPUTE_BITANGENT`
- `TEXCOORD_SECONDARY`

Extras texture coordinate ranges:

- stored in `extrasTexCoordRanges`
- each entry uses a `scaleBias` transform
- used for subdermis, translucency, fuzz, aniso, refraction mask, emissive, and AO regions

Confirmed extras feature mapping:

- `subdermisTex`
  - used by `SKIN`
  - sampled from `extras`
  - RGB controls subdermis color
  - alpha contributes skin blend/intensity
- `translucencyTex`
  - used by `SKIN`
  - sampled from `extras`
  - RGB multiplies transmission color
- `fuzzTex`
  - used by `SKIN` and `MICROFIBER`
  - sampled from `extras`
  - RGB modulates fuzz/fresnel contribution
- `anisoTex`
  - used by `ANISO`
  - sampled from `extras`
  - RGB is remapped from `[0, 1]` to `[-1, 1]` and interpreted as anisotropy direction input
- `refractionMaskTex`
  - used by `REFRACTION`
  - sampled from `extras`
  - X channel is the refraction mix mask
- `emissiveTex`
  - used by `EMISSIVE`
  - sampled from `extras`
  - RGB is decoded with `dG(...)` and scaled by `uEmissiveScale`
- `aoTex`
  - used by `AMBIENT_OCCLUSION`
  - sampled from `extras`
  - X channel is AO intensity, then squared

Secondary UV usage:

- `emissiveSecondaryUV` enables `EMISSIVE_SECONDARY_UV`
- `aoSecondaryUV` enables `AMBIENT_OCCLUSION_SECONDARY_UV`
- these features are the reason `TEXCOORD_SECONDARY` can become required by the material

Rust implication:

- exact parity is not just "load albedo/normal"
- the viewer eventually needs a material system that understands merged texture pairs, UV offsets, extras regions, and material feature flags

### Material Shader Semantics

The core material fragment path is composed from:

- `matsampling.glsl`
- `matlighting.glsl`
- `matshadows.glsl`
- `matskin.glsl`
- `matmicrofiber.glsl`
- `matstrips.glsl`
- optional `matdither.glsl`

Source references:

- shader bundle around JS lines `455-461`

Confirmed semantics from shader code:

- `dG(rgb)` converts texture RGB with a simple square: `rgb * rgb`
- albedo is sampled from `tAlbedo`, then multiplied by optional vertex color
- vertex color optionally applies its own SRGB decode before multiplication
- alpha comes from `tAlbedo.a`, then optionally multiplied by vertex alpha
- alpha test happens before any heavy lighting work
- normal maps are decoded through `dJ(...)`, which reconstructs a normal from tangent, bitangent, and geometric normal
- tangent-space reconstruction honors:
  - `TSPACE_ORTHOGONALIZE`
  - `TSPACE_RENORMALIZE`
  - `TSPACE_COMPUTE_BITANGENT`
- reflectivity is sampled from `tReflectivity.rgb` and squared with `dG(...)`
- gloss is sampled from `tReflectivity.a`
- non-GGX specular uses gloss to derive:
  - exponent `eA = 10 / log2(gloss * 0.968 + 0.03)`
  - scale term `eB = eA * (1 / (8*pi)) + (4 / (8*pi))`
- GGX specular uses:
  - `alpha = max((1 - gloss)^2, 1e-3)`
  - a Smith-style visibility term
  - a microfacet distribution term
- sky specular reflection is sampled from `tSkySpecular` through `em(...)`
- horizon occlusion is applied through `eu(...)`
- AO is sampled from `extras`, then squared before modulating diffuse terms
- emissive is sampled from `extras` and scaled by `uEmissiveScale`
- transparency can be standard alpha blend or dithered transparency depending on defines

Important helper behavior:

- `dM(uv, scaleBias)`:
  - samples from the merged `extras` texture
  - wraps with `fract(uv)`
  - applies `scaleBias`
  - on derivative-capable paths, applies a negative LOD bias when UV discontinuity is large
- `fi(view, normal, reflectivity, glossSq)`:
  - computes Fresnel reflectance from reflectivity and `uFresnel`
- `eu(reflectionDir, geometricNormal)`:
  - applies horizon occlusion as a clamped and squared term
- `em(reflectionDir, gloss)`:
  - samples the sky specular atlas using an octahedral-like mapping
  - chooses one of 8 gloss bands vertically
  - linearly blends between adjacent gloss bands

Feature-specific material behavior:

- `SKIN`
  - switches diffuse/specular model to the skin shader path
  - may also enable `SKIN_VERSION_2`
- `MICROFIBER`
  - replaces the default diffuse path with the microfiber/fuzz path
- `ANISO`
  - perturbs reflection direction and specular lobe using anisotropy direction and strength
- `REFRACTION`
  - adds refracted scene-color contribution after base diffuse/specular evaluation
- `DIFFUSE_UNLIT`
  - bypasses diffuse lighting and uses decoded albedo directly before adding specular/emissive/refraction

Rust implication:

- exact viewer parity requires a real tangent-space material path
- gloss is not roughness directly and should not be treated as a generic PBR roughness map without explicit conversion
- extras is a packed feature atlas, not a single-purpose texture

### Material Bind Inputs

When a material is bound, the JS provides at minimum:

- model/view/projection state
- sky matrix
- camera position
- fresnel
- alpha test
- horizon occlusion and smoothing
- diffuse coefficients from the sky
- light positions, directions, colors, and counts
- texture samplers:
  - `tAlbedo`
  - `tReflectivity`
  - `tNormal`
  - `tExtras`
  - `tSkySpecular`
- optional ranges for extras-based material features
- optional refraction texture

This means exact parity is a scene/material system problem, not only a mesh shader problem.

Exact uniforms confirmed from `Material.prototype.bind(...)`:

- transforms:
  - `uModelViewProjectionMatrix`
  - `uSkyMatrix`
- camera:
  - `uCameraPosition`
- core material:
  - `uFresnel`
  - `uAlphaTest`
  - `uHorizonOcclude`
  - `uHorizonSmoothing`
  - `uUVOffset`
- sky/light:
  - `uDiffuseCoefficients`
  - `uLightPositions`
  - `uLightDirections`
  - `uLightColors`
  - `uLightParams`
  - `uLightSpot`
  - `uShadowKernelRotation`
  - `uShadowMapSize`
  - `uShadowMatrices`
  - `uInvShadowMatrices`
  - `uShadowTexelPadProjections`
- skin:
  - `uSubdermisColor`
  - `uTransColor`
  - `uTransScatter`
  - `uFresnelColor`
  - `uFresnelOcc`
  - `uFresnelGlossMask`
  - `uFresnelIntegral`
  - `uTransIntegral`
  - `uSkinTransDepth`
  - `uTransSky`
  - `uSkinShadowBlur`
  - `uNormalSmooth`
- microfiber/aniso:
  - `uAnisoTangent`
  - `uAnisoStrength`
  - `uAnisoIntegral`
- refraction:
  - `uRefractionViewProjection`
  - `uRefractionRayDistance`
  - `uRefractionTint`
  - `uRefractionAlbedoTint`
  - `uRefractionIOREntry`
- extras ranges:
  - `uTexRangeSubdermis`
  - `uTexRangeTranslucency`
  - `uTexRangeFuzz`
  - `uTexRangeAniso`
  - `uTexRangeRefraction`
  - `uTexRangeEmissive`
  - `uTexRangeAO`
- emissive:
  - `uEmissiveScale`
- strip/debug:
  - `uStrips`
  - `uStripRes`

Exact texture samplers confirmed from the bind path:

- `tAlbedo`
- `tReflectivity`
- `tNormal`
- `tExtras`
- `tSkySpecular`
- optional `tRefraction`
- optional `tDepth0`
- optional `tDepth1`
- optional `tDepth2`

### Refraction Runtime

Refraction is a distinct runtime path, not only transparent blending.

Source references:

- `Material.prototype.bind(...)` around JS lines `187-189`
- `matfrag.glsl` in the shader bundle around JS line `455`

Confirmed behavior:

- the refracted direction is `refract(-viewDir, normal, 1 / IOR)`
- a refracted sample point is projected by `uRefractionViewProjection`
- projected UVs are converted to `[0, 1]`
- UVs are mirrored across tile boundaries using `floor`, `fract`, and parity checks
- the source color is sampled from `tRefraction`, which is the copied scene color after opaque passes
- refraction can be tinted by albedo when `useAlbedoTint` is enabled
- fresnel reflection contribution is removed from the refracted color before final tinting
- final refraction is blended into the lit surface using either:
  - `extras` refraction mask from `uTexRangeRefraction`
  - or `1.0` when `REFRACTION_NO_MASK_TEX`

Exact parameter behavior:

- `uRefractionIOREntry = 1 / IOR`
- if `newRefraction` is enabled:
  - `uRefractionRayDistance = 0.8 * mesh.bounds.averageExtent * max(IOR - 1, 0)`
- otherwise:
  - `uRefractionRayDistance = 1e10` when `distantBackground`
  - else `4 * mesh.bounds.maxExtent`
- `uRefractionTint` multiplies the refracted sample
- `uRefractionAlbedoTint` controls mixing between raw refraction and albedo-tinted refraction

Rust implication:

- matching refraction requires a copied opaque scene color target and a later refractive pass
- plain transparency is not equivalent

## Blend Modes

Blend behavior is selected from material `blend`.

Source references:

- JS material blend setup around line `172`

Observed blend family:

- `none`
- `alpha`
- additive-style modes also exist in the viewer material codepath

Rust implication:

- alpha handling must respect merged alpha texture content and material blend mode separately
- shadow and alpha-prepass behavior also depend on material alpha state, not just final shaded rendering

## Animation Data

Animation runtime centers on:

- `AnimatedObject`
- `AnimatedProperty`
- `Animation`
- `SceneAnimator`

Source references:

- keyframe unpacking around JS lines `24-47`
- transform evaluation around JS lines `34`, `61`, `65`, `67-68`

Keyframe storage behavior:

- keyframe data is shared in one byte stream
- JS exposes it as float, uint32, uint16, and byte views
- three packing modes are used:
  - packed mode `0`
  - packed mode `1`
  - packed mode `2`
- interpolation includes:
  - step
  - linear
  - spline-like curve cases

Property names confirmed in the JS:

- transform:
  - `Translation X/Y/Z`
  - `Rotation X/Y/Z`
  - `Scale X/Y/Z`
- visibility:
  - `Visible`
- material:
  - `OffsetU`
  - `OffsetV`
  - `EmissiveIntensity`
- fog/light/camera/playback:
  - `Red`
  - `Green`
  - `Blue`
  - `Brightness`
  - `Distance`
  - `Spot Angle`
  - `Spot Sharpness`
  - `Field Of View`
  - `CurrentAnimation`
  - `AnimationProgress`
  - `PlaybackSpeed`

## Transform Evaluation

Exact transform flow is:

1. `evaluateLocalTransformAtFramePercent`
2. `evaluateModelPartTransformAtFrame`
3. `getAnimatedLocalTransform`
4. `getWorldTransform`

Important behavior:

- pivot is subtracted during local evaluation and re-applied in model-part/world composition
- object-local transform order differs depending on scene-space vs model-part evaluation
- `modelPartIndex` remapping is a real runtime feature and must not be ignored
- `sceneScale` is applied to final translation

Source references:

- local transform: JS line `34`
- model-part transform: JS line `61`
- animated local transform: JS line `65`
- visibility: JS line `66`
- world transform: JS lines `67-68`

Rust implication:

- viewer and exporter must share the same transform solver
- any "simple node animation" that skips model-part remapping will be wrong
- cached frame evaluation behavior matters for performance, though not for the math itself

## Skinning and Deformation

Skinning is not standard joint-palette skinning first.

It is cluster-driven deformation.

Core flow:

1. `AnimatedObject.setupSkinningRig(...)`
2. `SkinningCluster.solveClusterTransformAtFrame(...)`
3. `SkinningRig.deformMeshVertices(...)`
4. `SkinningRig.deformMesh(...)`
5. `SceneAnimator.poseMeshes(...)`

Source references:

- setup and interpolation: JS line `33`
- simple cluster solve: JS line `330`
- additive cluster solve: JS line `329`
- cluster solve dispatch: JS line `331`
- original vertex/tangent/bitangent capture: JS lines `336-338`
- vertex deformation: JS line `339`
- deformation upload: JS line `345`
- scene pose loop: JS lines `301-302`

Important behavior:

- non-rigid skins deform vertex data directly
- rigid skins use object world transform only
- cluster matrices are solved in model-part space
- `modelPartScale * sceneScale` contributes to deformation
- when `unitScaleSkinnedMeshes` is enabled, the runtime normalizes display-matrix scale before deformation

Data retained by `SkinningRig`:

- cluster list
- `linkMapCount`
- `linkMapClusterIndices`
- `linkMapWeights`
- original vertex, normal, tangent, and bitangent snapshots
- a reusable `skinVertexTransform4x3` working buffer

### Skinning Binary Layout

From `SkinningRig(...)` and the current Rust parser:

- header words:
  - expected cluster count
  - expected vertex count
  - number of cluster links
  - original object index
  - rigid/non-rigid flag
  - tangent method
- each cluster contributes `7` header words
- after the cluster headers:
  - `linkMapCount` byte array per vertex
  - `linkMapClusterIndices` ushort array
  - `linkMapWeights` float array

### Deformation Behavior

`deformMeshVertices(...)` updates:

- positions
- normals
- tangents
- bitangents

This is critical:

- exact viewer parity requires deforming tangent space too, not only positions/normals
- normal mapping will remain wrong if tangents/bitangents are not deformed consistently

Exact `deformMeshVertices(...)` writeback layout:

- `b = a.stride / 4` is the float stride
- packed tangent-space data is written through a `Uint16Array` view of `dynamicVertexData`
- packed attribute byte offsets start at byte `20`
- if `secondaryTexCoord` is present, add `8` bytes to that base
- after that:
  - tangent packed offset = `g`
  - bitangent packed offset = `h = g + 4`
  - normal packed offset = `f = h + 4`

Per vertex:

- position floats are written to `d[b*m + 0..2]`
- tangent packed data is written through `e[p], e[p+1]`
- bitangent packed data is written through `e[r], e[r+1]`
- normal packed data is written through `e[s], e[s+1]`

Weighted transform behavior:

- `skinVertexTransform4x3` is the accumulated working matrix
- a single full-weight influence copies the cluster matrix directly
- otherwise cluster matrices are linearly accumulated by weight
- if any contributing cluster has `linkMode == 1`, the effective total weight is forced to `1`
- the final position is scaled by `1 / skinVertexWeights[l]`

Exact writeback behavior:

- transformed positions are multiplied by the supplied scale parameter `c`
- transformed normal, tangent, and bitangent are all normalized before repacking
- packing uses the same `32767.4 * (component / 2 + 0.5)` mapping as the source mesh data
- negative Z is encoded by adding `32768` to the second packed component

Rust implication:

- exact viewer parity requires CPU or GPU deformation that mirrors `deformMeshVertices`
- exporter-side "approximate skeleton export" is separate from viewer parity

## Visibility

Visibility is hierarchical and animated.

Two important code paths:

- `Animation.isVisibleAtFramePercent(...)`
- `SceneAnimator.updateVisibility(...)`

Source references:

- visibility test: JS line `66`
- scene application: JS line `313`

Behavior:

- visibility walks up the animated object parent chain
- a zero `Visible` value on any relevant ancestor hides the target
- visibility is applied to live `MeshRenderable.visible`
- visibility is specifically updated for submesh-linked scene objects through `subMeshObjectIndices` / `subMeshLiveIndices`

## Materials During Playback

Materials are animated by `SceneAnimator.updateMaterials(...)`.

Source references:

- JS lines `308-309`

Behavior:

- `OffsetU` updates material `uOffset`
- `OffsetV` updates material `vOffset`
- `EmissiveIntensity` updates material emissive scale

Important note:

- material animation is bound through animated objects of type `Material`
- material IDs are matched back to scene materials through `findMaterialIndexByPartIndex(...)`

Rust implication:

- UV scrolling and emissive animation are runtime features, not export-only metadata

## Lights During Playback

Lights are animated by `SceneAnimator.updateLights(...)`.

Source references:

- JS lines `303-306`

Behavior:

- light color channels can be animated from `Red/Green/Blue`
- light position/direction follow animated world transforms
- attenuation and spot properties can also animate
- light updates use animated world transforms when the light object is not fixed

## Fog During Playback

Fog is animated by `SceneAnimator.updateFog(...)`.

Source references:

- JS line `310`

Behavior:

- fog color channels can animate
- fog opacity, distance, and dispersion can animate
- fog rendering also depends on sky diffuse coefficients, light data, and attenuation mode

Exact fog draw inputs from `Fog.prototype.draw(...)`:

- `uDepthToZ`
- `uUnproject`
- `uInvViewMatrix`
- `uFogInvDistance`
- `uFogOpacity`
- `uFogDispersion`
- `uFogType`
- `uFogColor`
- `uFogIllum`
- `uLightMatrix`

For the IBL fog pass (`g == 0`):

- `uFogLightSphere` is built from `sky.diffuseCoefficients`
- coefficients `4..15` are scaled by `1 - dispersion`
- coefficients `16..35` are scaled by `1 - dispersion^2`

For light-driven fog passes:

- `uLightPosition`
- `uLightColor`
- `uSpotParams`
- `uLightAttenuation`
- optional shadow inputs:
  - `uShadowProj`
  - `uDitherOffset`
  - `uAABBMin`
  - `uAABBMax`
  - `uCylinder`
- depth sampler:
  - `tDepth`
- optional shadow sampler:
  - `uShadowMap`

Blend behavior:

- fog uses `blendFunc(ONE, ONE_MINUS_SRC_ALPHA)`
- it draws a fullscreen triangle once for the IBL term and once per light

Exact attenuation-related behavior visible from `Fog.prototype.draw(...)`:

- fog reconstructs rays from inverse view-projection state rather than using a single scalar depth fog curve
- it selects separate shader variants for:
  - IBL fog
  - spot fog
  - shadowed spot fog
  - point fog
  - shadowed point fog
- effective fog therefore depends on light type, shadow participation, reconstructed position, and fog type

Exact fog formulas from `fogfrag.glsl`:

- distance-to-fog response:
  - `B = distance * uFogInvDistance`
  - linear term: `min(B, 1.0)`
  - quadratic-like term: `1 - 1 / (1 + 16 * B * B)`
  - exponential term: `1 - exp(-3 * B)`
  - final unscaled fog amount:
    - `uFogType.x * linear + uFogType.y * quadratic + uFogType.z * exponential`
  - scaled by `uFogOpacity`
- IBL fog:
  - reconstruct world ray direction
  - evaluate SH-like fog lighting from `uFogLightSphere`
  - mix fog color toward fog-lit color by `uFogIllum`
  - output alpha is the fog amount
- spot/omni fog:
  - intersect the camera ray with the light volume
  - sample the segment in fixed steps
  - integrate attenuation minus already-applied fog accumulation
  - multiply by `A(segmentLength)` at the end
- directional fog:
  - base amount is `A(dot(rayDir, worldPos - cameraPos))`
  - directional term is modulated by `0.5 + 0.5 * dot(rayDir, -lightDir)`
  - dispersion further reshapes that directional term
- fog shadowing:
  - spot and directional shadowed variants subtract occluded fog contribution using shadow samples
  - shadowed spot uses stochastic sample offset from `uDitherOffset`

Rust implication:

- exact fog parity requires porting the fog shader family, not replacing it with one generic fullscreen fog pass

## Sky / Background

Sky runtime is handled by `Sky`.

Source references:

- `Sky` setup around JS lines `345-353`

Behavior:

- `sky.dat` or `sky.png` can provide specular/background content
- `backgroundMode` controls how the background is drawn
- `backgroundColor` is used directly when `backgroundMode < 1`
- `backgroundBrightness` modifies sky presentation
- diffuse coefficients are used for lighting

### Sky Data Behavior

Important findings:

- `sky.dat` is unpacked into a specular texture by planar channel slicing
- when `backgroundMode >= 1`, the runtime uses a background shader path
- for non-SH background mode, a derived background texture is rendered from the specular texture with brightness scaling
- for SH mode, background coefficients are pre-multiplied by `backgroundBrightness`

Exact background mode behavior confirmed from the JS:

- if `backgroundMode < 1`, the runtime only uses clear color
- if `backgroundMode >= 1`, it creates a background shader path
- if `backgroundMode == 3`, it binds spherical-harmonic coefficients through `uSkyCoefficients`
- otherwise it binds a derived `backgroundTexture` through `tSkyTexture`

Background draw state:

- alpha is driven by strip/debug fade:
  - `alpha = 0.07 + 0.94 * (1 - stripFade)`
- blend is enabled only if alpha is less than `1`
- depth writes are disabled while drawing the sky
- depth test is disabled during the sky draw and restored afterward

Important clear-color behavior:

- `Sky.setClearColor()` chooses clear color from transparent mode, background color, or a default dark sky tone

## Shadow Floor

Shadow floor is a real runtime subsystem.

Source references:

- `ShadowFloor` around JS lines `323-325`

Behavior:

- its own shader path
- depends on lights and shadow collector
- uses `desc.transform`
- participates in scene drawing before opaque mesh passes
- source geometry is a fixed two-triangle quad:
  - `[-1, 0, -1]`
  - `[-1, 0,  1]`
  - `[ 1, 0,  1]`
  - `[-1, 0, -1]`
  - `[ 1, 0,  1]`
  - `[ 1, 0, -1]`

Exact shadow floor inputs from `ShadowFloor.prototype.draw(...)`:

- `uModelViewProjectionMatrix`
- `uModelSkyMatrix`
- `uLightPositions`
- `uLightDirections`
- `uLightColors`
- `uLightParams`
- `uLightSpot`
- `uShadowKernelRotation`
- `uShadowMapSize`
- `uShadowMatrices`
- `uInvShadowMatrices`
- `uShadowTexelPadProjections`
- `uShadowCatcherParams`

Shadow floor samplers:

- `tDepth0`
- `tDepth1`
- `tDepth2`

Blend behavior:

- shadow floor uses `blendFunc(ZERO, SRC_COLOR)`
- depth writes are disabled for the pass
- the floor quad is drawn as two triangles

Exact shadow collector behavior from `shadowfloorfrag.glsl`:

- the pass includes `matshadows.glsl`
- floor shadow comparison overrides default behavior:
  - out-of-range shadow UVs are treated as fully lit
  - sampled depths greater than or equal to `1.0` are treated as fully lit
- it accumulates:
  - unshadowed direct light
  - shadowed direct light
- final floor color is approximately:
  - `(shadowedLight + epsilon) / (unshadowedLight + epsilon)`
- edge fade is driven by radial floor UV distance and `uShadowCatcherParams.z`
- final result is mixed toward white by edge fade and by `uShadowCatcherParams.y`
- `uShadowCatcherParams.x` mixes incoming light colors toward white before shadow application

This confirms the floor is a multiplicative shadow catcher, not a regular lit plane.

### Shadow Sampling Semantics

Source references:

- `matshadows.glsl` around JS line `459`

Confirmed behavior:

- shadow PCF uses a rotated 4-tap kernel driven by `uShadowKernelRotation`
- kernel size constant is:
  - desktop: `4 / 2048`
  - mobile: `4 / 1536`
- non-mobile paths bilinearly reconstruct compare results from four neighboring texels
- `uShadowTexelPadProjections` contributes normal-offset bias per light
- `uShadowMatrices` project current points into shadow space
- `uInvShadowMatrices` are additionally used on non-mobile paths to reconstruct a world-space distance-to-occluder term

Additional skin-shadow behavior:

- `eJ(...)` computes an approximate world-space travel distance from current point to inverse-projected shadow sample
- `SKIN_VERSION_2` uses that value for translucency depth effects

## Strip / Debug View

The JS viewer has a built-in diagnostic strip mode driven by `StripData`.

Source references:

- `StripData` around JS lines `355-357`
- `matstrips.glsl` around JS line `461`

Available strip labels:

- `Normals`
- `Albedo`
- `Reflectivity`
- `Gloss`
- `Topology`

Behavior:

- the strip menu animates horizontal cut lines across the frame
- active strips are selected by comparing skewed screen-space X against `uStrips`
- topology mode also forces:
  - `gloss = 0.5`
  - `reflectivity = vec3(0.1)`
- final strip display colors are:
  - normals: `normal * 0.5 + 0.5`
  - albedo: decoded albedo
  - reflectivity: decoded reflectivity
  - gloss: scalar gloss replicated to RGB
  - topology: `vec3(0.12) + 0.3 * diffuse + specular`
  - otherwise background/full shaded result

Rust implication:

- if the viewer wants full Marmoset parity, strip/debug mode is a real render feature and not just a UI overlay

## Cameras

Camera behavior is not only “main camera from scene.json”.

Source references:

- selected animated camera flow around JS lines `280-286`, `300`

Behavior:

- selected camera can come from animated camera objects
- user camera offsets are layered onto selected camera state
- animated camera parent/child transform relationships matter

Important camera runtime details:

- selected camera may be changed by UI controls
- user yaw/pitch offsets are stored separately from the animated camera base state
- when playback is paused, transforms and view state still refresh

Exact custom-view behavior:

- `selectDefaultCamera()` picks the camera whose animated object ID matches `selectedCamera` from animation data, otherwise camera `0`
- `setViewFromSelectedCamera()` copies from the selected camera view into the live scene view:
  - `pivot`
  - `rotation`
  - `radius`
  - `nearPlane`
  - `fov`
  - `limits`
- user orbit offsets are stored separately in:
  - `viewYawOffsets[selectedCameraIndex]`
  - `viewPitchOffsets[selectedCameraIndex]`
- `resetCustomView()` clears those offsets and restores the authored selected-camera view
- `updateUserCamera()`:
  - clears camera-child transform caches
  - compares current live scene-view yaw/pitch to the authored selected camera
  - stores the difference as custom offsets
  - computes the selected animated camera world transform at the current time
  - reconstructs a fixed replacement world/local transform so camera children keep following correctly

Rust implication:

- exact camera parity requires separating authored animated camera state from user orbit offsets
- simply replacing the scene camera with a free orbit camera is not equivalent

Rust implication:

- exact viewer parity requires a camera controller that can either follow the selected animated camera or allow user orbit offsets like the JS runtime

## Playback

Playback is managed by `SceneAnimator`.

Key controls:

- `selectedAnimationIndex`
- `selectedCameraIndex`
- `playbackSpeed`
- `scenePlaybackSpeed`
- `paused`
- `animationProgress`
- `totalSeconds`

Source references:

- JS lines `276-290`
- playback controls and timeline UI around JS lines `215-227`, `369-375`

Behavior:

- playback can be paused while transforms still refresh
- setting animation progress recalculates `totalSeconds`
- looping can auto-advance animations and roll turntables
- the viewer keeps drawing while playback, UI animation, or strip animation is active
- timeline dragging updates animation progress immediately

### Scene Update Order

Exact runtime scene update order from `SceneAnimator.updateScene()`:

1. update fog
2. update turntables
3. pose meshes
4. update lights
5. update materials
6. update visibility

This order matters.

Examples:

- lights should use the current animated transform state
- material UV/emissive updates happen after mesh posing in the current frame
- visibility is applied after scene object updates

There are two separate runtime update paths:

- `refreshTransformsOnly()`
- `updateScene()`

When playback is paused, the runtime still refreshes transforms and camera state through `refreshTransformsOnly()`.

### Draw Order

High-level scene draw order from `Scene.draw()`:

1. `sky.setClearColor()`
2. clear color/depth/stencil
3. enable depth test
4. draw sky
5. draw shadow floor
6. draw opaque visible non-blended non-refractive mesh renderables
7. alpha prepass with polygon offset and color mask disabled
8. transparent blended pass with `depthFunc(LEQUAL)` and `depthMask(false)`
9. disable blend and restore depth state
10. if refractive materials exist:
   - copy the main color target to a refraction surface
   - draw refractive renderables
11. if wireframe strip mode is active:
   - draw wire overlay for visible renderables

This means "matching Marmoset" is also a frame graph problem, not only shader math.

### Post / AA Behavior

Source references:

- `PostRender` around JS lines `229-246`
- viewer draw loop around JS lines `449-452`

Confirmed behavior:

- main post stack is built from `mainCamera.post`
- supported post feature toggles include:
  - sharpen
  - bloom
  - vignette
  - saturation
  - ACES tone mapping
  - optional color LUT
- anti-aliasing history is not temporal reconstruction in the heavy sense:
  - `sampleCount = 4` when enabled
  - projection is jittered using four fixed offsets:
    - `[-0.5, -0.5]`
    - `[0.5, -0.5]`
    - `[-0.5, 0.5]`
    - `[0.5, 0.5]`
  - `currentSample()` picks the current jitter
  - final present step blits through `aaShader`
- AA history is explicitly discarded when:
  - playback is active
  - camera/view changes
  - timeline scrubbing occurs
  - redraw is forced

Exact `mainCamera.post` JSON fields consumed by `PostRender`:

- `sharpen`
- `sharpenLimit`
- `bloomColor`
- `bloomSize`
- `vignette`
- `vignetteCurve`
- `saturation`
- `contrast`
- `brightness`
- `bias`
- `grain`
- `grainSharpness`
- `toneMap`
- `colorLUT`

Tone-map mapping:

- `toneMap == 1`
  - `REINHARD`
- `toneMap == 2`
  - `HEJL`
- `toneMap == 3`
  - `ACES`

Additional post behavior:

- bloom is enabled only if any `bloomColor.rgb * bloomColor.a` channel is non-zero
- vignette is enabled only if `vignette[3] != 0`
- saturation shader path is skipped only when all RGB scale/alpha pairs are identity
- contrast shader path is skipped only when contrast and brightness are identity
- grain is enabled only if `grain != 0`
- `colorLUT` is uploaded as a 1D RGB texture when present
- backing buffer prefers:
  - half float
  - then float on non-mobile
  - then unsigned byte fallback

Rust implication:

- exact final-frame parity is affected by post and AA state even if scene shading is otherwise correct

### Scene / Viewer Render Mode Toggles

Important runtime toggles visible from the JS:

- `stripData.selectedStrip`
  - controls strip/debug rendering and wire overlay activation
- `soloPart`
  - scene-level mesh isolation toggle
- `selectedPartIndex`
  - selected isolated part index
- `transparentBackground`
  - affects WebGL context alpha and clear color behavior
- `refractionSurface`
  - allocated only when refractive materials exist
- `mainDepth`
  - secondary depth-dependent path used by `drawSecondary(...)`

Additional findings:

- `drawSecondary(...)` only calls `fog.draw(this, depthTexture)`
- it is only invoked when `mainDepth` exists
- `mainDepth` is allocated only if `WEBGL_depth_texture` is available
- this means fog parity depends on a separate depth-texture-capable secondary pass
- `soloPart` and `selectedPartIndex` exist on `Scene`, but no active draw-time usage was found in this JS bundle
- current visible mesh filtering in this build is driven by:
  - animated visibility through `updateVisibility()`
  - strip/debug state
  - material blend/refraction state
  - not by a discovered solo-part branch in `Scene.draw()`

Rust implication:

- full viewer parity is not only asset loading; it also requires these scene-level render mode switches

## What The Rust Viewer Must Eventually Match

For true parity, the Rust runtime/viewer should execute these subsystems directly:

1. animated object transforms
2. visibility evaluation
3. material animation
4. light animation
5. fog animation
6. sky/background behavior
7. cluster-based skin deformation
8. material shader feature flags
9. extras texture coordinate ranges and merged texture semantics
10. camera selection and animated camera behavior

## Current Implementation Notes

As of this document:

- transform evaluation has been ported in principle
- cluster deformation has been partially ported into the viewer path
- albedo and alpha atlasing are present
- UV offset animation and visibility are present
- full material parity is still incomplete
- full sky/fog/light/camera runtime parity is still incomplete
- tangent and bitangent data are now preserved in the Rust mesh decoder for later shader parity work

## Remaining Spec Work

The main reverse-engineering gaps still worth documenting explicitly are:

1. cleanup of remaining encoding artifacts in this document
2. optional confirmation of dormant `soloPart` / `selectedPartIndex` state against other viewer bundle revisions
3. optional cross-check of post shader math against `postfrag.glsl` if pixel-perfect output becomes necessary

Once these are written down, the remaining viewer work becomes a direct subsystem port.

## Next Porting Order

Recommended next implementation order:

1. preserve tangent and bitangent data end-to-end
2. port normal-map shading using the JS tangent-space path
3. port merged reflectivity/gloss and extras texture semantics
4. port scene lights into the WGPU runtime rather than a single fallback light
5. port sky/background behavior from `Sky`
6. port fog/shadow-floor rendering behavior
7. port animated camera selection and offsets
