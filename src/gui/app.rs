use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use base64::Engine as _;
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
    ExportOptions, ProjectDocument, TextureExportFormat, default_output_dir, export_project,
    export_texture_asset, load_project,
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
      <button class="primary" id="exportButton" onclick="post({cmd:'exportScene'})" disabled>Export Scene</button>
      <select id="textureFormat" onchange="post({cmd:'setTextureFormat', format: this.value})">
        <option value="png">PNG</option>
        <option value="tga">TGA</option>
      </select>
    </div>
  </div>
  <main>
    <aside>
      <div class="card">
        <h3>Project</h3>
        <div class="summary-list" id="summary">
          <div class="empty">No scene loaded.</div>
        </div>
      </div>
      <div class="card">
        <h3>Output</h3>
        <div class="simple-list">
          <div id="outputDir">No output folder selected.</div>
        </div>
      </div>
      <div class="card">
        <h3>Recent Files</h3>
        <div class="simple-list" id="recentFiles">
          <div class="empty">No recent files yet.</div>
        </div>
      </div>
      <div class="card">
        <h3>Notes</h3>
        <div class="simple-list">
          <div>Rust stays responsible for export and filesystem work.</div>
          <div>The preview pane is now designed around embedded web content for exact Marmoset runtime parity.</div>
        </div>
      </div>
    </aside>
    <section class="content">
      <div class="preview">
        <div class="preview-inner">
          <div>
            <div id="previewHost"></div>
            <div class="preview-copy" id="previewText">
              <div>
                <div class="preview-title">Embedded Preview Shell</div>
                <div class="preview-body">
                  Open a scene to load the embedded Marmoset runtime preview.
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div class="panels">
        <div class="card">
          <h2>Materials</h2>
          <div id="materials" class="materials">
            <div class="empty">Open a scene to inspect materials and textures.</div>
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

    function post(message) {
      window.ipc.postMessage(JSON.stringify(message));
    }

    function textureFormat() {
      return document.getElementById('textureFormat').value;
    }

    function setStatus(message) {
      document.getElementById('status').textContent = message || 'Ready.';
    }

    function exportTexture(name) {
      post({ cmd: 'exportTexture', texture_name: name, format: textureFormat() });
    }

    function openRecent(path) {
      post({ cmd: 'openRecentProject', path });
    }

    function encodeTextureName(name) {
      const utf8 = new TextEncoder().encode(name);
      let binary = '';
      for (const byte of utf8) binary += String.fromCharCode(byte);
      return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
    }

    function textureUrl(name) {
      return `/texture/${encodeTextureName(name)}`;
    }

    function materialTextures(desc) {
      const slots = [
        ['albedo', desc.albedoTex],
        ['alpha', desc.alphaTex],
        ['normal', desc.normalTex],
        ['reflectivity', desc.reflectivityTex],
        ['gloss', desc.glossTex],
        ['extras', desc.extrasTex],
        ['extrasA', desc.extrasTexA],
        ['occlusion', desc.occlusionTex],
        ['emissive', desc.emissiveTex],
      ];
      return slots
        .filter(([, name]) => typeof name === 'string' && name.length)
        .map(([slot, name]) => ({ slot, name }));
    }

    function renderEmptySceneState() {
      document.getElementById('summary').innerHTML = '<div class="empty">No scene loaded.</div>';
      document.getElementById('materials').innerHTML = '<div class="empty">Open a scene to inspect materials and textures.</div>';
    }

    function renderRecentFiles(files) {
      const root = document.getElementById('recentFiles');
      if (!files || !files.length) {
        root.innerHTML = '<div class="empty">No recent files yet.</div>';
        return;
      }
      root.innerHTML = files.map(file => `
        <button class="recent-file" title="${escapeHtml(file)}" onclick="openRecent(${JSON.stringify(file)})">
          ${escapeHtml(file)}
        </button>
      `).join('');
    }

    function renderSceneState(scene) {
      const summary = document.getElementById('summary');
      const meta = scene.metaData || {};
      const animations = scene.sceneAnimator?.animations || [];
      const cameras = scene.cameras?.count ?? scene.cameras?.length ?? 0;
      const lights = scene.lights?.count ?? scene.lights?.length ?? 0;
      const meshes = scene.meshes?.length ?? 0;
      const materialsList = Array.isArray(scene.materialsList) ? scene.materialsList : [];

      summary.innerHTML = `
        <div>Title: ${escapeHtml(meta.title || '(untitled)')}</div>
        <div>Author: ${escapeHtml(meta.author || '(unknown)')}</div>
        <div>Meshes: ${meshes}</div>
        <div>Materials: ${materialsList.length}</div>
        <div>Cameras: ${cameras}</div>
        <div>Lights: ${lights}</div>
        <div>Animations: ${animations.length}</div>
      `;

      const materials = document.getElementById('materials');
      if (!materialsList.length) {
        materials.innerHTML = '<div class="empty">No materials detected in the loaded scene.</div>';
        return;
      }

      materials.innerHTML = materialsList.map((material, index) => {
        const desc = material?.desc || {};
        const name = desc.name || `Material ${index + 1}`;
        const textures = materialTextures(desc);
        return `
          <div class="material-card">
            <div class="material-header">
              <div class="material-name">${escapeHtml(name)}</div>
              <div class="texture-name">${textures.length} texture${textures.length === 1 ? '' : 's'}</div>
            </div>
            <div class="texture-strip">
              ${textures.length ? textures.map(texture => `
                <div class="texture-card">
                  <div class="texture-preview">
                    <img src="${textureUrl(texture.name)}" alt="${escapeHtml(texture.name)}" />
                  </div>
                  <div class="texture-slot">${escapeHtml(texture.slot)}</div>
                  <div class="texture-name">${escapeHtml(texture.name)}</div>
                  <button onclick="exportTexture(${JSON.stringify(texture.name)})">Export</button>
                </div>
              `).join('') : '<div class="empty">No bound textures</div>'}
            </div>
          </div>
        `;
      }).join('');
    }

    function watchSceneState() {
      const scene = runtimeViewer?.scene;
      if (!scene || !scene.sceneLoaded) {
        return;
      }
      renderSceneState(scene);
    }

    window.__mviewerReceive = function(state) {
      document.getElementById('inputPath').textContent = state.input_path || 'Open a .mview file';
      document.getElementById('outputDir').textContent = state.output_dir || 'No output folder selected.';
      document.getElementById('status').textContent = state.status || 'Ready.';
      document.getElementById('exportButton').disabled = !state.loaded;
      document.getElementById('textureFormat').value = state.texture_format || 'png';
      renderRecentFiles(state.recent_files || []);

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
      if (runtimeViewer && runtimeViewerSceneUrl === sceneUrl) {
        resizePreview();
        return;
      }
      host.innerHTML = '';
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
    project: Option<ProjectDocument>,
    output_dir: String,
    status: String,
    base_url: String,
    settings: AppSettings,
    settings_path: PathBuf,
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
    OpenRecentProject {
        path: String,
    },
    ChooseOutputDir,
    SetTextureFormat {
        format: String,
    },
    ExportScene,
    ExportTexture {
        texture_name: String,
        format: String,
    },
}

