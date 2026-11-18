//! Adaptador no oficial de X-Minus; contrato basado en HAR de noviembre de 2026.
//! Los secretos quedan en memoria de Rust. Nunca deben incorporarse a código ni logs.

use futures_util::StreamExt;
use regex::Regex;
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, COOKIE, LOCATION, ORIGIN, REFERER},
    multipart::{Form, Part},
    Client, Url,
};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::{Path, PathBuf}, time::Duration};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{webview::PageLoadEvent, AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex};

const PAGE: &str = "https://x-minus.pro/ai";
const API: &str = "https://mmd.uvronline.vip";

pub struct AppState {
    http: Client,
    current: Mutex<Context>,
}

#[derive(Default)]
struct Context {
    session: Option<Session>,
    job: Option<Job>,
}

#[derive(Clone)]
struct Session {
    auth_key: String,
    version: String,
    client_fp: String,
    show_setting_format: String,
    is_premium: bool,
    is_tester: bool,
    is_sng_country: bool,
}

#[derive(Clone)]
struct Job {
    id: String,
    stems: Vec<String>,
    source_filename: String,
}

// Catálogo derivado del JS público capturado: la misma definición alimenta Vue y Rust.
#[derive(Deserialize)]
struct Catalog {
    modes: Vec<CatalogMode>,
}

#[derive(Deserialize)]
struct CatalogMode {
    id: String,
    separation_type: String,
    models: Vec<CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    id: String,
    backend: String,
    stems: Vec<String>,
    access_level: u8,
}

fn model_config(preset_id: &str, model_id: &str) -> Result<(String, String, Vec<String>, u8), String> {
    let catalog: Catalog = serde_json::from_str(include_str!("../../src/catalog.json"))
        .map_err(|_| "Catálogo de modelos no válido")?;
    let preset = catalog.modes.iter().find(|m| m.id == preset_id)
        .ok_or("Tipo de separación desconocido")?;
    let model = preset.models.iter().find(|m| m.id == model_id)
        .ok_or("Modelo no disponible para esta separación")?;
    Ok((preset.separation_type.clone(), model.backend.clone(), model.stems.clone(), model.access_level))
}

fn access_allowed(session: &Session, level: u8) -> bool {
    // Misma regla `xa` del JS ai.v105. El servidor puede aplicar más restricciones.
    level == 0 || level == 3 || session.is_premium || session.is_tester ||
        (level == 1 && !session.is_sng_country)
}

#[derive(Serialize)]
pub struct SessionInfo {
    version: String,
    /// true si la página se abrió con una cookie (manual o embebida) y era válida.
    using_account: bool,
    /// true si había cookie incrustada pero el servicio ya no la acepta.
    account_expired: bool,
    is_premium: bool,
    is_tester: bool,
    is_sng_country: bool,
}

#[derive(Serialize)]
pub struct JobInfo {
    job_id: String,
    stems: Vec<String>,
    source_filename: String,
    status: String,
    remaining_time: Option<i64>,
    requests_left: Option<i64>,
    reused_previous_result: bool,
}

#[derive(Serialize)]
pub struct StatusInfo {
    status: String,
    remaining_time: Option<i64>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct StemResult {
    stem: String,
    path: Option<String>,
    error: Option<String>,
}

/// Cookie opcional incrustada al compilar (ver `secrets/README.md`).
/// Va ofuscada con XOR para que no aparezca en claro en el binario.
fn embedded_cookie() -> Option<String> {
    const KEY: &[u8] = b"xminus-stems-embedded";
    let hex = option_env!("XMINUS_EMBEDDED_COOKIE")?;
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).enumerate()
        .map(|(i, at)| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok()
            .map(|b| b ^ KEY[i % KEY.len()]))
        .collect();
    String::from_utf8(bytes?).ok().filter(|c| !c.is_empty())
}

fn user_agent() -> &'static str {
    option_env!("XMINUS_EMBEDDED_USER_AGENT").unwrap_or("XMinusStems/0.4")
}

impl AppState {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(user_agent())
            .timeout(Duration::from_secs(180))
            // No permitir redirecciones automáticas a dominios desconocidos.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("No se pudo crear el cliente HTTP");
        Self { http, current: Mutex::new(Context::default()) }
    }
}

