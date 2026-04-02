use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub fn write_viewer(output_dir: &Path, gltf_name: &str, runtime_name: &str) -> Result<()> {
    let html = build_html(gltf_name);
    let html = html.replace("__RUNTIME_JSON__", runtime_name);
    fs::write(output_dir.join("viewer.html"), html)
        .with_context(|| format!("failed to write {}", output_dir.join("viewer.html").display()))?;
    Ok(())
}

fn build_html(gltf_name: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>mviewer Runtime Player</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg0: #0b1118;
      --bg1: #101b27;
      --panel: rgba(9, 15, 22, 0.78);
      --line: rgba(189, 220, 255, 0.18);
      --text: #e8f0f7;
      --muted: #8ca0b3;
      --accent: #8fd0ff;
    }}
    html, body {{
      margin: 0;
      height: 100%;
      overflow: hidden;
      background:
        radial-gradient(circle at top, rgba(75, 125, 182, 0.18), transparent 38%),
        linear-gradient(180deg, var(--bg1), var(--bg0));
      color: var(--text);
      font: 13px/1.4 Consolas, "SFMono-Regular", monospace;
    }}
    #app {{
      width: 100%;
      height: 100%;
      position: relative;
    }}
    #hud {{
      position: absolute;
      top: 16px;
      left: 16px;
      width: 340px;
      padding: 14px 16px;
      background: var(--panel);
      border: 1px solid var(--line);
      backdrop-filter: blur(18px);
      box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
      z-index: 10;
    }}
    h1 {{
      margin: 0 0 10px;
      font-size: 14px;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .row {{
      display: grid;
      grid-template-columns: 110px 1fr;
      gap: 8px;
      align-items: center;
      margin-top: 8px;
    }}
    label, .muted {{
      color: var(--muted);
    }}
    select, input[type="range"], button {{
      width: 100%;
      box-sizing: border-box;
      background: rgba(255, 255, 255, 0.04);
      color: var(--text);
      border: 1px solid var(--line);
      padding: 6px 8px;
    }}
    button {{
      cursor: pointer;
    }}
    #status {{
      margin-top: 10px;
      min-height: 2.8em;
      color: var(--muted);
      white-space: pre-wrap;
    }}
  </style>
