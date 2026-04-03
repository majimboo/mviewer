use std::net::TcpListener;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::{ProjectDirs, UserDirs};
use rfd::FileDialog;
use serde::{Deserialize, Serialize};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use wry::{WebContext, WebViewBuilder};

use crate::{
    js_export::{
        GuiExportFormat, GuiExportOptions, JsExportScene, export_from_js_scene_path_with_options_and_progress,
    },
};

use super::icon::load_app_icon;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const APP_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>mviewer</title>
  <link rel="stylesheet" href="/app.css" />
</head>
<body>
  <div class="toolbar">
    <div class="toolbar-row">
      <div class="brand" aria-label="mviewer by github.com/majimboo">
        <div class="brand-title">mviewer</div>
        <div class="brand-subtitle">github.com/majimboo</div>
      </div>
      <div class="path" id="inputPath">Open a .mview file</div>
      <button onclick="openSourceDialog()" title="Open .mview">
        <span class="button-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 19.5V6.75A1.75 1.75 0 0 1 4.75 5h5.1l1.65 2h7.75A1.75 1.75 0 0 1 21 8.75V10" />
            <path d="M4.75 19h11.9a1.5 1.5 0 0 0 1.45-1.1l1.45-5.15A1.5 1.5 0 0 0 18.1 11H6.05a1.5 1.5 0 0 0-1.45 1.1L3.3 16.75A1.5 1.5 0 0 0 4.75 19Z" />
          </svg>
        </span>
        <span class="button-label">Open .mview</span>
      </button>
      <button class="primary" id="exportButton" onclick="openExportDialog()" disabled title="Export Scene">
        <span class="button-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 4.5v10.25" />
            <path d="m8.25 11.5 3.75 3.75 3.75-3.75" />
            <path d="M5 19.5h14" />
          </svg>
        </span>
        <span class="button-label">Export Scene</span>
      </button>
    </div>
  </div>
  <main>
    <section class="preview-shell">
      <div class="preview">
        <div class="preview-inner">
          <div id="previewHost"></div>
          <div class="preview-loading hidden" id="previewLoading">
            <div class="preview-loading-brand">
              <div class="preview-loading-title">mviewer</div>
              <div class="preview-loading-subtitle">github.com/majimboo</div>
            </div>
          </div>
          <div class="options-overlay hidden" id="sourceOverlay">
            <div class="options-dialog tools-dialog">
              <div class="export-eyebrow">Open .mview</div>
              <div class="export-title">Choose Source</div>
              <div class="options-section">
                <div class="options-heading">Local File</div>
                <div class="tool-input-row">
                  <input id="localPathInput" class="tool-input tool-input-clickable" type="text" placeholder="Click to choose a local .mview file" spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off" readonly onclick="browseLocalFile()" />
                  <button type="button" onclick="submitOpenLocal()">Open</button>
                </div>
              </div>
              <div class="options-section">
                <div class="options-heading">Load From URL</div>
                <div class="preview-body">
                  Paste a direct <code>.mview</code> URL, or a supported page URL that exposes a public <code>.mview</code> file.
                </div>
                <div class="tool-input-row">
                  <input id="downloadUrlInput" class="tool-input" type="text" inputmode="url" placeholder="https://www.artstation.com/artwork/3LBbA" spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off" />
                  <button class="primary" type="button" onclick="submitDownloadUrl()">Load URL</button>
                </div>
                <div class="tool-link-row">
                  <button class="tool-link-button" type="button" onclick="openArtstationModelFinder()">ArtStation model finder</button>
                </div>
                <datalist id="recentUrlList"></datalist>
              </div>
              <div class="options-footer">
                <button type="button" onclick="closeSourceDialog()">Cancel</button>
              </div>
            </div>
          </div>
          <div class="export-overlay hidden" id="exportOverlay">
            <div class="export-dialog">
              <div class="export-eyebrow">Exporting</div>
              <div class="export-title" id="exportStage">Preparing export...</div>
              <div class="export-progress-track">
                <div class="export-progress-bar" id="exportProgressBar"></div>
              </div>
              <div class="export-progress-meta">
                <span id="exportProgressText">0%</span>
                <span id="exportElapsed">Elapsed 0s</span>
                <span id="exportEta">ETA --</span>
              </div>
            </div>
          </div>
          <div class="options-overlay hidden" id="optionsOverlay">
            <div class="options-dialog">
              <div class="options-header">
                <div class="export-eyebrow">Export Options</div>
                <div class="export-title">Choose Export Settings</div>
              </div>
              <div class="options-scroll">
                <div class="options-grid">
                  <div class="options-section">
                    <div class="options-heading">Output Folder</div>
                    <div class="output-folder-row">
                      <div class="output-folder-path" id="outputDirText">Choose an output folder</div>
                      <div class="output-folder-actions">
                        <button type="button" onclick="post({cmd:'chooseOutputDir'})">Choose Folder</button>
                      </div>
                    </div>
                  </div>
                  <div class="options-section">
                    <div class="options-heading">Format</div>
                    <label class="check-row"><input type="radio" name="exportFormat" value="gltf" checked> <span>glTF</span></label>
                    <label class="check-row"><input type="radio" name="exportFormat" value="glb"> <span>GLB</span></label>
                    <label class="check-row"><input type="radio" name="exportFormat" value="obj"> <span>OBJ</span></label>
                  </div>
                  <div class="options-section" id="gltfOptions">
                    <div class="options-heading">glTF</div>
                    <label class="check-row"><input id="includeTextures" type="checkbox" checked> <span>Include textures</span></label>
                    <label class="check-row"><input id="includeAnimations" type="checkbox" checked> <span>Include animations</span></label>
                    <label class="check-row"><input id="includeCameras" type="checkbox" checked> <span>Include cameras</span></label>
                    <label class="check-row"><input id="includeLights" type="checkbox" checked> <span>Include lights</span></label>
                  </div>
                  <div class="options-section hidden" id="objOptions">
                    <div class="options-heading">OBJ</div>
                    <label class="check-row"><input id="includeTexturesObj" type="checkbox" checked> <span>Include textures</span></label>
                  </div>
                  <div class="options-section options-meshes">
                    <div class="options-heading">Meshes</div>
                    <div class="mesh-actions">
                      <button type="button" onclick="setAllMeshes(true)">All</button>
                      <button type="button" onclick="setAllMeshes(false)">None</button>
                    </div>
                    <div class="mesh-list" id="meshList"></div>
                  </div>
                </div>
              </div>
              <div class="options-footer">
                <button type="button" onclick="closeExportDialog()">Cancel</button>
                <button class="primary" type="button" onclick="submitExportDialog()">Export</button>
              </div>
            </div>
          </div>
          <div class="preview-copy" id="previewText">
            <div>
              <div class="preview-title">Embedded Preview Shell</div>
              <div class="preview-body">
                Open a scene to load the embedded Marmoset runtime preview.
              </div>
              <div class="preview-meta" id="previewMeta">No scene loaded.</div>
            </div>
          </div>
        </div>
      </div>
    </section>
  </main>
  <div class="status" id="status">Ready.</div>
  <script src="/marmoset.js"></script>
  <script src="/marmoset-fork.js"></script>
  <script>
    let runtimeViewer = null;
    let runtimeViewerSceneUrl = null;
    let currentArchiveBuffer = null;
    let latestState = null;
    let pendingExportSnapshot = null;

    function post(message) {
      window.ipc.postMessage(JSON.stringify(message));
    }

    function setStatus(message) {
      document.getElementById('status').textContent = message || 'Ready.';
    }

    function installArchiveCapture() {
      if (window.__mviewerArchiveCaptureInstalled || typeof Network === 'undefined' || !Network.fetchBinary) {
        return;
      }
      window.__mviewerArchiveCaptureInstalled = true;
      const originalFetchBinary = Network.fetchBinary.bind(Network);
      Network.fetchBinary = function(url, onload, onerror, onprogress) {
        return originalFetchBinary(
          url,
          function(buffer) {
            if (String(url).includes('/scene/current.mview')) {
              currentArchiveBuffer = buffer;
            }
            onload && onload(buffer);
          },
          onerror,
          onprogress,
        );
      };
    }

    function arrayBufferToBase64(buffer) {
      if (!buffer) return null;
      const bytes = new Uint8Array(buffer);
      const chunkSize = 0x8000;
      let binary = '';
      for (let offset = 0; offset < bytes.length; offset += chunkSize) {
        const chunk = bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length));
        binary += String.fromCharCode.apply(null, chunk);
      }
      return btoa(binary);
    }

    function collectRuntimeSnapshot() {
      if (runtimeViewer && window.mviewerMarmosetFork?.collectRuntimeSnapshot) {
        const snapshot = window.mviewerMarmosetFork.collectRuntimeSnapshot(runtimeViewer);
        if (snapshot) {
          snapshot.archiveBase64 = arrayBufferToBase64(currentArchiveBuffer);
          return snapshot;
        }
      }
      return null;
    }

    function renderEmptySceneState() {
      document.getElementById('previewMeta').textContent = 'No scene loaded.';
    }

    let exportTicker = null;

    function formatDuration(seconds) {
      if (seconds == null || !Number.isFinite(seconds) || seconds < 0) {
        return '--';
      }
      if (seconds < 60) {
        return `${Math.round(seconds)}s`;
      }
      const minutes = Math.floor(seconds / 60);
      const remain = Math.round(seconds % 60);
      return `${minutes}m ${remain}s`;
    }

    function updateExportTimer() {
      if (!latestState || !latestState.exporting || !latestState.export_started_at_ms) {
        return;
      }
      const elapsedSeconds = Math.max(0, (Date.now() - latestState.export_started_at_ms) / 1000);
      document.getElementById('exportElapsed').textContent = `Elapsed ${formatDuration(elapsedSeconds)}`;
      document.getElementById('exportEta').textContent = `ETA ${formatDuration(latestState.export_eta_seconds)}`;
    }

    function renderExportState(state) {
      latestState = state;
      const overlay = document.getElementById('exportOverlay');
      const button = document.getElementById('exportButton');
      button.disabled = !state.loaded || !!state.exporting;

      if (state.exporting) {
        overlay.classList.remove('hidden');
        document.getElementById('exportStage').textContent = state.export_stage || 'Exporting...';
        const progress = Math.max(0, Math.min(100, state.export_progress || 0));
        document.getElementById('exportProgressBar').style.width = `${progress}%`;
        document.getElementById('exportProgressText').textContent = `${progress}%`;
        updateExportTimer();
        if (!exportTicker) {
          exportTicker = window.setInterval(updateExportTimer, 1000);
        }
      } else {
        overlay.classList.add('hidden');
        if (exportTicker) {
          window.clearInterval(exportTicker);
          exportTicker = null;
        }
      }
    }

    function currentExportFormat() {
      return document.querySelector('input[name="exportFormat"]:checked')?.value || 'gltf';
    }

    function syncExportFormatPanels() {
      const format = currentExportFormat();
      document.getElementById('gltfOptions').classList.toggle('hidden', !(format === 'gltf' || format === 'glb'));
      document.getElementById('objOptions').classList.toggle('hidden', format !== 'obj');
    }

    function buildMeshList(snapshot, selectedIndices) {
      const container = document.getElementById('meshList');
      container.innerHTML = '';
      const meshes = snapshot?.meshes || [];
      meshes.forEach(mesh => {
        const row = document.createElement('label');
        row.className = 'check-row';
        const input = document.createElement('input');
        input.type = 'checkbox';
        input.dataset.meshIndex = String(mesh.index);
        input.checked = selectedIndices.includes(mesh.index);
        const text = document.createElement('span');
        text.textContent = `${mesh.name} (${mesh.vertexCount ?? 0}v)`;
        row.appendChild(input);
        row.appendChild(text);
        container.appendChild(row);
      });
    }

    function allMeshIndices(snapshot) {
      return (snapshot?.meshes || []).map(mesh => mesh.index);
    }

    function setAllMeshes(enabled) {
      document.querySelectorAll('#meshList input[type="checkbox"]').forEach(input => {
        input.checked = enabled;
      });
    }

    function openExportDialog() {
      let snapshot = null;
      try {
        snapshot = collectRuntimeSnapshot();
      } catch (error) {
        console.error('Failed to collect runtime snapshot', error);
        setStatus(`Runtime snapshot failed: ${error?.message || error}`);
        return;
      }
      if (!snapshot) {
        setStatus('Preview is not ready yet.');
        return;
      }
      pendingExportSnapshot = snapshot;
      const defaults = latestState?.export_defaults || {
        format: 'gltf',
        include_textures: true,
        include_animations: true,
        include_cameras: true,
        include_lights: true,
      };
      document.querySelector(`input[name="exportFormat"][value="${defaults.format || 'gltf'}"]`).checked = true;
      document.getElementById('includeTextures').checked = defaults.include_textures !== false;
      document.getElementById('includeTexturesObj').checked = defaults.include_textures !== false;
      document.getElementById('includeAnimations').checked = defaults.include_animations !== false;
      document.getElementById('includeCameras').checked = defaults.include_cameras !== false;
      document.getElementById('includeLights').checked = defaults.include_lights !== false;
      buildMeshList(snapshot, allMeshIndices(snapshot));
      syncExportFormatPanels();
      document.getElementById('optionsOverlay').classList.remove('hidden');
    }

    function closeExportDialog() {
      document.getElementById('optionsOverlay').classList.add('hidden');
    }

    function openSourceDialog() {
      document.getElementById('sourceOverlay').classList.remove('hidden');
    }

    function closeSourceDialog() {
      document.getElementById('sourceOverlay').classList.add('hidden');
    }

    function browseLocalFile() {
      post({ cmd: 'browseLocalFile' });
    }

    function submitOpenLocal() {
      const input = document.getElementById('localPathInput');
      const path = input.value.trim();
      if (!path) {
        setStatus('Choose or enter a local .mview file first.');
        return;
      }
      closeSourceDialog();
      post({ cmd: 'openRecentFile', path });
    }

    function submitDownloadUrl() {
      const input = document.getElementById('downloadUrlInput');
      const url = input.value.trim();
      if (!url) {
        setStatus('Enter a URL first.');
        return;
      }
      closeSourceDialog();
      post({ cmd: 'downloadFromUrl', url });
    }

    function openArtstationModelFinder() {
      post({
        cmd: 'openExternalUrl',
        url: 'https://www.google.com/search?q=site:artstation.com/embed/+mview&udm=2&source=univ'
      });
    }

    function renderRecentDatalist(id, items) {
      const list = document.getElementById(id);
      if (!list) return;
      list.innerHTML = '';
      (items || []).slice(0, 5).forEach(item => {
        const option = document.createElement('option');
        option.value = item;
        list.appendChild(option);
      });
    }

    function submitExportDialog() {
      if (!pendingExportSnapshot) {
        closeExportDialog();
        return;
      }
      const format = currentExportFormat();
      const selectedMeshes = Array.from(document.querySelectorAll('#meshList input[type="checkbox"]'))
        .filter(input => input.checked)
        .map(input => Number(input.dataset.meshIndex));
      const exportOptions = {
        format,
        includedMeshes: selectedMeshes,
        includeTextures: format === 'obj'
          ? document.getElementById('includeTexturesObj').checked
          : document.getElementById('includeTextures').checked,
        includeAnimations: document.getElementById('includeAnimations').checked,
        includeCameras: document.getElementById('includeCameras').checked,
        includeLights: document.getElementById('includeLights').checked,
      };
      closeExportDialog();
      post({ cmd: 'exportSceneWithRuntime', snapshot: pendingExportSnapshot, exportOptions });
    }

    function renderSceneState(scene) {
      const meta = scene.metaData || {};
      const animations = scene.sceneAnimator?.animations || [];
      const cameras = scene.cameras?.count ?? scene.cameras?.length ?? 0;
      const lights = scene.lights?.count ?? scene.lights?.length ?? 0;
      const meshes = scene.meshes?.length ?? 0;
      const materialsList = Array.isArray(scene.materialsList) ? scene.materialsList : [];
      document.getElementById('previewMeta').textContent =
        `${meta.title || '(untitled)'} | ${meshes} meshes | ${materialsList.length} materials | ${cameras} cameras | ${lights} lights | ${animations.length} animations`;
    }

    function watchSceneState() {
      const scene = runtimeViewer?.scene;
      if (!scene || !scene.sceneLoaded) {
        return;
      }
      renderSceneState(scene);
    }

    window.__mviewerReceive = function(state) {
      latestState = state;
      document.getElementById('inputPath').textContent = state.input_path || 'Open a .mview file';
      document.getElementById('localPathInput').value = state.local_path_draft || '';
      document.getElementById('outputDirText').textContent = state.output_dir || 'Choose an output folder';
      document.getElementById('status').textContent = state.status || 'Ready.';
      renderRecentDatalist('recentLocalList', state.recent_files);
      renderRecentDatalist('recentUrlList', state.recent_urls);
      renderExportState(state);

      const previewText = document.getElementById('previewText');
      if (state.loaded && state.scene_url) {
        previewText.classList.add('hidden');
        ensurePreview(state.scene_url);
      } else {
        previewText.classList.remove('hidden');
        document.getElementById('previewLoading').classList.add('hidden');
        runtimeViewerSceneUrl = null;
        runtimeViewer = null;
        renderEmptySceneState();
      }
    };

    function ensurePreview(sceneUrl) {
      const host = document.getElementById('previewHost');
      if (!window.marmoset || !host) {
        setStatus('Embedded Marmoset runtime not available.');
        return;
      }
      installArchiveCapture();
      if (runtimeViewer && runtimeViewerSceneUrl === sceneUrl) {
        resizePreview();
        return;
      }
      document.getElementById('previewLoading').classList.remove('hidden');
      host.innerHTML = '';
      currentArchiveBuffer = null;
      const width = Math.max(320, host.clientWidth || 960);
      const height = Math.max(180, host.clientHeight || 540);
      try {
        runtimeViewer = new marmoset.WebViewer(width, height, sceneUrl, false);
      } catch (error) {
        console.error('Failed to create WebViewer', error);
        setStatus(`Preview init failed: ${error?.message || error}`);
        return;
      }
      runtimeViewerSceneUrl = sceneUrl;
      runtimeViewer.domRoot.style.width = '100%';
      runtimeViewer.domRoot.style.height = '100%';
      host.appendChild(runtimeViewer.domRoot);
      runtimeViewer.onLoad = () => {
        document.getElementById('previewLoading').classList.add('hidden');
        setStatus('Preview loaded.');
        watchSceneState();
      };
      try {
        runtimeViewer.loadScene(sceneUrl);
        setStatus('Loading embedded preview...');
      } catch (error) {
        document.getElementById('previewLoading').classList.add('hidden');
        console.error('Failed to load preview scene', error);
        setStatus(`Preview load failed: ${error?.message || error}`);
      }
      resizePreview();
      renderEmptySceneState();
    }

    function resizePreview() {
      const host = document.getElementById('previewHost');
      if (!runtimeViewer || !host) {
        return;
      }
      const width = Math.max(320, host.clientWidth || 960);
      const height = Math.max(180, host.clientHeight || 540);
      runtimeViewer.resize(width, height);
    }

    window.addEventListener('resize', resizePreview);
    window.setInterval(watchSceneState, 250);
    window.addEventListener('change', event => {
      if (event.target && event.target.name === 'exportFormat') {
        syncExportFormatPanels();
      }
    });
    window.addEventListener('error', event => {
      console.error(event.error || event.message);
      setStatus(`UI error: ${event.message}`);
    });
    window.addEventListener('unhandledrejection', event => {
      console.error(event.reason);
      setStatus(`UI error: ${event.reason?.message || event.reason}`);
    });

    function escapeHtml(value) {
      return String(value)
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('\"', '&quot;');
    }

    post({ cmd: 'ready' });
    renderEmptySceneState();
  </script>