fn page_setting(html: &str, key: &str) -> Option<String> {
    let pat = format!(r#"['\"]{}['\"]\s*:\s*['\"]([^'\"]+)['\"]"#, regex::escape(key));
    Regex::new(&pat).ok()?.captures(html).map(|x| x[1].to_owned())
}

fn input_value(page: &Html, id: &str) -> Option<String> {
    let selector = Selector::parse(&format!("input#{id}")).ok()?;
    page.select(&selector).next()?.value().attr("value").map(str::to_owned)
}

fn read_session(html: &str) -> Result<Session, String> {
    let page = Html::parse_document(html);
    let auth_key = input_value(&page, "vocal-cut-auth-key")
        .filter(|v| !v.is_empty())
        .ok_or("El servicio ha cambiado o no permite conectarse ahora mismo. Inténtalo más tarde.")?;
    let version = page_setting(html, "version").ok_or("El servicio ha cambiado y la app no lo reconoce. Puede que necesite una actualización.")?;
    let client_fp = input_value(&page, "vocal-cut-fp").unwrap_or_else(|| "-".into());
    let show_setting_format = page_setting(html, "show_setting_format").unwrap_or_else(|| "0".into());
    Ok(Session {
        auth_key, version, client_fp, show_setting_format,
        is_premium: page_bool(html, "is_premium"),
        is_tester: page_bool(html, "is_tester"),
        is_sng_country: page_bool(html, "is_sng_country"),
    })
}

fn page_bool(html: &str, key: &str) -> bool {
    let pat = format!(r#"['\"]{}['\"]\s*:\s*(true|false|[01])"#, regex::escape(key));
    Regex::new(&pat).ok().and_then(|pat| pat.captures(html))
        .map(|captures| &captures[1] == "1" || &captures[1] == "true")
        .unwrap_or(false)
}

fn request_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ORIGIN, HeaderValue::from_static("https://x-minus.pro"));
    h.insert(REFERER, HeaderValue::from_static(PAGE));
    h.insert(ACCEPT, HeaderValue::from_static("application/json, text/plain, */*"));
    h
}

async fn body_as_json(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let text = resp.text().await.map_err(|_| "Se cortó la conexión con el servicio".to_string())?;
    if !status.is_success() {
        // Evitar mostrar HTML o secretos del proveedor en la interfaz.
        return Err(format!("El servicio respondió con un error ({}). Inténtalo de nuevo en unos minutos.", status.as_u16()));
    }
    serde_json::from_str(&text)
        .map_err(|_| "El servicio respondió de forma inesperada; puede haber cambiado.".into())
}

fn json_string<'a>(json: &'a Value, field: &str) -> Option<&'a str> {
    json.get(field).and_then(Value::as_str)
}

const LOGIN_LABEL: &str = "login";
static WATCH_ID: AtomicU64 = AtomicU64::new(0);

// Barra de progreso fina arriba, como la de un navegador. Arranca al navegar
// (clic, formulario, cierre de página) y también al cargar la página nueva;
// avanza sola y se completa en el evento `load`.
const LOADING_OVERLAY: &str = r#"(function(){
  if (window.top !== window) return;
  var bar, timer, pct = 0;
  function make(){
    if (bar && bar.isConnected) return;
    bar = document.createElement('div');
    bar.style.cssText = 'position:fixed;top:0;left:0;height:3px;width:0;z-index:2147483647;pointer-events:none;background:linear-gradient(90deg,#7c5cff,#ff5c8a);box-shadow:0 0 8px #7c5cff;transition:width .3s ease,opacity .3s ease;opacity:1';
    (document.documentElement || document).appendChild(bar);
  }
  function start(){
    clearInterval(timer); make(); pct = 6; bar.style.opacity = '1'; bar.style.width = pct + '%';
    timer = setInterval(function(){ pct += (92 - pct) * 0.08; bar.style.width = pct + '%'; }, 250);
  }
  function done(){
    clearInterval(timer); if (!bar) return;
    bar.style.width = '100%';
    setTimeout(function(){ bar.style.opacity = '0'; setTimeout(function(){ bar && bar.remove(); }, 350); }, 250);
  }
  start();
  window.addEventListener('load', done);
  document.addEventListener('readystatechange', function(){ if (document.readyState === 'complete') done(); });
  document.addEventListener('click', function(e){
    var a = e.target && e.target.closest && e.target.closest('a[href]');
    if (a && a.href && a.target !== '_blank' && a.href.indexOf('javascript:') !== 0 && a.href.split('#')[0] !== location.href.split('#')[0]) start();
  }, true);
  document.addEventListener('submit', start, true);
  window.addEventListener('beforeunload', start);
  setTimeout(done, 30000);
})();"#;

