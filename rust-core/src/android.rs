// SPDX-License-Identifier: GPL-3.0-or-later
//! Android glue: JNI entry points, ANativeWindow → Vulkan surface, persistence.
//! Everything is called on the main thread except `kbModel` (background model loading).

use crate::engine::Model;
use crate::gfx::{self, Canvas, Gpu};
use crate::keyboard::{Cmd, Editor, Keyboard};
use crate::prefs::{Prefs, Theme};
use crate::ui::{fonts, kbview, settings::SettingsApp, settings::SettingsCmd, theme};
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use raw_window_handle::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

#[link(name = "android")]
extern "C" {}

static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);
static KB: Mutex<Option<KbHost>> = Mutex::new(None);
/// The keyboard's Vulkan window. It has its own lock so that presenting a frame (which can
/// wait for the display) never blocks touch handling, which only needs `KB`.
static KWIN: Mutex<Option<Win>> = Mutex::new(None);
static ST: Mutex<Option<StHost>> = Mutex::new(None);

fn init_log() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        android_logger::init_once(android_logger::Config::default().with_tag("Rynk").with_max_level(log::LevelFilter::Info));
        std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
    });
}

// ---------------------------------------------------------------------------- window

/// A Vulkan surface on an ANativeWindow with its egui canvas
struct Win {
    native: *mut ndk_sys::ANativeWindow,
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    canvas: Canvas,
    gpu: Arc<Gpu>,
}

// The window is only touched from the main thread; the mutex makes the compiler happy
unsafe impl Send for Win {}

impl Win {
    fn new(env: &JNIEnv, surface: &JObject) -> Result<Self, String> {
        let native = unsafe { ndk_sys::ANativeWindow_fromSurface(env.get_raw() as *mut _, surface.as_raw() as *mut _) };
        let nn = NonNull::new(native as *mut std::ffi::c_void).ok_or("no native window")?;
        let instance = INSTANCE.get_or_init(gfx::instance).clone();
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(nn)),
        };
        let surface = match unsafe { instance.create_surface_unsafe(target) } {
            Ok(s) => s,
            Err(e) => {
                unsafe { ndk_sys::ANativeWindow_release(native) };
                return Err(format!("surface: {e}"));
            }
        };
        let gpu = {
            let mut g = GPU.lock().unwrap();
            match &*g {
                Some(gpu) => gpu.clone(),
                None => {
                    let gpu = Arc::new(Gpu::new(instance, Some(&surface)).inspect_err(|_| unsafe { ndk_sys::ANativeWindow_release(native) })?);
                    *g = Some(gpu.clone());
                    gpu
                }
            }
        };
        let caps = surface.get_capabilities(&gpu.adapter);
        if caps.formats.is_empty() {
            return Err("surface has no formats".into());
        }
        let format = gfx::pick_format(&caps.formats);
        let (w, h) = unsafe { (ndk_sys::ANativeWindow_getWidth(native), ndk_sys::ANativeWindow_getHeight(native)) };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w.max(1) as u32,
            height: h.max(1) as u32,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);
        let canvas = Canvas::new(&gpu, format);
        Ok(Self { native, surface: Some(surface), config, canvas, gpu })
    }

    fn resize(&mut self, w: i32, h: i32) {
        if w > 0 && h > 0 && (w as u32 != self.config.width || h as u32 != self.config.height) {
            self.config.width = w as u32;
            self.config.height = h as u32;
            if let Some(s) = &self.surface {
                s.configure(&self.gpu.device, &self.config);
            }
        }
    }

    /// The next swapchain image, or None (after reconfiguring a stale surface)
    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        let surface = self.surface.as_ref()?;
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&self.gpu.device, &self.config);
                None
            }
            _ => None,
        }
    }

    /// Draws one frame; returns whether egui asks for another one soon
    fn frame(&mut self, ppp: f32, events: Vec<egui::Event>, time: f64, clear: egui::Color32, ui: impl FnMut(&mut egui::Ui)) -> bool {
        let Some(surface) = &self.surface else { return false };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&self.gpu.device, &self.config);
                return true;
            }
            _ => return true,
        };
        let size = [self.config.width, self.config.height];
        let full = self.canvas.run(size, ppp, events, time, ui);
        let again = full.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.as_secs_f32() < 0.2);
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.canvas.draw(&self.gpu, &view, size, full, clear);
        self.gpu.queue.present(frame);
        again
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        // the Vulkan surface must go before the window it lives on
        self.surface.take();
        unsafe { ndk_sys::ANativeWindow_release(self.native) };
    }
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