</body>
</html>
"#;

#[derive(Debug)]
struct AppState {
    input_path: Option<PathBuf>,
    local_path_draft: String,
    output_dir: String,
    status: String,
    base_url: String,
    scene_revision: u64,
    exporting: bool,
    export_progress: u8,
    export_stage: String,
    export_started_at_ms: Option<u64>,
    export_eta_seconds: Option<u64>,
    export_job_id: u64,
    settings: AppSettings,
    settings_path: PathBuf,
    js_export_scene: Option<JsExportScene>,
}

#[derive(Debug, Default)]
struct ProtocolState {
    current_scene_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum FrontendCommand {
    Ready,
    OpenProject,
    BrowseLocalFile,
    OpenRecentFile {
        path: String,
    },
    ChooseOutputDir,
    ExportScene,
    DownloadFromUrl {
        url: String,
    },
    OpenExternalUrl {
        url: String,
    },
    ExportSceneWithRuntime {
        snapshot: JsExportScene,
        #[serde(rename = "exportOptions")]
        export_options: GuiExportOptions,
    },
}

#[derive(Debug)]
enum UserEvent {
    Frontend(FrontendCommand),
    ExportProgress {
        job_id: u64,
        progress: u8,
        stage: String,
    },
    ExportFinished {
        job_id: u64,
        result: Result<crate::ExportReport, String>,
    },
    DownloadFinished {
        result: Result<PathBuf, String>,
    },
    ArtstationResolved {
        result: Result<String, String>,
    },
}

#[derive(Debug, Serialize)]
struct AppViewState {
    loaded: bool,
    input_path: Option<String>,
    local_path_draft: String,
    output_dir: String,
    recent_files: Vec<String>,
    recent_urls: Vec<String>,
    scene_url: Option<String>,
    status: String,
    exporting: bool,
    export_progress: u8,
    export_stage: String,
    export_started_at_ms: Option<u64>,
    export_eta_seconds: Option<u64>,
    export_defaults: ExportDefaultsView,
}

#[derive(Debug, Serialize)]
struct ExportDefaultsView {
    format: String,
    include_textures: bool,
    include_animations: bool,
    include_cameras: bool,
    include_lights: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AppSettings {
    recent_files: Vec<String>,
    recent_urls: Vec<String>,
    last_opened_file: Option<String>,
    last_output_dir: Option<String>,
    export_format: Option<String>,
    include_textures: Option<bool>,
    include_animations: Option<bool>,
    include_cameras: Option<bool>,
    include_lights: Option<bool>,
}

#[derive(Debug, Clone)]
struct AppPaths {
    settings_path: PathBuf,
    webview_data_dir: PathBuf,
}

struct HiddenResolverWindow {
    _window: tao::window::Window,
    _webview: wry::WebView,
}

pub fn run() -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let protocol_state = Arc::new(Mutex::new(ProtocolState::default()));
    let app_paths = app_paths()?;
    let base_url = start_http_server(protocol_state.clone())?;
    let mut web_context = WebContext::new(Some(app_paths.webview_data_dir.clone()));
    let mut hidden_web_context =
        WebContext::new(Some(app_paths.webview_data_dir.join("resolver")));
    let window = WindowBuilder::new()
        .with_title(&format!("mviewer {}", APP_VERSION))
        .with_inner_size(LogicalSize::new(1400.0, 900.0))
        .with_window_icon(Some(load_app_icon()?))
        .build(&event_loop)
        .context("failed to create application window")?;

    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_ipc_handler(move |request| {
            if let Ok(command) = serde_json::from_str::<FrontendCommand>(request.body()) {
                let _ = ipc_proxy.send_event(UserEvent::Frontend(command));
            }
        })
        .with_url(&format!("{base_url}/index.html"))
        .build(&window)
        .context("failed to build webview")?;