fn cookie_header(win: &WebviewWindow) -> String {
    Url::parse("https://x-minus.pro/").ok()
        .and_then(|u| win.cookies_for_url(u).ok())
        .map(|cs| {
            let mut v: Vec<String> = cs.iter().map(|c| format!("{}={}", c.name(), c.value())).collect();
            v.sort();
            v.join("; ")
        })
        .unwrap_or_default()
}

/// Vigila en segundo plano la ventana de login y avisa a la interfaz cuando
/// el usuario termina de entrar (volviendo a x-minus.pro con cookies nuevas).
fn watch_login(app: AppHandle) {
    let id = WATCH_ID.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        let baseline = app.get_webview_window(LOGIN_LABEL).map(|w| cookie_header(&w)).unwrap_or_default();
        let baseline_names: Vec<String> = baseline.split("; ")
            .filter_map(|c| c.split('=').next()).map(str::to_owned).collect();
        let mut left_site = false;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            if WATCH_ID.load(Ordering::SeqCst) != id { return; }
            let Some(win) = app.get_webview_window(LOGIN_LABEL) else {
                let _ = app.emit("login-closed", ());
                return;
            };
            if !win.is_visible().unwrap_or(false) { return; }
            let on_site = win.url().ok().and_then(|u| u.host_str().map(|h| h == "x-minus.pro")).unwrap_or(false);
            if !on_site { left_site = true; continue; }
            let now = cookie_header(&win);
            let new_names = now.split("; ").filter_map(|c| c.split('=').next())
                .any(|n| !n.is_empty() && !baseline_names.iter().any(|b| b == n));
            if (left_site && now != baseline) || new_names {
                let _ = win.hide();
                let _ = app.emit("login-detected", ());
                return;
            }
        }
    });
}

/// Ventana con la web de X-Minus donde el usuario inicia sesión él mismo
/// (también con Google). Sus cookies viven en el almacén del propio webview.
fn login_window(app: &AppHandle, visible: bool) -> Result<WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(LOGIN_LABEL) {
        return Ok(win);
    }
    let url = PAGE.parse().map_err(|_| "URL no válida".to_string())?;
    WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Iniciar sesión")
        .inner_size(480.0, 740.0)
        .visible(visible)
        .on_navigation(|u| u.scheme() == "https")
        .initialization_script(LOADING_OVERLAY)
        .on_page_load(|w, p| {
            // Solo informa a la interfaz mientras la ventana de login está a la vista.
            if w.is_visible().unwrap_or(false) {
                let _ = w.app_handle().emit("login-loading", matches!(p.event(), PageLoadEvent::Started));
            }
        })
        .build()
        .map_err(|_| "No se pudo abrir la ventana de inicio de sesión".to_string())
}

/// Cookies de x-minus.pro guardadas por la ventana de login (si hay).
fn login_cookie(app: &AppHandle) -> Option<String> {
    let win = login_window(app, false).ok()?;
    let cookies = win.cookies_for_url(Url::parse("https://x-minus.pro/").ok()?).ok()?;
    let header = cookies.iter().map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>().join("; ");
    (!header.is_empty() && !header.contains(['\r', '\n'])).then_some(header)
}

#[tauri::command]
pub async fn open_login(app: AppHandle) -> Result<(), String> {
    let win = login_window(&app, true)?;
    win.show().and_then(|_| win.set_focus()).map_err(|_| "No se pudo mostrar la ventana".to_string())?;
    watch_login(app);
    Ok(())
}