fn locales(s: &str) -> Vec<String> {
    s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()
}

fn mtime(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Writes atomically: temp file, then rename
fn save(path: &PathBuf, text: &str) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

// ---------------------------------------------------------------------------- keyboard host

struct KbHost {
    kb: Keyboard,
    files: PathBuf,
    locales: Vec<String>,
    night: bool,
    prefs_mtime: Option<SystemTime>,
    font_scripts: Vec<&'static str>,
    started: std::time::Instant,
}

impl KbHost {
    fn prefs_path(&self) -> PathBuf {
        self.files.join("prefs.txt")
    }

    fn palette(&self) -> theme::Palette {
        let dark = match self.kb.prefs.theme {
            Theme::System => self.night,
            Theme::Light => false,
            Theme::Dark => true,
        };
        if dark { theme::DARK } else { theme::LIGHT }
    }

    fn reload_prefs(&mut self) {
        let p = self.prefs_path();
        let m = mtime(&p);
        if m.is_some() && m != self.prefs_mtime {
            self.prefs_mtime = m;
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let prefs = Prefs::parse(&text, &self.locales);
            if prefs != self.kb.prefs {
                self.kb.apply_prefs(prefs);
            }
        }
    }

    fn update_fonts(&mut self, ctx: &egui::Context) {
        let mut scripts: Vec<&'static str> = self.kb.prefs.languages.iter().filter_map(|c| crate::layout::LangDef::by_code(c)).map(|l| l.script).collect();
        scripts.sort_unstable();
        scripts.dedup();
        if scripts != self.font_scripts {
            ctx.set_fonts(fonts::definitions(&scripts));
            self.font_scripts = scripts;
        }
    }

    fn save_all(&mut self) {
        for (code, text) in self.kb.user_dicts() {
            save(&self.files.join(format!("dict_{code}.txt")), &text);
        }
        let calib = self.kb.calibration_text();
        save(&self.files.join("touch.txt"), &calib);
    }

    /// Lays out a frame (under the keyboard lock, which is held only for this)
    fn layout(&mut self, win: &mut Win) -> (egui::FullOutput, egui::Color32) {
        self.update_fonts(&win.canvas.ctx);
        let pal = self.palette();
        let t = self.started.elapsed().as_secs_f64();
        let dp = self.kb.dp;
        let kb = &self.kb;
        let size = [win.config.width, win.config.height];
        let full = win.canvas.run(size, dp, Vec::new(), t, |ui| kbview::paint(kb, ui.painter(), pal));
        self.kb.dirty = false;
        (full, pal.bg)
    }
}

fn with_kb<R: Default>(f: impl FnOnce(&mut KbHost) -> R) -> R {
    match KB.lock() {
        Ok(mut g) => g.as_mut().map(f).unwrap_or_default(),
        Err(_) => R::default(),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbInit(mut env: JNIEnv, _: JClass, files: JString, locs: JString, night: jboolean) {
    init_log();
    let files = PathBuf::from(jstr(&mut env, &files));
    let locales = locales(&jstr(&mut env, &locs));
    let mut g = KB.lock().unwrap();
    if g.is_some() {
        return;
    }
    let prefs_path = files.join("prefs.txt");
    let prefs = Prefs::parse(&std::fs::read_to_string(&prefs_path).unwrap_or_default(), &locales);
    if !prefs_path.exists() {
        save(&prefs_path, &prefs.serialize());
    }
    let mut kb = Keyboard::new(prefs);
    if let Ok(rd) = std::fs::read_dir(&files) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(code) = name.strip_prefix("dict_").and_then(|n| n.strip_suffix(".txt")) {
                if let Ok(t) = std::fs::read_to_string(e.path()) {
                    kb.load_user_dict(code, &t);
                }
            }
        }
    }
    let host = KbHost {
        kb,
        prefs_mtime: mtime(&prefs_path),
        files,
        locales,
        night: night == JNI_TRUE,
        font_scripts: Vec::new(),
        started: std::time::Instant::now(),
    };
    *g = Some(host);
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbNight(_: JNIEnv, _: JClass, night: jboolean) {
    with_kb(|h| {
        h.night = night == JNI_TRUE;
        h.kb.dirty = true;
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbMeasure(_: JNIEnv, _: JClass, width: jint, dp: jfloat, inset: jint, screen_h: jint) -> jint {
    with_kb(|h| {
        h.kb.set_screen_height(screen_h as f32);
        h.reload_prefs();
        let calib = std::fs::read_to_string(h.files.join("touch.txt")).unwrap_or_default();
        let first = h.kb.height <= 0.0;
        let height = h.kb.measure(width as f32, dp, inset as f32) as jint;
        if first {
            h.kb.load_calibration(&calib);
        }
        height
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbAttach(env: JNIEnv, _: JClass, surface: JObject) -> jboolean {
    let mut wg = KWIN.lock().unwrap_or_else(|e| e.into_inner());
    *wg = None;
    match Win::new(&env, &surface) {
        Ok(w) => {
            *wg = Some(w);
            drop(wg);
            with_kb(|h| {
                h.font_scripts.clear();
                h.kb.dirty = true;
            });
            JNI_TRUE
        }
        Err(e) => {
            log::error!("keyboard surface: {e}");
            JNI_FALSE
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbResize(_: JNIEnv, _: JClass, w: jint, hh: jint) {
    if let Some(win) = KWIN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        win.resize(w, hh);
    }
    with_kb(|h| h.kb.dirty = true)
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbDetach(_: JNIEnv, _: JClass) {
    *KWIN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbStartInput(_: JNIEnv, _: JClass, input_type: jint, ime_options: jint, restarting: jboolean) {
    with_kb(|h| {
        h.reload_prefs();
        h.kb.start_input(Editor::from_android(input_type, ime_options), restarting == JNI_TRUE);
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbFinishInput(_: JNIEnv, _: JClass) {
    with_kb(|h| {
        h.kb.reset_composition(true);
        h.kb.touch_cancel();
        h.save_all();
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbContext(mut env: JNIEnv, _: JClass, before: JString, caps: jboolean) {
    let before = jstr(&mut env, &before);
    with_kb(|h| h.kb.set_context(&before, caps == JNI_TRUE))
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbAbandon(_: JNIEnv, _: JClass) {
    with_kb(|h| h.kb.reset_composition(true))
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbTouch(_: JNIEnv, _: JClass, action: jint, id: jint, x: jfloat, y: jfloat, time: jlong) {
    with_kb(|h| {
        let t = time as f64;
        match action {
            0 => h.kb.touch_down(id, x, y, t),
            1 => h.kb.touch_move(id, x, y, t),
            2 => h.kb.touch_up(id, x, y, t),
            _ => h.kb.touch_cancel(),
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbTick(_: JNIEnv, _: JClass, time: jlong) {
    with_kb(|h| h.kb.tick(time as f64))
}

/// Uptime (ms) of the next timer, or -1
#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbWake(_: JNIEnv, _: JClass) -> jlong {
    with_kb(|h| h.kb.next_wake().map_or(-1, |t| t.ceil() as jlong))
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbRender(_: JNIEnv, _: JClass) {
    // Runs on the render thread. Order: window lock, then a short keyboard lock for layout.
    let mut wg = KWIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(win) = wg.as_mut() else { return };
    let Some(frame) = win.acquire() else {
        with_kb(|h| h.kb.dirty = true);
        return;
    };
    let Some((full, clear)) = with_kb(|h| Some(h.layout(win))) else { return };
    let size = [win.config.width, win.config.height];
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
    win.canvas.draw(&win.gpu, &view, size, full, clear);
    win.gpu.queue.present(frame);
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbNeedsRender(_: JNIEnv, _: JClass) -> jboolean {
    with_kb(|h| if h.kb.dirty { JNI_TRUE } else { JNI_FALSE })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbTakeCommands(env: JNIEnv, _: JClass) -> jstring {
    let s = with_kb(|h| Cmd::encode(&h.kb.take_commands()));
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Color emoji drawn by the platform: `names` space-separated, each image `size`×`size` RGBA
#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbEmoji(mut env: JNIEnv, _: JClass, names: JString, size: jint, data: JByteArray) {
    let names = jstr(&mut env, &names);
    let Ok(bytes) = env.convert_byte_array(&data) else { return };
    let n = (size.max(1) as usize).pow(2) * 4;
    with_kb(|h| {
        for (i, name) in names.split(' ').enumerate() {
            let Some(px) = bytes.get(i * n..(i + 1) * n) else { break };
            let image = crate::keyboard::EmojiImage { size: [size as usize, size as usize], rgba: Arc::new(px.to_vec()) };
            h.kb.emoji_loaded(name, image);
        }
    });
}

/// Builds a language model (`lm/*.rlm`) or conversion dictionary (`cjk/*.rcd`) from asset bytes
/// on a background thread, then hands it to the keyboard
#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_kbModel(mut env: JNIEnv, _: JClass, path: JString, data: JByteArray) {
    let path = jstr(&mut env, &path);
    let Ok(bytes) = env.convert_byte_array(&data) else { return };
    if path.ends_with(".rcd") {
        match crate::engine::cjk::ConvDict::decode(&bytes) {
            Some(d) => {
                let d = Arc::new(d);
                with_kb(|h| h.kb.conv_loaded(&path, d));
            }
            None => log::error!("corrupt dictionary {path}"),
        }
        return;
    }
    match Model::from_bytes(&bytes) {
        Some(m) => {
            let m = Arc::new(m);
            with_kb(|h| h.kb.model_loaded(&path, m));
        }
        None => log::error!("corrupt language model {path}"),
    }
}

// ---------------------------------------------------------------------------- settings host

struct StHost {
    app: SettingsApp,
    win: Option<Win>,
    files: PathBuf,
    night: bool,
    dp: f32,
    insets: (f32, f32),
    events: Vec<egui::Event>,
    started: std::time::Instant,
    fonts_set: bool,
    out: Vec<char>,
}

fn with_st<R: Default>(f: impl FnOnce(&mut StHost) -> R) -> R {
    match ST.lock() {
        Ok(mut g) => g.as_mut().map(f).unwrap_or_default(),
        Err(_) => R::default(),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stInit(mut env: JNIEnv, _: JClass, files: JString, locs: JString, night: jboolean) {
    init_log();
    let files = PathBuf::from(jstr(&mut env, &files));
    let locales = locales(&jstr(&mut env, &locs));
    let prefs = Prefs::parse(&std::fs::read_to_string(files.join("prefs.txt")).unwrap_or_default(), &locales);
    let app = SettingsApp::new(prefs, locales.first().map(|s| s.as_str()).unwrap_or("en"));
    *ST.lock().unwrap() = Some(StHost {
        app,
        win: None,
        files,
        night: night == JNI_TRUE,
        dp: 2.75,
        insets: (0.0, 0.0),
        events: Vec::new(),
        started: std::time::Instant::now(),
        fonts_set: false,
        out: Vec::new(),
    });
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stAttach(env: JNIEnv, _: JClass, surface: JObject) -> jboolean {
    with_st(|h| {
        h.win = None;
        match Win::new(&env, &surface) {
            Ok(w) => {
                h.win = Some(w);
                h.fonts_set = false;
                JNI_TRUE
            }
            Err(e) => {
                log::error!("settings surface: {e}");
                JNI_FALSE
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stResize(_: JNIEnv, _: JClass, w: jint, hh: jint, dp: jfloat, top: jint, bottom: jint) {
    with_st(|h| {
        h.dp = dp;
        h.insets = (top as f32 / dp, bottom as f32 / dp);
        if let Some(win) = &mut h.win {
            win.resize(w, hh);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stDetach(_: JNIEnv, _: JClass) {
    with_st(|h| h.win = None)
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stImeState(_: JNIEnv, _: JClass, enabled: jboolean, selected: jboolean, night: jboolean) {
    with_st(|h| {
        h.app.enabled = enabled == JNI_TRUE;
        h.app.selected = selected == JNI_TRUE;
        h.night = night == JNI_TRUE;
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stTouch(_: JNIEnv, _: JClass, action: jint, _id: jint, x: jfloat, y: jfloat, _time: jlong) {
    with_st(|h| {
        let pos = egui::pos2(x / h.dp, y / h.dp);
        let events = h.app.touch(action, pos);
        h.events.extend(events);
    })
}

/// Draws a settings frame; true when another frame is wanted (animations, pending input)
#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stRender(_: JNIEnv, _: JClass) -> jboolean {
    with_st(|h| {
        let dark = match h.app.prefs.theme {
            Theme::System => h.night,
            Theme::Light => false,
            Theme::Dark => true,
        };
        let Some(win) = &mut h.win else { return JNI_FALSE };
        if !h.fonts_set {
            win.canvas.ctx.set_fonts(fonts::definitions(&[]));
            h.fonts_set = true;
        }
        SettingsApp::style(&win.canvas.ctx, dark);
        let events = std::mem::take(&mut h.events);
        let had_events = !events.is_empty();
        let (top, bottom) = h.insets;
        let bg = if dark { egui::Color32::from_rgb(0x16, 0x16, 0x18) } else { egui::Color32::from_rgb(0xF2, 0xF3, 0xF5) };
        let app = &mut h.app;
        let again = win.frame(h.dp, events, h.started.elapsed().as_secs_f64(), bg, |ui| app.ui(ui, top + 8.0, bottom + 8.0));
        if h.app.changed {
            h.app.changed = false;
            save(&h.files.join("prefs.txt"), &h.app.prefs.serialize());
        }
        for c in std::mem::take(&mut h.app.out) {
            match c {
                SettingsCmd::EnableIme => h.out.push('E'),
                SettingsCmd::PickIme => h.out.push('P'),
                SettingsCmd::Finish => h.out.push('F'),
                SettingsCmd::DictCleared => {
                    if let Ok(rd) = std::fs::read_dir(&h.files) {
                        for e in rd.flatten() {
                            if e.file_name().to_string_lossy().starts_with("dict_") {
                                let _ = std::fs::remove_file(e.path());
                            }
                        }
                    }
                    if let Ok(mut g) = KB.lock() {
                        if let Some(k) = g.as_mut() {
                            k.kb.clear_user_dicts();
                        }
                    }
                }
            }
        }
        if again || had_events { JNI_TRUE } else { JNI_FALSE }
    })
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stTakeCommands(env: JNIEnv, _: JClass) -> jstring {
    let s: String = with_st(|h| std::mem::take(&mut h.out).into_iter().collect());
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_org_rynk_keyboard_Native_stBack(_: JNIEnv, _: JClass) -> jboolean {
    with_st(|h| if h.app.back() { JNI_TRUE } else { JNI_FALSE })
}