</head>
<body>
  <div id="app">
    <div id="hud">
      <h1>mviewer Runtime Player</h1>
      <div class="row">
        <label for="clip">Clip</label>
        <select id="clip"></select>
      </div>
      <div class="row">
        <label for="frame">Frame</label>
        <input id="frame" type="range" min="0" max="0" step="1" value="0">
      </div>
      <div class="row">
        <label for="speed">Speed</label>
        <input id="speed" type="range" min="0" max="3" step="0.05" value="1">
      </div>
      <div class="row">
        <label>Controls</label>
        <button id="toggle">Pause</button>
      </div>
      <div id="status">Loading...</div>
    </div>
  </div>
  <script type="importmap">
    {{
      "imports": {{
        "three": "https://cdn.jsdelivr.net/npm/three@0.167.1/build/three.module.js",
        "three/addons/": "https://cdn.jsdelivr.net/npm/three@0.167.1/examples/jsm/"
      }}
    }}
  </script>
  <script type="module">
    import * as THREE from 'three';
    import {{ OrbitControls }} from 'three/addons/controls/OrbitControls.js';
    import {{ GLTFLoader }} from 'three/addons/loaders/GLTFLoader.js';

    const GLTF_NAME = {gltf_name:?};
    const app = document.getElementById('app');
    const clipSelect = document.getElementById('clip');
    const frameSlider = document.getElementById('frame');
    const speedSlider = document.getElementById('speed');
    const toggleButton = document.getElementById('toggle');
    const status = document.getElementById('status');

    const renderer = new THREE.WebGLRenderer({{ antialias: true }});
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    renderer.setSize(window.innerWidth, window.innerHeight);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.shadowMap.enabled = true;
    app.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, window.innerWidth / window.innerHeight, 0.01, 1000);
    camera.position.set(0, 1.5, 4);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(0, 1, 0);
    controls.update();

    const ambient = new THREE.AmbientLight(0xffffff, 0.15);
    scene.add(ambient);

    const clock = new THREE.Clock();
    const loader = new GLTFLoader();
    const runtimeState = {{
      clipIndex: 0,
      frameIndex: 0,
      playing: true,
      lastAppliedKey: null,
      root: null,
      nodeMap: new Map(),
      materials: [],
      lightsByIndex: new Map(),
      camerasByNodeIndex: new Map(),
      runtime: null,
      frameAccumulator: 0,
      floor: null,
    }};

    window.addEventListener('resize', () => {{
      camera.aspect = window.innerWidth / window.innerHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(window.innerWidth, window.innerHeight);
    }});

    toggleButton.addEventListener('click', () => {{
      runtimeState.playing = !runtimeState.playing;
      toggleButton.textContent = runtimeState.playing ? 'Pause' : 'Play';
    }});

    clipSelect.addEventListener('change', () => {{
      runtimeState.clipIndex = Number(clipSelect.value) || 0;
      runtimeState.frameIndex = 0;
      runtimeState.frameAccumulator = 0;
      refreshFrameSlider();
      applyCurrentFrame(true);
    }});

    frameSlider.addEventListener('input', () => {{
      runtimeState.frameIndex = Number(frameSlider.value) || 0;
      runtimeState.frameAccumulator = 0;
      applyCurrentFrame(true);
    }});

    async function main() {{
      const gltfJson = await fetch(GLTF_NAME).then((response) => response.json());
      const runtimePayload = await fetch('__RUNTIME_JSON__').then((response) => response.json());
      runtimeState.runtime = runtimePayload?.runtime ?? null;
      if (!runtimeState.runtime) {{
        throw new Error('Missing MVIEWER_marmoset_runtime payload');
      }}

      const gltf = await loader.loadAsync(GLTF_NAME);
      runtimeState.root = gltf.scene;
      scene.add(gltf.scene);
      mapRuntimeObjects(gltf);
      configureSceneFromRuntime(gltfJson, runtimePayload);
      buildClipUi();
      applyCurrentFrame(true);
      status.textContent = buildStatus();
      animate();
    }}

    function mapRuntimeObjects(gltf) {{
      runtimeState.materials = gltf.parser.json.materials ?? [];
      runtimeState.nodeMap.clear();
      runtimeState.lightsByIndex.clear();
      runtimeState.camerasByNodeIndex.clear();

      gltf.scene.traverse((object) => {{
        const association = gltf.parser.associations.get(object);
        const nodeIndex = association?.nodes;
        if (nodeIndex === undefined) {{
          return;
        }}
        runtimeState.nodeMap.set(nodeIndex, object);
        if (object.isLight) {{
          const lightIndex = object.userData?.gltfExtensions?.KHR_lights_punctual?.light;
          if (lightIndex !== undefined) {{
            runtimeState.lightsByIndex.set(lightIndex, object);
          }}
        }}
        if (object.isCamera) {{
          runtimeState.camerasByNodeIndex.set(nodeIndex, object);
        }}
      }});
    }}

    function configureSceneFromRuntime(gltfJson, mviewer) {{
      const sky = mviewer?.sourceScene?.sky ?? mviewer?.sky;
      const fog = mviewer?.sourceScene?.fog ?? mviewer?.fog;
      const shadowFloor = mviewer?.sourceScene?.shadowFloor ?? mviewer?.shadowFloor;

      if (sky?.backgroundColor?.length >= 3) {{
        scene.background = new THREE.Color(sky.backgroundColor[0], sky.backgroundColor[1], sky.backgroundColor[2]);
      }}
      if (fog?.color?.length >= 3) {{
        scene.fog = new THREE.Fog(
          new THREE.Color(fog.color[0], fog.color[1], fog.color[2]),
          0.1,
          Math.max(1.0, fog.distance ?? 100.0),
        );
      }}
      if (shadowFloor?.transform) {{
        const floor = new THREE.Mesh(
          new THREE.PlaneGeometry(10, 10),
          new THREE.ShadowMaterial({{ opacity: shadowFloor.alpha ?? 0.45 }})
        );
        floor.receiveShadow = true;
        floor.matrixAutoUpdate = false;
        floor.matrix.fromArray(shadowFloor.transform);
        floor.matrix.decompose(floor.position, floor.quaternion, floor.scale);
        runtimeState.floor = floor;
        scene.add(floor);
      }}
    }}

    function buildClipUi() {{
      const clips = runtimeState.runtime.clips ?? [];
      clipSelect.innerHTML = '';
      clips.forEach((clip, index) => {{
        const option = document.createElement('option');
        option.value = String(index);
        option.textContent = `${{index}}: ${{clip.name}}`;
        clipSelect.appendChild(option);
      }});
      runtimeState.clipIndex = Math.min(runtimeState.runtime.selectedAnimation ?? 0, Math.max(0, clips.length - 1));
      clipSelect.value = String(runtimeState.clipIndex);
      refreshFrameSlider();
    }}

    function refreshFrameSlider() {{
      const clip = currentClip();
      const maxFrame = Math.max(0, (clip?.sampledFrames?.length ?? 1) - 1);
      frameSlider.max = String(maxFrame);
      frameSlider.value = String(Math.min(runtimeState.frameIndex, maxFrame));
    }}

    function currentClip() {{
      return runtimeState.runtime?.clips?.[runtimeState.clipIndex] ?? null;
    }}

    function applyCurrentFrame(force = false) {{
      const clip = currentClip();
      if (!clip) {{
        return;
      }}
      const frames = clip.sampledFrames ?? [];
      if (!frames.length) {{
        return;
      }}
      runtimeState.frameIndex = Math.min(runtimeState.frameIndex, frames.length - 1);
      frameSlider.value = String(runtimeState.frameIndex);
      const frame = frames[runtimeState.frameIndex];
      const applyKey = `${{runtimeState.clipIndex}}:${{runtimeState.frameIndex}}`;
      if (!force && runtimeState.lastAppliedKey === applyKey) {{
        return;
      }}
      runtimeState.lastAppliedKey = applyKey;

      for (const objectState of frame.objects) {{
        const nodeIndex = objectState.nodeIndex;
        if (nodeIndex !== null && nodeIndex !== undefined) {{
          const object = runtimeState.nodeMap.get(nodeIndex);
          if (object) {{
            applyObjectState(object, objectState);
          }}
        }}
      }}

      status.textContent = buildStatus();
    }}

    function applyObjectState(object, objectState) {{
      const worldMatrix = objectState.worldMatrix;
      if (Array.isArray(worldMatrix) && worldMatrix.length === 16) {{
        object.matrixAutoUpdate = false;
        const matrix = new THREE.Matrix4().fromArray(worldMatrix);
        if (object.parent) {{
          const parentWorldInv = new THREE.Matrix4().copy(object.parent.matrixWorld).invert();
          matrix.premultiply(parentWorldInv);
        }}
        matrix.decompose(object.position, object.quaternion, object.scale);
        object.matrix.compose(object.position, object.quaternion, object.scale);
      }}

      const runtimeProperties = objectState.runtimeProperties ?? {{}};
      if (runtimeProperties.VisibleInherited !== undefined) {{
        object.visible = Boolean(runtimeProperties.VisibleInherited);
      }}

      if (object.isLight) {{
        applyLightState(object, runtimeProperties);
      }}
      if (object.isCamera) {{
        applyCameraState(object, runtimeProperties);
      }}
      if (object.material) {{
        applyMaterialState(object.material, runtimeProperties);
      }}
      if (Array.isArray(object.children)) {{
        for (const child of object.children) {{
          if (child.material) {{
            applyMaterialState(child.material, runtimeProperties);
          }}
        }}
      }}
    }}

    function applyLightState(light, props) {{
      const r = props.Red ?? 1;
      const g = props.Green ?? 1;
      const b = props.Blue ?? 1;
      light.color.setRGB(r, g, b);
      if (props.Brightness !== undefined) {{
        light.intensity = props.Brightness;
      }}
      if (props.Distance !== undefined && 'distance' in light) {{
        light.distance = props.Distance;
      }}
      if (props['Spot Angle'] !== undefined && 'angle' in light) {{
        light.angle = THREE.MathUtils.degToRad(props['Spot Angle'] * 0.5);
      }}
      if (props['Spot Sharpness'] !== undefined && 'penumbra' in light) {{
        light.penumbra = THREE.MathUtils.clamp(1 - props['Spot Sharpness'], 0, 1);
      }}
    }}

    function applyCameraState(cam, props) {{
      if (props['Field Of View'] !== undefined) {{
        cam.fov = props['Field Of View'];
        cam.updateProjectionMatrix();
      }}
    }}

    function applyMaterialState(material, props) {{
      const materials = Array.isArray(material) ? material : [material];
      for (const entry of materials) {{
        if (!entry) {{
          continue;
        }}
        if (props.EmissiveIntensity !== undefined && entry.emissive) {{
          entry.emissiveIntensity = props.EmissiveIntensity;
        }}
        if (props.OffsetU !== undefined || props.OffsetV !== undefined) {{
          if (entry.map) {{
            entry.map.offset.set(props.OffsetU ?? entry.map.offset.x, props.OffsetV ?? entry.map.offset.y);
            entry.map.needsUpdate = true;
          }}
          if (entry.normalMap) {{
            entry.normalMap.offset.set(props.OffsetU ?? entry.normalMap.offset.x, props.OffsetV ?? entry.normalMap.offset.y);
            entry.normalMap.needsUpdate = true;
          }}
          if (entry.metalnessMap) {{
            entry.metalnessMap.offset.set(props.OffsetU ?? entry.metalnessMap.offset.x, props.OffsetV ?? entry.metalnessMap.offset.y);
            entry.metalnessMap.needsUpdate = true;
          }}
          if (entry.roughnessMap) {{
            entry.roughnessMap.offset.set(props.OffsetU ?? entry.roughnessMap.offset.x, props.OffsetV ?? entry.roughnessMap.offset.y);
            entry.roughnessMap.needsUpdate = true;
          }}
        }}
      }}
    }}

    function buildStatus() {{
      const clip = currentClip();
      if (!clip) {{
        return 'No runtime clip data';
      }}
      return [
        `Scene: ${{GLTF_NAME}}`,
        `Clip: ${{clip.name}}`,
        `Frame: ${{runtimeState.frameIndex}} / ${{Math.max(0, (clip.sampledFrames?.length ?? 1) - 1)}}`,
        `Objects: ${{clip.sampledFrames?.[runtimeState.frameIndex]?.objects?.length ?? 0}}`
      ].join('\\n');
    }}

    function animate() {{
      requestAnimationFrame(animate);
      const clip = currentClip();
      const delta = clock.getDelta();
      if (clip && runtimeState.playing) {{
        const fps = clip.originalFPS || 30;
        runtimeState.frameAccumulator += delta * Number(speedSlider.value || 1);
        const step = 1 / fps;
        while (runtimeState.frameAccumulator >= step) {{
          runtimeState.frameAccumulator -= step;
          runtimeState.frameIndex = (runtimeState.frameIndex + 1) % Math.max(1, clip.sampledFrames.length);
          applyCurrentFrame();
        }}
      }}
      controls.update();
      renderer.render(scene, camera);
    }}

    main().catch((error) => {{
      console.error(error);
      status.textContent = `Failed to load runtime viewer\\n${{error.message}}`;
    }});
  </script>
</body>
</html>
"#,
        gltf_name = gltf_name
    )
}