#[tauri::command]
pub async fn close_login(app: AppHandle) -> Result<(), String> {
    WATCH_ID.fetch_add(1, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(LOGIN_LABEL) { let _ = win.hide(); }
    Ok(())
}

/// Borra las cookies de x-minus.pro del almacén de la app.
#[tauri::command]
pub async fn logout(app: AppHandle) -> Result<(), String> {
    let win = login_window(&app, false)?;
    let cookies = win.cookies_for_url(Url::parse("https://x-minus.pro/").map_err(|e| e.to_string())?)
        .map_err(|_| "No se pudieron leer las cookies".to_string())?;
    for c in cookies { let _ = win.delete_cookie(c); }
    WATCH_ID.fetch_add(1, Ordering::SeqCst);
    let _ = win.hide();
    Ok(())
}

fn is_x_minus_host(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https" && (host == "x-minus.pro" || host.ends_with(".x-minus.pro"))
}

/// Dominios a los que puede redirigir la página. Solo `x-minus.pro` recibe la cookie.
fn trusted_page_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    is_x_minus_host(url) || (url.scheme() == "https" && ["uvronline.app", "uvronline.vip"]
        .iter().any(|d| host == *d || host.ends_with(&format!(".{d}"))))
}

async fn fetch_session(client: &Client, cookie: Option<&str>) -> Result<Session, String> {
    // Las redirecciones están desactivadas en el cliente: se siguen a mano y solo
    // hacia x-minus.pro, que es el único dominio al que se envía la cookie.
    const MAX_REDIRECTS: usize = 5;
    let mut url = Url::parse(PAGE).map_err(|e| e.to_string())?;
    for _ in 0..=MAX_REDIRECTS {
        let mut req = client.get(url.clone()).header(ACCEPT, "text/html");
        if let Some(cookie) = cookie.filter(|_| is_x_minus_host(&url)) {
            req = req.header(COOKIE, cookie);
        }
        let resp = req.send().await
            .map_err(|_| "No se pudo conectar. Comprueba tu conexión a internet.".to_string())?;
        if resp.status().is_redirection() {
            let location = resp.headers().get(LOCATION).and_then(|v| v.to_str().ok())
                .ok_or("El servicio redirigió sin indicar adónde.")?;
            let next = url.join(location).map_err(|_| "Redirección no válida".to_string())?;
            if !trusted_page_url(&next) {
                return Err(format!("El servicio redirigió a un sitio no permitido ({}).",
                    next.host_str().unwrap_or("desconocido")));
            }
            url = next;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("El servicio no está disponible ahora mismo (HTTP {}).", resp.status().as_u16()));
        }
        let final_url = url.clone();
        let html = resp.text().await.map_err(|e| e.to_string())?;
        return read_session(&html).map_err(|e| {
            // Diagnóstico sin secretos: dónde se aterrizó y qué página era.
            let title = Selector::parse("title").ok()
                .and_then(|sel| Html::parse_document(&html).select(&sel).next()
                    .map(|t| t.text().collect::<String>().trim().chars().take(60).collect::<String>()))
                .unwrap_or_default();
            format!("{e} [{}{} · «{}» · {} bytes · cookie {}]",
                final_url.host_str().unwrap_or("?"), final_url.path(), title, html.len(),
                if cookie.is_some() { "enviada" } else { "no enviada" })
        });
    }
    Err("Demasiadas redirecciones al abrir el servicio.".into())
}

#[tauri::command]
pub async fn initialize_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_cookie: Option<String>,
) -> Result<SessionInfo, String> {
    let manual = session_cookie.map(|c| c.trim().trim_start_matches("Cookie:").trim().to_owned())
        .filter(|c| !c.is_empty());
    if manual.as_deref().is_some_and(|c| c.contains(['\r', '\n'])) {
        return Err("La cookie no es válida".into());
    }
    let cookie = manual.clone().or_else(|| login_cookie(&app)).or_else(embedded_cookie);
    let (session, using_account, account_expired) =
        match fetch_session(&state.http, cookie.as_deref()).await {
            Ok(session) => (session, cookie.is_some(), false),
            // Si la cookie embebida caducó, seguir en modo gratuito sin ella.
            Err(_) if manual.is_none() && cookie.is_some() =>
                (fetch_session(&state.http, None).await?, false, true),
            Err(e) => return Err(e),
        };
    let info = SessionInfo {
        version: session.version.clone(),
        using_account,
        account_expired,
        is_premium: session.is_premium,
        is_tester: session.is_tester,
        is_sng_country: session.is_sng_country,
    };
    let mut ctx = state.current.lock().await;
    ctx.session = Some(session);
    ctx.job = None;
    Ok(info)
}