    let settings = load_settings(&app_paths.settings_path);
    let mut hidden_resolver: Option<HiddenResolverWindow> = None;
    let mut state = AppState {
        input_path: None,
        local_path_draft: String::new(),
        output_dir: settings.last_output_dir.clone().unwrap_or_default(),
        status: "Ready.".to_string(),
        base_url,
        scene_revision: 0,
        exporting: false,
        export_progress: 0,
        export_stage: String::new(),
        export_started_at_ms: None,
        export_eta_seconds: None,
        export_job_id: 0,
        settings,
        settings_path: app_paths.settings_path.clone(),
        js_export_scene: None,
    };
    restore_last_project(&mut state, &protocol_state);
    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Frontend(command)) => {
                handle_command(
                    command,
                    &mut state,
                    &protocol_state,
                    &proxy,
                    target,
                    &mut hidden_web_context,
                    &mut hidden_resolver,
                );
                if let Err(err) = push_state(&webview, &state) {
                    state.status = format!("UI sync failed: {err:#}");
                    let _ = push_state(&webview, &state);
                }
            }
            Event::UserEvent(UserEvent::ExportProgress {
                job_id,
                progress,
                stage,
            }) => {
                if job_id == state.export_job_id {
                    apply_export_progress(&mut state, progress, &stage);
                }
                if let Err(err) = push_state(&webview, &state) {
                    state.status = format!("UI sync failed: {err:#}");
                    let _ = push_state(&webview, &state);
                }
            }
            Event::UserEvent(UserEvent::ExportFinished { job_id, result }) => {
                if job_id == state.export_job_id {
                    finish_export(&mut state, result);
                }
                if let Err(err) = push_state(&webview, &state) {
                    state.status = format!("UI sync failed: {err:#}");
                    let _ = push_state(&webview, &state);
                }
            }
            Event::UserEvent(UserEvent::DownloadFinished { result }) => {
                match result {
                    Ok(path) => open_project_path(&mut state, &protocol_state, path),
                    Err(err) => state.status = format!("Download failed: {err}"),
                }
                if let Err(err) = push_state(&webview, &state) {
                    state.status = format!("UI sync failed: {err:#}");
                    let _ = push_state(&webview, &state);
                }
            }
            Event::UserEvent(UserEvent::ArtstationResolved { result }) => {
                hidden_resolver = None;
                match result {
                    Ok(url) => {
                        state.status = format!("Resolved ArtStation embed: {url}");
                        start_download_job(&mut state, &proxy, url);
                    }
                    Err(err) => state.status = format!("ArtStation resolve failed: {err}"),
                }
                if let Err(err) = push_state(&webview, &state) {
                    state.status = format!("UI sync failed: {err:#}");
                    let _ = push_state(&webview, &state);
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });

    #[allow(unreachable_code)]
    Ok(())
}

fn handle_command(
    command: FrontendCommand,
    state: &mut AppState,
    protocol_state: &Arc<Mutex<ProtocolState>>,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
    hidden_web_context: &mut WebContext,
    hidden_resolver: &mut Option<HiddenResolverWindow>,
) {
    match command {
        FrontendCommand::Ready => {}
        FrontendCommand::OpenProject => open_project(state, protocol_state),
        FrontendCommand::BrowseLocalFile => browse_local_file(state),
        FrontendCommand::OpenRecentFile { path } => open_project_path(state, protocol_state, PathBuf::from(path)),
        FrontendCommand::ChooseOutputDir => choose_output_dir(state),
        FrontendCommand::ExportScene => export_scene(state, proxy),
        FrontendCommand::DownloadFromUrl { url } => {
            if is_artstation_artwork_url(&url) {
                state.status = format!("Resolving ArtStation page {url}");
                match start_artstation_resolver(target, proxy, hidden_web_context, &url) {
                    Ok(resolver) => *hidden_resolver = Some(resolver),
                    Err(err) => state.status = format!("ArtStation resolve failed: {err:#}"),
                }
            } else {
                start_download_job(state, proxy, url);
            }
        }
        FrontendCommand::OpenExternalUrl { url } => {
            if let Err(err) = open_external_url(&url) {
                state.status = format!("Open browser failed: {err:#}");
            }
        }
        FrontendCommand::ExportSceneWithRuntime {
            snapshot,
            export_options,
        } => {
            remember_export_defaults(state, &export_options);
            state.js_export_scene = Some(snapshot);
            export_scene_with_options(state, proxy, export_options);
        }
    }
}