#[derive(Debug)]
enum UserEvent {
    Frontend(FrontendCommand),
}

#[derive(Debug, Serialize)]
struct AppViewState {
    loaded: bool,
    input_path: Option<String>,
    scene_url: Option<String>,
    output_dir: String,
    status: String,
    texture_format: String,
    recent_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AppSettings {
    recent_files: Vec<String>,
    last_opened_file: Option<String>,
    last_output_dir: Option<String>,
    texture_format: String,
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
        project: None,
        output_dir: settings.last_output_dir.clone().unwrap_or_default(),
        status: "Ready.".to_string(),
        base_url,
        settings,
        settings_path: app_paths.settings_path.clone(),
    };
    restore_last_project(&mut state, &protocol_state);
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Frontend(command)) => {
                handle_command(command, &mut state, &protocol_state);
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
) {
    match command {
        FrontendCommand::Ready => {}
        FrontendCommand::OpenProject => open_project(state, protocol_state),
        FrontendCommand::OpenRecentProject { path } => {
            open_project_path(state, protocol_state, PathBuf::from(path))
        }
        FrontendCommand::ChooseOutputDir => choose_output_dir(state),
        FrontendCommand::SetTextureFormat { format } => set_texture_format(state, &format),
        FrontendCommand::ExportScene => export_scene(state),
        FrontendCommand::ExportTexture {
            texture_name,
            format,
        } => export_texture(state, &texture_name, &format),
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
    match load_project(&path) {
        Ok(project) => {
            if state.output_dir.trim().is_empty() {
                state.output_dir = default_output_dir(&path).display().to_string();
            }
            state.status = format!("Loaded {}", path.display());
            state.project = Some(project);
            if let Ok(mut shared) = protocol_state.lock() {
                shared.current_scene_path = Some(path.clone());
            }
            remember_recent_file(&mut state.settings, &path);
            state.settings.last_opened_file = Some(path.display().to_string());
            state.settings.last_output_dir = Some(state.output_dir.clone());
            persist_settings(state);
        }
        Err(err) => {
            state.status = format!("Load failed: {err:#}");
        }
    }
}

fn choose_output_dir(state: &mut AppState) {
    if let Some(path) = FileDialog::new().pick_folder() {
        state.output_dir = path.display().to_string();
        state.status = format!("Output folder set to {}", path.display());
        state.settings.last_output_dir = Some(state.output_dir.clone());
        persist_settings(state);
    }
}

fn set_texture_format(state: &mut AppState, format: &str) {
    state.settings.texture_format = match format.to_ascii_lowercase().as_str() {
        "tga" => "tga".to_string(),
        _ => "png".to_string(),
    };
    persist_settings(state);
}

fn export_scene(state: &mut AppState) {
    let Some(project) = &state.project else {
        state.status = "Load a .mview file first.".to_string();
        return;
    };
    let output_dir = PathBuf::from(state.output_dir.trim());
    match export_project(project, &output_dir, &ExportOptions::include_all(&project.scene)) {
        Ok(report) => {
            state.status = format!(
                "Exported {} of {} meshes to {}",
                report.exported_meshes,
                report.total_meshes,
                report.output_dir.display()
            );
        }
        Err(err) => {
            state.status = format!("Export failed: {err:#}");
        }
    }
}

fn export_texture(state: &mut AppState, texture_name: &str, format: &str) {
    let Some(project) = &state.project else {
        state.status = "Load a .mview file first.".to_string();
        return;
    };
    let output_dir = PathBuf::from(state.output_dir.trim()).join("textures");
    let format = match format.to_ascii_lowercase().as_str() {
        "tga" => TextureExportFormat::Tga,
        _ => TextureExportFormat::Png,
    };
    state.settings.texture_format = match format {
        TextureExportFormat::Png => "png".to_string(),
        TextureExportFormat::Tga => "tga".to_string(),
    };
    persist_settings(state);
    match export_texture_asset(project, texture_name, &output_dir, format) {
        Ok(path) => {
            state.status = format!("Exported texture to {}", path.display());
        }
        Err(err) => {
            state.status = format!("Texture export failed: {err:#}");
        }
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
    let (loaded, input_path) = if let Some(project) = &state.project {
        (
            true,
            Some(project.input_path.display().to_string()),
        )
    } else {
        (false, None)
    };

    AppViewState {
        loaded,
        input_path,
        scene_url: loaded.then_some(format!("{}/scene/current.mview", state.base_url)),
        output_dir: state.output_dir.clone(),
        status: state.status.clone(),
        texture_format: if state.settings.texture_format.is_empty() {
            "png".to_string()
        } else {
            state.settings.texture_format.clone()
        },
        recent_files: state.settings.recent_files.clone(),
    }
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
    if settings.texture_format.is_empty() {
        settings.texture_format = "png".to_string();
    }
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
        _ if path.starts_with("/texture/") => {
            let encoded = path.trim_start_matches("/texture/");
            let texture_name = match decode_texture_route(encoded) {
                Some(name) => name,
                None => return response_status(StatusCode(400), b"invalid texture route".to_vec(), "text/plain; charset=utf-8"),
            };
            let current = protocol_state
                .lock()
                .ok()
                .and_then(|state| state.current_scene_path.clone());
            let Some(scene_path) = current else {
                return response_status(StatusCode(404), b"scene not loaded".to_vec(), "text/plain; charset=utf-8");
            };
            match load_project(&scene_path)
                .ok()
                .and_then(|project| project.archive.get(&texture_name).map(|entry| (entry.data.clone(), texture_name)))
            {
                Some((bytes, name)) => response_bytes(texture_content_type(&name), bytes),
                None => response_status(StatusCode(404), b"texture not found".to_vec(), "text/plain; charset=utf-8"),
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

fn decode_texture_route(encoded: &str) -> Option<String> {
    let padded = match encoded.len() % 4 {
        0 => encoded.to_string(),
        2 => format!("{encoded}=="),
        3 => format!("{encoded}="),
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(padded.as_bytes())
        .ok()?;
    String::from_utf8(bytes).ok()
}

fn texture_content_type(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("tga") => "image/x-tga",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

#[allow(dead_code)]
fn sanitize_preview_path(path: &Path) -> String {
    path.display().to_string()
}