/// Muestra en el explorador de archivos un MP3 que acaba de guardar la app.
#[tauri::command]
pub async fn reveal_file(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let is_mp3 = path.extension().and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    if !is_mp3 || !path.is_absolute() || !path.is_file() {
        return Err("El archivo ya no existe".into());
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg("/select,").arg(&path).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(Path::new("/"))).spawn();
    result.map(|_| ()).map_err(|_| "No se pudo abrir la carpeta".into())
}

fn file_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

#[tauri::command]
pub async fn start_separation(
    state: State<'_, AppState>,
    file_path: String,
    preset_id: String,
    model_id: String,
) -> Result<JobInfo, String> {
    let session = state.current.lock().await.session.clone()
        .ok_or("Todavía no se ha conectado con el servicio. Espera un momento y vuelve a intentarlo.")?;
    let (separation_type, separation, stems, access_level) = model_config(&preset_id, &model_id)?;
    if !access_allowed(&session, access_level) {
        return Err("Esta opción necesita una cuenta Premium de X-Minus. Elige otra opción.".into());
    }
    let path = PathBuf::from(&file_path);
    let name = path.file_name().and_then(|x| x.to_str())
        .ok_or("El archivo debe tener un nombre válido")?.to_owned();
    let metadata = fs::metadata(&path).await.map_err(|_| "No se puede abrir el archivo. ¿Lo has movido o borrado?".to_string())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("El archivo está vacío o no es un audio".into());
    }
    let bytes = fs::read(&path).await.map_err(|_| "No se pudo leer el archivo de audio".to_string())?;
    let file_part = Part::bytes(bytes)
        .file_name(name)
        .mime_str(file_mime(&path))
        .map_err(|e| e.to_string())?;
    let multipart = Form::new()
        .text("auth_key", session.auth_key)
        .text("locale", "es_ES")
        .text("separation", separation)
        .text("separation_type", separation_type)
        .text("format", "mp3")
        .text("version", session.version)
        .text("model", model_id)
        .text("aggressiveness", "2")
        .text("lvpanning", "center")
        .text("uvrbve_ct", "auto")
        .text("pre_rate", "100")
        .text("bve_preproc", "auto")
        .text("show_setting_format", session.show_setting_format)
        .text("hostname", "x-minus.pro")
        .text("client_fp", session.client_fp)
        .part("myfile", file_part);

    let json = body_as_json(state.http
        .post(format!("{API}/upload/vocalCutAi?catch-file"))
        .headers(request_headers())
        .multipart(multipart)
        .send().await.map_err(|_| "No se pudo subir la canción. Comprueba tu conexión.".to_string())?
    ).await?;

    let status = json_string(&json, "status").unwrap_or("unknown");
    if status != "accepted" && status != "done" {
        let error = json_string(&json, "err_msg")
            .or_else(|| json_string(&json, "message"))
            .unwrap_or("El servicio no ha aceptado la canción");
        return Err(format!("{}", error.chars().take(300).collect::<String>()));
    }
    let job_id = json_string(&json, "job_id").ok_or("El servicio respondió de forma inesperada. Inténtalo de nuevo.")?.to_owned();
    let source_filename = json_string(&json, "source_filename").unwrap_or("audio").to_owned();
    let job = Job { id: job_id.clone(), stems: stems.clone(), source_filename: source_filename.clone() };
    state.current.lock().await.job = Some(job);
    Ok(JobInfo {
        job_id,
        stems,
        source_filename,
        status: status.to_owned(),
        remaining_time: json.get("remaining_time").and_then(Value::as_i64),
        requests_left: json.get("requests_left").and_then(Value::as_i64),
        reused_previous_result: json_string(&json, "similar_job_id").is_some(),
    })
}