fn open_project(state: &mut AppState, protocol_state: &Arc<Mutex<ProtocolState>>) {
    let Some(path) = FileDialog::new()
        .add_filter("Marmoset Viewer scene", &["mview"])
        .pick_file()
    else {
        return;
    };

    open_project_path(state, protocol_state, path);
}

fn open_project_path(
    state: &mut AppState,
    protocol_state: &Arc<Mutex<ProtocolState>>,
    path: PathBuf,
) {
    state.input_path = Some(path.clone());
    state.local_path_draft = path.display().to_string();
    state.js_export_scene = None;
    state.scene_revision = state.scene_revision.saturating_add(1);
    if state.output_dir.trim().is_empty() {
        state.output_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .display()
            .to_string();
    }
    state.status = format!("Loaded {}", path.display());
    if let Ok(mut shared) = protocol_state.lock() {
        shared.current_scene_path = Some(path.clone());
    }
    remember_recent_file(&mut state.settings, &path);
    state.settings.last_opened_file = Some(path.display().to_string());
    state.settings.last_output_dir = Some(state.output_dir.clone());
    persist_settings(state);
}

fn export_scene(state: &mut AppState, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
    export_scene_with_options(state, proxy, current_export_defaults(state));
}

fn export_scene_with_options(
    state: &mut AppState,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    export_options: GuiExportOptions,
) {
    if state.exporting {
        return;
    }
    let Some(scene) = state.js_export_scene.clone() else {
        state.status = "Export failed: Preview is not ready yet. Wait for the Marmoset scene to finish loading.".to_string();
        return;
    };
    let Some(input_path) = state.input_path.as_deref() else {
        state.status = "Export failed: Load a .mview file first.".to_string();
        return;
    };
    let output_dir = resolve_export_dir(input_path, state.output_dir.trim());
    state.exporting = true;
    state.export_progress = 1;
    state.export_stage = "Preparing export".to_string();
    state.export_started_at_ms = Some(now_ms());
    state.export_eta_seconds = None;
    state.status = "Exporting...".to_string();
    state.export_job_id = state.export_job_id.saturating_add(1);

    let job_id = state.export_job_id;
    let input_path = input_path.to_path_buf();
    let output_dir = output_dir.clone();
    let scene = scene.clone();
    let export_options_clone = export_options.clone();
    let proxy = proxy.clone();
    thread::spawn(move || {
        let progress_proxy = proxy.clone();
        let result = export_from_js_scene_path_with_options_and_progress(
            &input_path,
            &output_dir,
            &scene,
            &export_options_clone,
            |progress, stage| {
            let _ = progress_proxy.send_event(UserEvent::ExportProgress {
                job_id,
                progress,
                stage: stage.to_string(),
            });
        },
        )
        .map_err(|err| format!("{err:#}"));
        let _ = proxy.send_event(UserEvent::ExportFinished { job_id, result });
    });
}

fn apply_export_progress(state: &mut AppState, progress: u8, stage: &str) {
    state.exporting = true;
    state.export_progress = progress.min(99);
    state.export_stage = stage.to_string();
    if let Some(started_at_ms) = state.export_started_at_ms {
        let elapsed_seconds = now_ms().saturating_sub(started_at_ms) / 1000;
        if progress > 0 {
            let eta = (elapsed_seconds * u64::from(100_u8.saturating_sub(progress))) / u64::from(progress);
            state.export_eta_seconds = Some(eta);
        }
    }
    state.status = format!("{stage} ({}%)", state.export_progress);
}

