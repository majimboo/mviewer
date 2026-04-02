(function () {
  function identityMatrix() {
    if (window.Matrix && typeof window.Matrix.identity === 'function') {
      return window.Matrix.identity();
    }
    return [1, 0, 0, 0,
            0, 1, 0, 0,
            0, 0, 1, 0,
            0, 0, 0, 1];
  }

  function collectAnimationSamples(scene, animation, sceneScale) {
    if (!animation || !Array.isArray(animation.animatedObjects) || !animation.animatedObjects.length) {
      return [];
    }

    const totalFrames = Math.max(0, animation.totalFrames || 0);
    const totalSeconds = Math.max(0, animation.totalSeconds || 0);
    const sampleCount = totalFrames > 0 ? Math.min(totalFrames + 1, 240) : 0;
    if (!sampleCount) {
      return [];
    }

    const samples = [];
    for (let sampleIndex = 0; sampleIndex < sampleCount; sampleIndex += 1) {
      const frame = sampleCount === 1 ? 0 : (sampleIndex * totalFrames) / (sampleCount - 1);
      const seconds = totalFrames > 0 ? (frame / totalFrames) * totalSeconds : 0;
      const objects = animation.animatedObjects.map(obj => {
        const matrix = identityMatrix();
        animation.getWorldTransform(obj.id, seconds, matrix, sceneScale, true);
        return {
          id: obj.id,
          worldMatrix: Array.from(matrix)
        };
      });
      samples.push({
        sampleIndex,
        frame,
        seconds,
        objects
      });
    }
    return samples;
  }

  function applyControlFork() {
    if (!window.marmoset || !window.marmoset.WebViewer || window.__mviewerControlsPatched) {
      return;
    }

    window.__mviewerControlsPatched = true;

    const WebViewer = window.marmoset.WebViewer;
    const originalBindInput = WebViewer.prototype.bindInput;
    const UI = window.UI;

    function hideNode(node) {
      if (!node || !node.style) {
        return;
      }
      node.style.display = 'none';
      node.style.pointerEvents = 'none';
      node.style.visibility = 'hidden';
    }

    function stripViewerChrome(viewer) {
      const root = viewer && viewer.domRoot;
      if (!root) {
        return;
      }

      const selectors = [
        'div[title="Made with Marmoset Toolbag"]',
        'input[title="Help"]',
        'input[title="Full Screen"]',
        'a[href*="marmoset.co/viewer"]',
        'a[href*="utm_source=inapp"]',
      ];

      selectors.forEach(selector => {
        root.querySelectorAll(selector).forEach(node => {
          hideNode(node);
        });
      });

      const ui = viewer.ui;
      if (ui && ui.sigCluster && ui.sigCluster.style) {
        hideNode(ui.sigCluster);
      }
      if (ui && ui.menuCluster) {
        const menuRoot = ui.menuCluster;
        const visibleButtons = [];
        menuRoot.querySelectorAll('input[type="image"]').forEach(node => {
          const title = (node.getAttribute('title') || '').trim();
          if (title === 'Layer Views') {
            visibleButtons.push(node);
          }
        });
        visibleButtons.forEach((node, index) => {
          node.style.left = '0px';
          node.style.top = `${index * (window.marmoset && window.marmoset.largeUI ? 48 : 24)}px`;
          node.style.bottom = 'auto';
        });
        if (menuRoot.style) {
          menuRoot.style.top = window.marmoset && window.marmoset.largeUI ? '22px' : '16px';
          if (window.marmoset && window.marmoset.largeUI) {
            menuRoot.style.width = '72px';
            menuRoot.style.height = `${Math.max(48, visibleButtons.length * 48)}px`;
          } else {
            menuRoot.style.width = '36px';
            menuRoot.style.height = `${Math.max(24, visibleButtons.length * 24)}px`;
          }
        }
      }
      if (ui && ui.helpOverlay && ui.helpOverlay.parentNode) {
        ui.helpOverlay.parentNode.removeChild(ui.helpOverlay);
        ui.helpOverlay.active = false;
      }

      root.querySelectorAll('*').forEach(node => {
        const text = (node.textContent || '').trim().toLowerCase();
        const href = typeof node.getAttribute === 'function' ? (node.getAttribute('href') || '') : '';
        const title = typeof node.getAttribute === 'function' ? (node.getAttribute('title') || '') : '';
        if (
          text.includes('www.marmoset.co/viewer') ||
          href.includes('marmoset.co/viewer') ||
          title.includes('Marmoset Toolbag')
        ) {
          hideNode(node);
        }
      });
    }

    function installChromeObserver(viewer) {
      const root = viewer && viewer.domRoot;
      if (!root || root.__mviewerChromeObserverInstalled) {
        return;
      }
      root.__mviewerChromeObserverInstalled = true;
      const observer = new MutationObserver(function () {
        stripViewerChrome(viewer);
      });
      observer.observe(root, { childList: true, subtree: true, attributes: true });
      stripViewerChrome(viewer);
    }

    if (UI && UI.prototype && !UI.prototype.__mviewerChromePatched) {
      UI.prototype.__mviewerChromePatched = true;
      const originalShowActiveView = UI.prototype.showActiveView;
      UI.prototype.showActiveView = function () {
        originalShowActiveView.call(this);
        if (this && this.viewer) {
          stripViewerChrome(this.viewer);
        }
      };
    }

    WebViewer.prototype.bindInput = function () {
      originalBindInput.call(this);
      if (!this.input) {
        return;
      }

      this.input.onDrag.length = 0;
      this.input.onDrag.push(function (_pointer, _source, dx, dy) {
        const damping = 1 - 2.2 / (Math.sqrt(dx * dx + dy * dy) + 2.2);
        const view = this.scene.view;
        view.rotation[1] -= 0.28 * dx * damping;
        view.rotation[0] -= 0.28 * dy * damping;
        view.rotation[0] = Math.max(-90, Math.min(90, view.rotation[0]));
        view.updateView();
        this.wake();
      }.bind(this));

      this.input.onPan.length = 0;
      this.input.onPan.push(function (dx, dy) {
        const view = this.scene.view;
        const scale = view.fov / 45 * 0.7 * (view.radius / this.domRoot.clientHeight);
        const panX = -dx * scale;
        const panY = dy * scale;
        view.pivot[0] += panX * view.transform[0] + panY * view.transform[4];
        view.pivot[1] += panX * view.transform[1] + panY * view.transform[5];
        view.pivot[2] += panX * view.transform[2] + panY * view.transform[6];
        view.updateView();
        this.wake();
      }.bind(this));

      this.input.onPan2.length = 0;
      this.input.onPan2.push(function (dx, dy) {
        const damping = 1 - 2.2 / (Math.sqrt(dx * dx + dy * dy) + 2.2);
        this.scene.lights.rotation -= 0.28 * dx * damping;
        this.wake();
      }.bind(this));

      this.input.onZoom.length = 0;
      this.input.onZoom.push(function (delta) {
        const view = this.scene.view;
        view.radius *= 1 - 0.0015 * delta;
        view.radius = Math.max(0.001, Math.min(1000, view.radius));
        view.updateView();
        this.wake();
      }.bind(this));

      this.input.onDoubleTap.length = 0;
      this.input.onDoubleTap.push(function () {
        this.scene.view.reset();
        this.scene.sceneAnimator && this.scene.sceneAnimator.resetCustomView();
        this.wake();
      }.bind(this));

      stripViewerChrome(this);
      installChromeObserver(this);
    };

    window.mviewerMarmosetFork = {
      version: '1',
      describeScene(scene) {
        if (!scene) return null;
        return {
          title: scene.metaData && scene.metaData.title,
          author: scene.metaData && scene.metaData.author,
          meshCount: Array.isArray(scene.meshes) ? scene.meshes.length : 0,
          materialCount: Array.isArray(scene.materialsList) ? scene.materialsList.length : 0,
          cameraCount: scene.cameras && typeof scene.cameras.count === 'number' ? scene.cameras.count : 0,
          lightCount: scene.lights && typeof scene.lights.count === 'number' ? scene.lights.count : 0,
          animationCount: scene.sceneAnimator && Array.isArray(scene.sceneAnimator.animations) ? scene.sceneAnimator.animations.length : 0
        };
      },
      collectRuntimeSnapshot(viewer) {
        const scene = viewer && viewer.scene;
        if (!scene || !scene.sceneLoaded) {
          return null;
        }

        const animator = scene.sceneAnimator || null;
        const animations = Array.isArray(animator && animator.animations) ? animator.animations : [];
        const selectedAnimationIndex = animator && typeof animator.selectedAnimationIndex === 'number'
          ? animator.selectedAnimationIndex
          : -1;

        const meshObjects = selectedAnimationIndex >= 0 && animations[selectedAnimationIndex]
          ? (animations[selectedAnimationIndex].meshObjects || [])
          : [];
        const selectedAnimation = selectedAnimationIndex >= 0 ? animations[selectedAnimationIndex] : null;
        const animationSamples = selectedAnimation
          ? collectAnimationSamples(scene, selectedAnimation, animator && typeof animator.sceneScale === 'number' ? animator.sceneScale : 1)
          : [];

        const cameras = [];
        if (scene.cameras && typeof scene.cameras.count === 'number') {
          for (let i = 0; i < scene.cameras.count; i += 1) {
            cameras.push({
              index: i,
              name: scene.cameras[i] && scene.cameras[i].name ? scene.cameras[i].name : `Camera ${i + 1}`,
              fov: scene.cameras[i] && typeof scene.cameras[i].fov === 'number' ? scene.cameras[i].fov : null,
              near: scene.cameras[i] && typeof scene.cameras[i].nearPlane === 'number' ? scene.cameras[i].nearPlane : null,
              far: scene.cameras[i] && typeof scene.cameras[i].farPlane === 'number' ? scene.cameras[i].farPlane : null,
              transform: scene.cameras[i] && scene.cameras[i].transform ? Array.from(scene.cameras[i].transform) : null
            });
          }
        }

        const lights = [];
        if (scene.lights && typeof scene.lights.count === 'number') {
          for (let i = 0; i < scene.lights.count; i += 1) {
            lights.push({
              index: i,
              color: scene.lights.getLightColor ? Array.from(scene.lights.getLightColor(i)) : null,
              position: scene.lights.getLightPos ? Array.from(scene.lights.getLightPos(i)) : null,
              direction: scene.lights.getLightDir ? Array.from(scene.lights.getLightDir(i)) : null,
              parameters: scene.lights.parameters ? Array.from(scene.lights.parameters.slice(i * 3, i * 3 + 3)) : null,
              spot: scene.lights.spot ? Array.from(scene.lights.spot.slice(i * 3, i * 3 + 3)) : null
            });
          }
        }

        const meshes = Array.isArray(scene.meshes) ? scene.meshes.map((mesh, index) => ({
          index,
          name: mesh && mesh.name ? mesh.name : `Mesh ${index + 1}`,
          vertexCount: typeof mesh.vertexCount === 'number' ? mesh.vertexCount : null,
          indexCount: typeof mesh.indexCount === 'number' ? mesh.indexCount : null,
          displayMatrix: mesh && mesh.displayMatrix ? Array.from(mesh.displayMatrix) : null
        })) : [];

        return {
          source: 'mviewer-marmoset-fork',
          version: 1,
          selectedAnimationIndex,
          selectedCameraIndex: animator && typeof animator.selectedCameraIndex === 'number'
            ? animator.selectedCameraIndex
            : -1,
          animationProgress: animator && typeof animator.animationProgress === 'number'
            ? animator.animationProgress
            : null,
          totalSeconds: animator && typeof animator.totalSeconds === 'number'
            ? animator.totalSeconds
            : null,
          scene: this.describeScene(scene),
          cameras,
          lights,
          meshes,
          sampledAnimation: selectedAnimation ? {
            index: selectedAnimationIndex,
            name: selectedAnimation.name || `Animation ${selectedAnimationIndex + 1}`,
            samples: animationSamples
          } : null,
          animations: animations.map((animation, index) => ({
            index,
            name: animation.name || `Animation ${index + 1}`,
            totalSeconds: animation.totalSeconds,
            totalFrames: animation.totalFrames,
            originalFPS: animation.originalFPS,
            animatedObjects: Array.isArray(animation.animatedObjects) ? animation.animatedObjects.map(obj => ({
              id: obj.id,
              name: obj.name,
              parentIndex: obj.parentIndex,
              sceneObjectType: obj.sceneObjectType,
              modelPartIndex: obj.modelPartIndex,
              modelPartFPS: obj.modelPartFPS,
              modelPartScale: obj.modelPartScale,
              animationLength: obj.animationLength,
              totalFrames: obj.totalFrames,
              startTime: obj.startTime,
              endTime: obj.endTime,
              meshIndex: obj.meshIndex,
              materialIndex: obj.materialIndex,
              lightIndex: obj.lightIndex,
              skinningRigIndex: obj.skinningRigIndex,
              pivot: obj.pivot ? { x: obj.pivot.x, y: obj.pivot.y, z: obj.pivot.z } : null
            })) : []
          })),
          meshBindings: meshObjects.map(obj => ({
            animatedObjectId: obj.id,
            name: obj.name,
            meshIndex: obj.meshIndex,
            materialIndex: obj.materialIndex,
            modelPartIndex: obj.modelPartIndex,
            skinningRigIndex: obj.skinningRigIndex,
            displayMatrix: obj.mesh && obj.mesh.displayMatrix ? Array.from(obj.mesh.displayMatrix) : null
          })),
          materials: Array.isArray(scene.materialsList) ? scene.materialsList.map((material, index) => ({
            index,
            name: material && material.desc && material.desc.name ? material.desc.name : `Material ${index + 1}`,
            desc: material && material.desc ? material.desc : null
          })) : []
        };
      }
    };
  }

  applyControlFork();
})();
