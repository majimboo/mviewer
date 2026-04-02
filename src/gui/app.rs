use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::ProjectDirs;
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
    js_export::{JsExportScene, export_from_js_scene_path_with_progress},
};

use super::icon::load_app_icon;

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
      <div class="title">mviewer</div>
      <div class="path" id="inputPath">Open a .mview file</div>
      <button onclick="post({cmd:'openProject'})">Open .mview</button>
      <button onclick="post({cmd:'chooseOutputDir'})">Choose Output Folder</button>
      <button class="primary" id="exportButton" onclick="triggerExportScene()" disabled>Export Scene</button>
    </div>
  </div>
  <main>
    <section class="preview-shell">
      <div class="preview">
        <div class="preview-inner">
          <div id="previewHost"></div>
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

    function triggerExportScene() {
      if (runtimeViewer && window.mviewerMarmosetFork?.collectRuntimeSnapshot) {
        try {
          const snapshot = window.mviewerMarmosetFork.collectRuntimeSnapshot(runtimeViewer);
          if (snapshot) {
            snapshot.archiveBase64 = arrayBufferToBase64(currentArchiveBuffer);
            post({ cmd: 'exportSceneWithRuntime', snapshot });
            return;
          }
        } catch (error) {
          console.error('Failed to collect runtime snapshot', error);
          setStatus(`Runtime snapshot failed: ${error?.message || error}`);
        }
      }
      post({ cmd: 'exportScene' });
    }

    function renderEmptySceneState() {
      document.getElementById('previewMeta').textContent = 'No scene loaded.';
    }

    let exportTicker = null;
    let latestState = null;

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
      document.getElementById('status').textContent = state.status || 'Ready.';
      renderExportState(state);

      const previewText = document.getElementById('previewText');
      if (state.loaded && state.scene_url) {
        previewText.classList.add('hidden');
        ensurePreview(state.scene_url);
      } else {
        previewText.classList.remove('hidden');
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
        setStatus('Preview loaded.');
        watchSceneState();
      };
      try {
        runtimeViewer.loadScene(sceneUrl);
        setStatus('Loading embedded preview...');
      } catch (error) {
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
    ChooseOutputDir,
    ExportScene,
    ExportSceneWithRuntime {
        snapshot: JsExportScene,
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
}

#[derive(Debug, Serialize)]
struct AppViewState {
    loaded: bool,
    input_path: Option<String>,
    scene_url: Option<String>,
    status: String,
    exporting: bool,
    export_progress: u8,
    export_stage: String,
    export_started_at_ms: Option<u64>,
    export_eta_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AppSettings {
    recent_files: Vec<String>,
    last_opened_file: Option<String>,
    last_output_dir: Option<String>,
}

#[derive(Debug, Clone)]
struct AppPaths {
    settings_path: PathBuf,
    webview_data_dir: PathBuf,
}

pub fn run() -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let protocol_state = Arc::new(Mutex::new(ProtocolState::default()));
    let app_paths = app_paths()?;
    let base_url = start_http_server(protocol_state.clone())?;
    let mut web_context = WebContext::new(Some(app_paths.webview_data_dir.clone()));
    let window = WindowBuilder::new()
        .with_title("mviewer")
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
    let mut state = AppState {
        input_path: None,
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
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Frontend(command)) => {
                handle_command(command, &mut state, &protocol_state, &proxy);
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
) {
    match command {
        FrontendCommand::Ready => {}
        FrontendCommand::OpenProject => open_project(state, protocol_state),
        FrontendCommand::ChooseOutputDir => choose_output_dir(state),
        FrontendCommand::ExportScene => export_scene(state, proxy),
        FrontendCommand::ExportSceneWithRuntime { snapshot } => {
            state.js_export_scene = Some(snapshot);
            export_scene(state, proxy);
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
    let proxy = proxy.clone();
    thread::spawn(move || {
        let progress_proxy = proxy.clone();
        let result = export_from_js_scene_path_with_progress(&input_path, &output_dir, &scene, |progress, stage| {
            let _ = progress_proxy.send_event(UserEvent::ExportProgress {
                job_id,
                progress,
                stage: stage.to_string(),
            });
        })
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