fn finish_export(state: &mut AppState, result: Result<crate::ExportReport, String>) {
    state.exporting = false;
    state.export_eta_seconds = Some(0);
    match result {
        Ok(report) => {
            state.export_progress = 100;
            state.export_stage = "Export complete".to_string();
            state.status = format!(
                "Exported {} of {} meshes to {}",
                report.exported_meshes,
                report.total_meshes,
                report.output_dir.display()
            );
        }
        Err(err) => {
            state.export_progress = 0;
            state.export_stage = "Export failed".to_string();
            state.status = format!("Export failed: {err}");
        }
    }
}

fn choose_output_dir(state: &mut AppState) {
    if let Some(path) = FileDialog::new().pick_folder() {
        state.output_dir = path.display().to_string();
        state.status = format!("Export parent folder set to {}", path.display());
        state.settings.last_output_dir = Some(state.output_dir.clone());
        persist_settings(state);
    }
}

fn browse_local_file(state: &mut AppState) {
    let Some(path) = FileDialog::new()
        .add_filter("Marmoset Viewer scene", &["mview"])
        .pick_file()
    else {
        return;
    };
    state.local_path_draft = path.display().to_string();
    state.status = format!("Selected {}", path.display());
}

fn start_download_job(
    state: &mut AppState,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    url: String,
) {
    state.status = format!("Resolving {url}");
    remember_recent_url(&mut state.settings, &url);
    persist_settings(state);
    let proxy = proxy.clone();
    thread::spawn(move || {
        let result = download_scene_from_url(&url).map_err(|err| format!("{err:#}"));
        let _ = proxy.send_event(UserEvent::DownloadFinished { result });
    });
}

fn push_state(webview: &wry::WebView, state: &AppState) -> Result<()> {
    let json = serde_json::to_string(&build_view_state(state)).context("failed to serialize UI state")?;
    webview
        .evaluate_script(&format!("window.__mviewerReceive({json});"))
        .context("failed to evaluate UI update script")?;
    Ok(())
}

fn build_view_state(state: &AppState) -> AppViewState {
    let (loaded, input_path) = if let Some(path) = &state.input_path {
        (
            true,
            Some(path.display().to_string()),
        )
    } else {
        (false, None)
    };

    AppViewState {
        loaded,
        input_path,
        local_path_draft: state.local_path_draft.clone(),
        output_dir: state.output_dir.clone(),
        recent_files: state.settings.recent_files.iter().take(5).cloned().collect(),
        recent_urls: state.settings.recent_urls.iter().take(5).cloned().collect(),
        scene_url: loaded.then_some(format!(
            "{}/scene/current.mview?v={}",
            state.base_url, state.scene_revision
        )),
        status: state.status.clone(),
        exporting: state.exporting,
        export_progress: state.export_progress,
        export_stage: state.export_stage.clone(),
        export_started_at_ms: state.export_started_at_ms,
        export_eta_seconds: state.export_eta_seconds,
        export_defaults: ExportDefaultsView {
            format: state
                .settings
                .export_format
                .clone()
                .unwrap_or_else(|| "gltf".to_string()),
            include_textures: state.settings.include_textures.unwrap_or(true),
            include_animations: state.settings.include_animations.unwrap_or(true),
            include_cameras: state.settings.include_cameras.unwrap_or(true),
            include_lights: state.settings.include_lights.unwrap_or(true),
        },
    }
}

fn resolve_export_dir(input_path: &Path, parent_dir: &str) -> PathBuf {
    let root = if parent_dir.trim().is_empty() {
        input_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        PathBuf::from(parent_dir)
    };
    let stem = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("scene");
    root.join(stem)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn current_export_defaults(state: &AppState) -> GuiExportOptions {
    GuiExportOptions {
        format: match state.settings.export_format.as_deref() {
            Some("glb") => GuiExportFormat::Glb,
            Some("obj") => GuiExportFormat::Obj,
            _ => GuiExportFormat::Gltf,
        },
        included_meshes: Vec::new(),
        include_textures: state.settings.include_textures.unwrap_or(true),
        include_animations: state.settings.include_animations.unwrap_or(true),
        include_cameras: state.settings.include_cameras.unwrap_or(true),
        include_lights: state.settings.include_lights.unwrap_or(true),
    }
}

fn remember_export_defaults(state: &mut AppState, export_options: &GuiExportOptions) {
    state.settings.export_format = Some(match export_options.format {
        GuiExportFormat::Gltf => "gltf".to_string(),
        GuiExportFormat::Glb => "glb".to_string(),
        GuiExportFormat::Obj => "obj".to_string(),
    });
    state.settings.include_textures = Some(export_options.include_textures);
    state.settings.include_animations = Some(export_options.include_animations);
    state.settings.include_cameras = Some(export_options.include_cameras);
    state.settings.include_lights = Some(export_options.include_lights);
    persist_settings(state);
}

fn app_paths() -> Result<AppPaths> {
    let dirs = ProjectDirs::from("com", "majidarif", "mviewer")
        .context("failed to resolve app data directories")?;
    let config_dir = dirs.config_dir();
    let data_local_dir = dirs.data_local_dir();
    std::fs::create_dir_all(config_dir).context("failed to create config directory")?;
    std::fs::create_dir_all(data_local_dir).context("failed to create local data directory")?;
    let webview_data_dir = data_local_dir.join("webview");
    std::fs::create_dir_all(&webview_data_dir).context("failed to create webview data directory")?;
    Ok(AppPaths {
        settings_path: config_dir.join("settings.json"),
        webview_data_dir,
    })
}

fn load_settings(settings_path: &Path) -> AppSettings {
    let mut settings = std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|json| serde_json::from_str::<AppSettings>(&json).ok())
        .unwrap_or_default();
    settings.recent_files.retain(|path| Path::new(path).exists());
    settings.recent_urls.truncate(10);
    settings
}

fn persist_settings(state: &AppState) {
    if let Ok(json) = serde_json::to_string_pretty(&state.settings) {
        let _ = std::fs::write(&state.settings_path, json);
    }
}

fn remember_recent_file(settings: &mut AppSettings, path: &Path) {
    let path_string = path.display().to_string();
    settings.recent_files.retain(|existing| existing != &path_string);
    settings.recent_files.insert(0, path_string);
    settings.recent_files.truncate(10);
}

fn remember_recent_url(settings: &mut AppSettings, url: &str) {
    settings.recent_urls.retain(|existing| existing != url);
    settings.recent_urls.insert(0, url.to_string());
    settings.recent_urls.truncate(10);
}