#[tauri::command]
pub async fn check_status(state: State<'_, AppState>) -> Result<StatusInfo, String> {
    let ctx = state.current.lock().await;
    let session = ctx.session.clone().ok_or("No hay conexión con el servicio")?;
    let job = ctx.job.clone().ok_or("Todavía no se ha subido ninguna canción")?;
    drop(ctx);

    let form = Form::new()
        .text("job_id", job.id)
        .text("auth_key", session.auth_key)
        .text("locale", "es_ES");
    let json = body_as_json(state.http
        .post(format!("{API}/upload/vocalCutAi?check-job-status"))
        .headers(request_headers())
        .multipart(form)
        .send().await.map_err(|_| "Se perdió la conexión mientras se procesaba".to_string())?
    ).await?;

    Ok(StatusInfo {
        status: json_string(&json, "status").unwrap_or("unknown").to_owned(),
        remaining_time: json.get("remaining_time").and_then(Value::as_i64),
        error: json_string(&json, "err_msg").filter(|x| !x.is_empty())
            .map(|x| x.chars().take(300).collect()),
    })
}

fn trusted_download_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https" &&
        (host == "mmd.uvronline.vip" || host == "x-minus.pro" ||
         host.ends_with(".uvronline.app") || host == "uvronline.app")
}

fn safe_filename(raw: &str) -> String {
    let cleaned: String = raw.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == ' ')
        .take(72)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() { "audio".into() } else { cleaned.into() }
}

fn stem_filename(stem: &str) -> String {
    let name = match stem {
        "vocal" => "voces", "inst" => "instrumental", "drums" => "bateria", "bass" => "bajo",
        "guitar" => "guitarra", "piano" => "piano", "toms" => "toms", "cymbals" => "platos",
        "snare" => "caja", "kick" => "bombo", "wind" => "viento", "strings" => "cuerdas",
        "acguitar" => "guitarra_acustica", "lv" => "voz_principal", "bv" => "coros",
        "male" => "voz_masculina", "female" => "voz_femenina", "enhanced" => "mejorado",
        "similarity" => "centro", "inst2" => "instrumental_2", "vocal2" => "voces_2",
        _ => stem,
    };
    safe_filename(name)
}

// X-Minus can redirect either directly to /separated/*.mp3 (vocal/inst)
// or via /encode -> /separated/*.mp3 (drums/bass). Resolve both absolute
// and relative Location headers against the URL which returned them.
fn resolve_download_redirect(current: &Url, location: &str) -> Result<Url, String> {
    let next = current.join(location).map_err(|_| "Location de descarga no válido")?;
    if !trusted_download_url(&next) {
        return Err("Descarga bloqueada: redirección hacia un destino no permitido".into());
    }
    Ok(next)
}

// reqwest has redirects disabled on purpose so every hop can be validated.
async fn get_download_response(client: &Client, start: Url) -> Result<reqwest::Response, String> {
    const MAX_REDIRECTS: usize = 6;
    let mut current = start;
    for hops in 0..=MAX_REDIRECTS {
        if !trusted_download_url(&current) {
            return Err("Descarga bloqueada: dominio o protocolo no permitido".into());
        }
        let resp = client.get(current.clone())
            .header(REFERER, PAGE)
            .send().await.map_err(|_| "No se pudo descargar. Comprueba tu conexión.".to_string())?;
        if resp.status().is_redirection() {
            if hops == MAX_REDIRECTS {
                return Err(format!("Demasiadas redirecciones en la descarga (máximo {MAX_REDIRECTS})"));
            }
            let location = resp.headers().get(LOCATION)
                .ok_or("El servidor redirigió sin cabecera Location")?
                .to_str().map_err(|_| "Location de descarga no es texto válido")?;
            current = resolve_download_redirect(&current, location)?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("No se pudo descargar (error {}). Puede que la pista haya caducado.", resp.status().as_u16()));
        }
        return Ok(resp);
    }
    Err("No se pudo resolver la descarga".into())
}

