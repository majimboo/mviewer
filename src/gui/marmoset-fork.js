(function () {
  function applyControlFork() {
    if (!window.marmoset || !window.marmoset.WebViewer || window.__mviewerControlsPatched) {
      return;
    }

    window.__mviewerControlsPatched = true;

    const WebViewer = window.marmoset.WebViewer;
    const originalBindInput = WebViewer.prototype.bindInput;

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
      }
    };
  }

  applyControlFork();
})();