fn restore_last_project(state: &mut AppState, protocol_state: &Arc<Mutex<ProtocolState>>) {
    let Some(path) = state.settings.last_opened_file.as_ref().map(PathBuf::from) else {
        return;
    };
    if path.exists() {
        open_project_path(state, protocol_state, path);
    }
}

fn start_http_server(protocol_state: Arc<Mutex<ProtocolState>>) -> Result<String> {
    let port = reserve_local_port()?;
    let bind_addr = format!("127.0.0.1:{port}");
    let server = Server::http(&bind_addr)
        .map_err(|err| anyhow::anyhow!("failed to start local preview server: {err}"))?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let url: &str = request.url();
            let path = url.split('?').next().unwrap_or("/");
            let method = request.method().clone();
            let response = handle_http_request(method, path, &protocol_state);
            let _ = request.respond(response);
        }
    });
    Ok(format!("http://{bind_addr}"))
}

fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to reserve preview server port")?;
    let port = listener.local_addr().context("failed to inspect preview server port")?.port();
    drop(listener);
    Ok(port)
}

fn handle_http_request(
    method: Method,
    path: &str,
    protocol_state: &Arc<Mutex<ProtocolState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if method != Method::Get && method != Method::Head {
        return response_status(StatusCode(405), b"method not allowed".to_vec(), "text/plain; charset=utf-8");
    }

    match path {
        "/" | "/index.html" => response_bytes("text/html; charset=utf-8", APP_HTML.as_bytes().to_vec()),
        "/app.css" => response_bytes("text/css; charset=utf-8", include_bytes!("app.css").to_vec()),
        "/favicon.ico" => response_bytes("image/x-icon", Vec::new()),
        "/marmoset.js" => response_bytes(
            "text/javascript; charset=utf-8",
            include_bytes!("../../docs/reverse-engineering/marmoset-d3f745560e47d383adc4f6a322092030.js").to_vec(),
        ),
        "/marmoset-fork.js" => response_bytes(
            "text/javascript; charset=utf-8",
            include_bytes!("marmoset-fork.js").to_vec(),
        ),
        "/scene/current.mview" => {
            let current = protocol_state
                .lock()
                .ok()
                .and_then(|state| state.current_scene_path.clone());
            match current.and_then(|scene_path| std::fs::read(scene_path).ok()) {
                Some(bytes) => response_bytes("application/octet-stream", bytes),
                None => response_status(StatusCode(404), b"scene not loaded".to_vec(), "text/plain; charset=utf-8"),
            }
        }
        _ => response_status(StatusCode(404), b"not found".to_vec(), "text/plain; charset=utf-8"),
    }
}

fn response_bytes(
    content_type: &'static str,
    bytes: Vec<u8>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_data(bytes);
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()) {
        response.add_header(header);
    }
    if let Ok(header) = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]) {
        response.add_header(header);
    }
    response
}

fn response_status(
    status: StatusCode,
    bytes: Vec<u8>,
    content_type: &'static str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_data(bytes).with_status_code(status);
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()) {
        response.add_header(header);
    }
    response
}


#[allow(dead_code)]
fn sanitize_preview_path(path: &Path) -> String {
    path.display().to_string()
}

fn open_external_url(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("cmd")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/C", "start", "", url])
            .spawn()
            .context("failed to launch default browser")?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .context("failed to launch default browser")?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .context("failed to launch default browser")?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", all(unix, not(target_os = "macos")))))]
    {
        anyhow::bail!("opening external URLs is not supported on this platform")
    }
}

fn is_artstation_artwork_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("artstation.com/artwork/")
}

fn start_artstation_resolver(
    target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    web_context: &mut WebContext,
    url: &str,
) -> Result<HiddenResolverWindow> {
    const SCRIPT: &str = r#"
        (() => {
          const send = (payload) => window.ipc.postMessage(JSON.stringify(payload));
          const absolutize = (value) => {
            try { return new URL(value, window.location.href).toString(); } catch (_) { return value; }
          };
          const findEmbed = () => {
            const html = document.documentElement?.innerHTML || '';
            const patterns = [
              /https?:\/\/www\.artstation\.com\/embed\/\d+/i,
              /https?:\/\/artstation\.com\/embed\/\d+/i,
              /\/embed\/\d+/i,
            ];
            for (const pattern of patterns) {
              const match = html.match(pattern);
              if (match && match[0]) return absolutize(match[0]);
            }
            const nodes = document.querySelectorAll('[src],[href],[content]');
            for (const node of nodes) {
              for (const attr of ['src', 'href', 'content']) {
                const value = node.getAttribute && node.getAttribute(attr);
                if (!value) continue;
                if (/artstation\.com\/embed\/\d+/i.test(value) || /\/embed\/\d+/i.test(value)) {
                  return absolutize(value);
                }
              }
            }
            return null;
          };
          let attempts = 0;
          const tick = () => {
            const embed = findEmbed();
            if (embed) {
              send({ kind: 'artstationResolverResult', embedUrl: embed });
              return;
            }
            attempts += 1;
            if (attempts > 60) {
              send({ kind: 'artstationResolverResult', error: 'Could not find ArtStation embed URL on the rendered page' });
              return;
            }
            setTimeout(tick, 250);
          };
          if (document.readyState === 'complete' || document.readyState === 'interactive') {
            tick();
          } else {
            window.addEventListener('DOMContentLoaded', tick, { once: true });
          }
        })();
    "#;

    let window = WindowBuilder::new()
        .with_visible(false)
        .with_title("mviewer-artstation-resolver")
        .build(target)
        .context("failed to create hidden ArtStation resolver window")?;
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_initialization_script(SCRIPT)
        .with_ipc_handler(move |request| {
            let body = request.body();
            let value: serde_json::Value = match serde_json::from_str(body) {
                Ok(value) => value,
                Err(_) => return,
            };
            if value.get("kind").and_then(|v| v.as_str()) != Some("artstationResolverResult") {
                return;
            }
            let result = if let Some(url) = value.get("embedUrl").and_then(|v| v.as_str()) {
                Ok(url.to_string())
            } else {
                Err(value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown ArtStation resolver failure")
                    .to_string())
            };
            let _ = ipc_proxy.send_event(UserEvent::ArtstationResolved { result });
        })
        .with_url(url)
        .build(&window)
        .context("failed to build hidden ArtStation resolver webview")?;
    Ok(HiddenResolverWindow {
        _window: window,
        _webview: webview,
    })
}