async fn download_one(state: &AppState, job: &Job, dir: &Path, stem: &str) -> Result<PathBuf, String> {
    // La web usa job_id original incluso si el backend devuelve similar_job_id.
    let mut url = Url::parse(&format!("{API}/dl/vocalCutAi")).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("job-id", &job.id)
        .append_pair("stem", stem)
        .append_pair("fmt", "mp3")
        .append_pair("cdn", "0");

    let resp = get_download_response(&state.http, url).await?;
    // Impide guardar una página de error HTML como MP3.
    if let Some(content_type) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
        if content_type.to_str().unwrap_or("").contains("text/html") {
            return Err("El servicio no devolvió un audio. Puede que la pista haya caducado.".into());
        }
    }

    fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    let name = format!("{}_{}.mp3", safe_filename(&job.source_filename), stem_filename(stem));
    let mut destination = dir.join(&name);
    let mut part_path = dir.join(format!(".{name}.part"));
    for attempt in 1..=100 {
        if !fs::try_exists(&destination).await.map_err(|e| e.to_string())? &&
           !fs::try_exists(&part_path).await.map_err(|e| e.to_string())? {
            break;
        }
        if attempt == 100 { return Err("Demasiados archivos con el mismo nombre".into()); }
        let new_name = format!("{}_{}_{}.mp3", safe_filename(&job.source_filename), stem_filename(stem), attempt + 1);
        destination = dir.join(&new_name);
        part_path = dir.join(format!(".{new_name}.part"));
    }
    let mut output = fs::OpenOptions::new().write(true).create_new(true)
        .open(&part_path).await.map_err(|_| "No se puede guardar en esa carpeta".to_string())?;
    let mut stream = resp.bytes_stream();
    let result: Result<(), String> = async {
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            output.write_all(&bytes).await.map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
        }
        output.flush().await.map_err(|e| e.to_string())?;
        if total < 128 { return Err("Descarga incompleta o vacía".into()); }
        Ok(())
    }.await;
    drop(output);
    if let Err(err) = result {
        let _ = fs::remove_file(&part_path).await;
        return Err(err);
    }
    fs::rename(&part_path, &destination).await.map_err(|e| e.to_string())?;
    Ok(destination)
}

/// Recupera una separación ya procesada sin consumir otra solicitud de subida.
/// El estado se verifica posteriormente con check_status y la sesión actual.
#[tauri::command]
pub async fn restore_job(
    state: State<'_, AppState>,
    job_id: String,
    source_filename: String,
    preset_id: String,
    model_id: String,
) -> Result<JobInfo, String> {
    let session = state.current.lock().await.session.clone()
        .ok_or("Todavía no se ha conectado con el servicio")?;
    let (_, _, stems, access_level) = model_config(&preset_id, &model_id)?;
    if !access_allowed(&session, access_level) {
        return Err("Esta opción necesita una cuenta Premium de X-Minus".into());
    }
    let job_id = job_id.trim().to_ascii_lowercase();
    if !(40..=64).contains(&job_id.len()) || !job_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("El código no es válido. Cópialo completo, sin espacios.".into());
    }
    let filename = safe_filename(&source_filename);
    let job = Job { id: job_id.clone(), stems: stems.clone(), source_filename: filename.clone() };
    state.current.lock().await.job = Some(job);
    Ok(JobInfo {
        job_id,
        stems,
        source_filename: filename,
        status: "restored".into(),
        remaining_time: None,
        requests_left: None,
        reused_previous_result: false,
    })
}