fn download_scene_from_url(page_or_file_url: &str) -> Result<PathBuf> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("mviewer/2.1.0")
        .build()
        .context("failed to create HTTP client")?;
    let scene_url = resolve_mview_url(&client, page_or_file_url)?;
    let filename = file_name_from_url(&scene_url).unwrap_or_else(|| "scene.mview".to_string());
    let download_dir = UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .context("failed to resolve download directory")?;
    std::fs::create_dir_all(&download_dir).context("failed to create download directory")?;
    let output_path = unique_output_path(download_dir.join(filename));
    let bytes = client
        .get(scene_url.clone())
        .send()
        .with_context(|| format!("failed to download {}", scene_url))?
        .error_for_status()
        .with_context(|| format!("download failed for {}", scene_url))?
        .bytes()
        .context("failed to read download body")?;
    std::fs::write(&output_path, &bytes).with_context(|| format!("failed to write {}", output_path.display()))?;
    Ok(output_path)
}

fn resolve_mview_url(client: &reqwest::blocking::Client, page_or_file_url: &str) -> Result<reqwest::Url> {
    let initial = reqwest::Url::parse(page_or_file_url).context("invalid URL")?;
    if initial.path().to_ascii_lowercase().ends_with(".mview") {
        return Ok(initial);
    }
    let html = client
        .get(initial.clone())
        .send()
        .with_context(|| format!("failed to fetch {}", initial))?
        .error_for_status()
        .with_context(|| format!("page request failed for {}", initial))?
        .text()
        .context("failed to read page body")?;

    if let Some(url) = find_first_mview_url(&html, &initial) {
        return Ok(url);
    }

    for embed_url in extract_artstation_embed_urls(&html, &initial) {
        let Ok(response) = client.get(embed_url.clone()).send() else {
            continue;
        };
        let Ok(response) = response.error_for_status() else {
            continue;
        };
        let Ok(body) = response.text() else {
            continue;
        };
        if let Some(url) = find_first_mview_url(&body, &embed_url) {
            return Ok(url);
        }
    }

    for candidate in extract_http_urls(&normalize_embedded_text(&html)) {
        if !looks_like_embedded_viewer_url(&candidate) {
            continue;
        }
        let Ok(candidate_url) = initial.join(&candidate).or_else(|_| reqwest::Url::parse(&candidate)) else {
            continue;
        };
        let Ok(response) = client.get(candidate_url.clone()).send() else {
            continue;
        };
        let Ok(response) = response.error_for_status() else {
            continue;
        };
        let Ok(body) = response.text() else {
            continue;
        };
        if let Some(url) = find_first_mview_url(&body, &candidate_url) {
            return Ok(url);
        }
    }

    anyhow::bail!("could not find a public .mview URL on that page")
}

fn extract_artstation_embed_urls(body: &str, base_url: &reqwest::Url) -> Vec<reqwest::Url> {
    let normalized = normalize_embedded_text(body);
    let mut urls = Vec::new();
    for candidate in extract_embed_like_urls(&normalized) {
        let lower = candidate.to_ascii_lowercase();
        if !(lower.contains("artstation.com/embed/") || lower.starts_with("/embed/")) {
            continue;
        }
        if let Ok(url) = base_url.join(&candidate).or_else(|_| reqwest::Url::parse(&candidate)) {
            urls.push(url);
        }
    }
    urls
}

fn find_first_mview_url(body: &str, base_url: &reqwest::Url) -> Option<reqwest::Url> {
    let normalized = normalize_embedded_text(body);
    extract_mview_like_urls(&normalized)
        .into_iter()
        .find_map(|candidate| base_url.join(&candidate).or_else(|_| reqwest::Url::parse(&candidate)).ok())
}

fn normalize_embedded_text(text: &str) -> String {
    text.replace("\\u002F", "/")
        .replace(r"\/", "/")
        .replace("&amp;", "&")
}

fn extract_mview_like_urls(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut search_start = 0usize;
    while let Some(relative_end) = lower[search_start..].find(".mview") {
        let end = search_start + relative_end + ".mview".len();
        let mut start = search_start + relative_end;
        while start > 0 && !is_url_delimiter(bytes[start - 1] as char) {
            start -= 1;
        }
        let mut final_end = end;
        while final_end < bytes.len() && !is_url_delimiter(bytes[final_end] as char) {
            final_end += 1;
        }
        let candidate = text[start..final_end]
            .trim_matches(&['"', '\'', '`'][..])
            .to_string();
        if !candidate.is_empty() {
            candidates.push(candidate);
        }
        search_start = end;
    }
    candidates
}

fn extract_http_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < text.len() {
        let slice = &text[index..];
        let relative = slice.find("https://").or_else(|| slice.find("http://"));
        let Some(relative) = relative else {
            break;
        };
        let start = index + relative;
        let mut end = start;
        while end < bytes.len() && !is_url_delimiter(bytes[end] as char) {
            end += 1;
        }
        urls.push(
            text[start..end]
                .trim_matches(&['"', '\'', '`'][..])
                .to_string(),
        );
        index = end;
    }
    urls
}

fn extract_embed_like_urls(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut search_start = 0usize;
    while let Some(relative_start) = lower[search_start..].find("/embed/") {
        let start = search_start + relative_start;
        let mut actual_start = start;
        while actual_start > 0 && !is_url_delimiter(bytes[actual_start - 1] as char) {
            actual_start -= 1;
        }
        let mut end = start;
        while end < bytes.len() && !is_url_delimiter(bytes[end] as char) {
            end += 1;
        }
        let candidate = text[actual_start..end]
            .trim_matches(&['"', '\'', '`'][..])
            .to_string();
        if !candidate.is_empty() {
            candidates.push(candidate);
        }
        search_start = end;
    }
    candidates
}

fn looks_like_embedded_viewer_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("marmoset") || lower.contains("viewer") || lower.contains("iframe")
}

fn is_url_delimiter(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(ch, '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '{' | '}' | '[' | ']')
}

fn file_name_from_url(url: &reqwest::Url) -> Option<String> {
    url.path_segments()
        .and_then(|segments| segments.last())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

fn unique_output_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("scene");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("mview");
    for index in 2..1000 {
        let candidate = parent.join(format!("{stem}-{index}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}