#[tauri::command]
pub async fn download_stems(
    state: State<'_, AppState>,
    directory: String,
    requested_stems: Option<Vec<String>>,
) -> Result<Vec<StemResult>, String> {
    let job = state.current.lock().await.job.clone().ok_or("No hay ninguna canción separada")?;
    let path = PathBuf::from(directory);
    if !path.is_dir() { return Err("Elige una carpeta que exista".into()); }
    let selected = requested_stems.unwrap_or_else(|| job.stems.clone());
    if selected.is_empty() || selected.iter().any(|s| !job.stems.contains(s)) {
        return Err("Las pistas pedidas no corresponden a esta canción".into());
    }
    let mut results = Vec::new();
    for stem in &selected {
        match download_one(&state, &job, &path, stem).await {
            Ok(filename) => results.push(StemResult {
                stem: stem.into(), path: Some(filename.to_string_lossy().to_string()), error: None,
            }),
            Err(error) => results.push(StemResult { stem: stem.into(), path: None, error: Some(error) }),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_session_from_html_without_leaking_secret() {
        let html = r#"<input id="vocal-cut-auth-key" value="secret-x" />
            <input id="vocal-cut-fp" value="-" /><script>
            var aiInitData={'version':'3-4-0','show_setting_format':'0'};
            </script>"#;
        let session = read_session(html).unwrap();
        assert_eq!(session.auth_key, "secret-x");
        assert_eq!(session.version, "3-4-0");
    }

    #[test]
    fn embedded_cookie_is_absent_or_non_empty() {
        assert!(embedded_cookie().map_or(true, |c| !c.is_empty() && !c.contains('\n')));
    }

    #[test]
    fn page_redirects_only_to_x_minus() {
        assert!(trusted_page_url(&Url::parse("https://x-minus.pro/ai?lang=es").unwrap()));
        assert!(!trusted_page_url(&Url::parse("http://x-minus.pro/ai").unwrap()));
        assert!(!trusted_page_url(&Url::parse("https://x-minus.pro.evil.com/ai").unwrap()));
        assert!(trusted_page_url(&Url::parse("https://uvronline.app/ai").unwrap()));
        assert!(!trusted_page_url(&Url::parse("https://uvronline.app.evil.com/ai").unwrap()));
        assert!(!is_x_minus_host(&Url::parse("https://uvronline.app/ai").unwrap()));
    }

    #[test]
    fn only_known_cdn_allowed() {
        assert!(trusted_download_url(&Url::parse("https://a43.uvronline.app/separated/a.mp3").unwrap()));
        assert!(!trusted_download_url(&Url::parse("https://uvronline.app.evil.com/x").unwrap()));
        assert!(!trusted_download_url(&Url::parse("http://a43.uvronline.app/x").unwrap()));
    }

    #[test]
    fn resolve_two_hops_and_relative_location() {
        let initial = Url::parse("https://mmd.uvronline.vip/dl/vocalCutAi?job-id=abc&stem=drums&fmt=mp3&cdn=0").unwrap();
        let encode = resolve_download_redirect(&initial,
            "https://a43.uvronline.app/encode?job-id=abc&stem=Drums&fmt=mp3").unwrap();
        assert_eq!(encode.path(), "/encode");
        let audio = resolve_download_redirect(&encode,
            "/separated/abc_Drums.mp3?fn=some%20file.mp3").unwrap();
        assert_eq!(audio.path(), "/separated/abc_Drums.mp3");
        assert_eq!(audio.host_str(), Some("a43.uvronline.app"));
        assert!(resolve_download_redirect(&audio, "https://attacker.example/file.mp3").is_err());
    }

    #[test]
    fn catalog_maps_models_to_correct_download_stems() {
        let (kind, backend, stems, _) = model_config("splitter_4", "htdemucs").unwrap();
        assert_eq!((kind.as_str(), backend.as_str()), ("splitter", "bdov"));
        assert_eq!(stems, ["vocal", "inst", "drums", "bass"]);
        let (kind, backend, stems, _) = model_config("splitter_6", "bs_roformer_sw_bdovgp").unwrap();
        assert_eq!((kind.as_str(), backend.as_str()), ("splitter", "bdovgp"));
        assert_eq!(stems.len(), 6);
        let (_, backend, stems, _) = model_config("splitter_4", "htdemucs_drumsep").unwrap();
        assert_eq!(backend, "ckst");
        assert_eq!(stems, ["toms", "cymbals", "snare", "kick"]);
        let (_, backend, stems, _) = model_config("vocals_music", "bs_roformer_hyperace").unwrap();
        assert_eq!(backend, "inst_vocal");
        assert_eq!(stems, ["vocal", "inst"]);
    }

    #[test]
    fn rejects_model_mismatched_to_category() {
        assert!(model_config("splitter_4", "mel_roformer_deux_becruily").is_err());
    }
}
