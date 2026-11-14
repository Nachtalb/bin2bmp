//! Viewer for the bin2bmp visualisations plus 3D byte graphs. Runs as a desktop
//! window and, compiled to wasm, in the browser.

mod graph;

use std::sync::mpsc::{channel, Receiver, Sender};

use bin2bmp::{Layout, Mode, Options};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use graph::Camera;

const MAX_SAMPLES: usize = 4_000_000; // pairs/triples counted per file
const MAX_POINTS: usize = 400_000; // points drawn
const MAX_SIDE: usize = 4096; // largest texture every backend accepts
const THUMB: usize = 112;

/// A file's name and contents, on its way in from a dialog or a drop.
type Opened = (String, Vec<u8>);

struct Loaded {
    name: String,
    bytes: Vec<u8>,
    thumb: TextureHandle,
    kind: String,
}

#[derive(PartialEq)]
enum View {
    Flat,
    Cube,
}

/// The mode names shown in the 2D view; block and bit come from their sliders.
const FLAT_MODES: [(&str, &str); 6] = [
    ("bytes", "One pixel per byte, value = brightness."),
    ("class", "One pixel per byte: 0x00 black, 0xFF white, printable text blue, everything else red."),
    ("bit", "One pixel per byte: white if the chosen bit is set. Data hidden in the low bits looks like even static."),
    ("entropy", "One pixel per block, brighter = more random. Compressed and encrypted data is white."),
    ("chi", "One pixel per block: white where the lowest bits look overwritten. Only meaningful on raw pixels or audio."),
    ("digraph", "256×256: pixel (x, y) is how often byte x is followed by byte y."),
];

struct App {
    files: Vec<Loaded>,
    current: usize,
    opened: (Sender<Opened>, Receiver<Opened>),
    view: View,

    flat_mode: usize,
    hilbert: bool,
    block: usize,
    bit: u8,
    flat: Option<TextureHandle>,
    flat_note: String,
    zoom: f32,
    pan: egui::Vec2,

    cube_mode: graph::Mode,
    by_position: bool,
    point: usize,
    min_count: u32,
    rotate: bool,
    camera: Camera,
    tally: Option<graph::Tally>,
    cloud: Option<graph::Cloud>,
    cube: Option<TextureHandle>,
    drawn: Option<(Camera, [usize; 2], usize)>,
}

fn byte_map(bytes: &[u8]) -> ColorImage {
    let width = ((bytes.len() as f64).sqrt().ceil() as usize).max(1);
    let height = bytes.len().div_ceil(width).max(1);
    let px = (0..THUMB * THUMB).map(|i| {
        let (x, y) = (i % THUMB, i / THUMB);
        let at = (y * height / THUMB) * width + x * width / THUMB;
        Color32::from_gray(bytes.get(at).copied().unwrap_or(0))
    });
    ColorImage::new([THUMB; 2], px.collect())
}

fn human(n: usize) -> String {
    match n {
        0..1024 => format!("{n} bytes"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let mut app = Self {
            files: Vec::new(),
            current: 0,
            opened: channel(),
            view: View::Cube,
            flat_mode: 0,
            hilbert: false,
            block: 256,
            bit: 0,
            flat: None,
            flat_note: String::new(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            cube_mode: graph::Mode::Tri,
            by_position: false,
            point: 2,
            min_count: 1,
            rotate: true,
            camera: Camera {
                yaw: 0.8,
                pitch: 0.45,
                distance: 3.0,
            },
            tally: None,
            cloud: None,
            cube: None,
            drawn: None,
        };
        // Built-in samples so there is something to look at before a file is opened.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let noise: Vec<u8> = (0..200_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 32) as u8
            })
            .collect();
        let signal = noise.iter().enumerate().map(|(i, n)| {
            let i = i as f32;
            (128.0 + 70.0 * (i / 40.0).sin() + 40.0 * (i / 7.3).sin() + (n & 7) as f32 - 4.0) as u8
        });
        let source = include_str!("main.rs").as_bytes().to_vec();
        app.add(ctx, "this program's source (text)".into(), source);
        app.add(
            ctx,
            "smooth signal (like raw audio or pixels)".into(),
            signal.collect(),
        );
        app.add(ctx, "random bytes (like encrypted data)".into(), noise);
        app.select(0);
        app
    }

    /// Lists a file with a preview: the picture itself if it decodes, otherwise a byte map.
    fn add(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) {
        let (image, kind) = match image::load_from_memory(&bytes) {
            Ok(picture) => {
                let small = picture.thumbnail(THUMB as u32, THUMB as u32).to_rgba8();
                let size = [small.width() as usize, small.height() as usize];
                let kind = format!("image {}×{}", picture.width(), picture.height());
                (ColorImage::from_rgba_unmultiplied(size, &small), kind)
            }
            Err(_) => (byte_map(&bytes), "byte map".into()),
        };
        let thumb = ctx.load_texture(&name, image, TextureOptions::LINEAR);
        let kind = format!("{} · {kind}", human(bytes.len()));
        self.files.push(Loaded {
            name,
            bytes,
            thumb,
            kind,
        });
        self.select(self.files.len() - 1);
    }

    fn select(&mut self, index: usize) {
        self.current = index;
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.changed();
    }

    /// The file or a setting changed: recompute on the next frame.
    fn changed(&mut self) {
        self.flat = None;
        self.tally = None;
        self.cloud = None;
        self.drawn = None;
    }

    fn open_dialog(&self, ctx: &egui::Context) {
        let (send, ctx) = (self.opened.0.clone(), ctx.clone());
        #[cfg(not(target_arch = "wasm32"))]
        for path in rfd::FileDialog::new().pick_files().unwrap_or_default() {
            if let Ok(bytes) = std::fs::read(&path) {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                send.send((name.into_owned(), bytes)).ok();
            }
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            for file in rfd::AsyncFileDialog::new()
                .pick_files()
                .await
                .unwrap_or_default()
            {
                send.send((file.file_name(), file.read().await)).ok();
            }
            ctx.request_repaint();
        });
        #[cfg(not(target_arch = "wasm32"))]
        ctx.request_repaint();
    }

    fn take_new_files(&mut self, ctx: &egui::Context) {
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = file
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let send = self.opened.0.clone();
            #[cfg(not(target_arch = "wasm32"))]
            if let Ok(bytes) = file.bytes() {
                send.send((name, bytes)).ok();
            }
            #[cfg(target_arch = "wasm32")]
            {
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(bytes) = file.bytes_async().await {
                        send.send((name, bytes)).ok();
                        ctx.request_repaint();
                    }
                });
            }
        }
        while let Ok((name, bytes)) = self.opened.1.try_recv() {
            self.add(ctx, name, bytes);
        }
    }

    fn flat_options(&self) -> Options {
        let mode = match FLAT_MODES[self.flat_mode].0 {
            "class" => Mode::Class,
            "bit" => Mode::Bit { bit: self.bit },
            "entropy" => Mode::Entropy { block: self.block },
            "chi" => Mode::Chi { block: self.block },
            "digraph" => Mode::Digraph,
            _ => Mode::Bytes,
        };
        let layout = if self.hilbert {
            Layout::Hilbert
        } else {
            Layout::Rows
        };
        Options {
            mode,
            layout,
            width: None,
        }
    }

    /// Renders the current file with the library. Files too big for one texture are thinned first.
    fn render_flat(&mut self, ctx: &egui::Context) {
        let Some(file) = self.files.get(self.current) else {
            return;
        };
        let opts = self.flat_options();
        let stride = (opts.mode.pixel_count(file.bytes.len() as u64) as usize)
            .div_ceil(MAX_SIDE * MAX_SIDE / 4)
            .max(1);
        let thinned: Vec<u8>;
        let data = if stride > 1 {
            thinned = file.bytes.iter().step_by(stride).copied().collect();
            &thinned
        } else {
            &file.bytes
        };
        self.flat_note = match bin2bmp::render(data, &opts) {
            Ok(image) => {
                let px = image.pixels.iter().map(|&i| {
                    let [r, g, b] = image.palette[i as usize];
                    Color32::from_rgb(r, g, b)
                });
                let size = [image.width as usize, image.height as usize];
                let picture = ColorImage::new(size, px.collect());
                self.flat = Some(ctx.load_texture("flat", picture, TextureOptions::NEAREST));
                let thinned = if stride > 1 {
                    format!("\nlarge file: showing 1 byte in {stride}")
                } else {
                    String::new()
                };
                format!("{}×{} pixels{thinned}", image.width, image.height)
            }
            Err(error) => error.to_string(),
        };
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("bin2bmp viewer");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::Cube, "3D graphs");
            ui.selectable_value(&mut self.view, View::Flat, "2D maps");
        });
        ui.separator();
        let mut changed = false;
        if self.view == View::Flat {
            ui.horizontal_wrapped(|ui| {
                for (i, (name, _)) in FLAT_MODES.iter().enumerate() {
                    changed |= ui.selectable_value(&mut self.flat_mode, i, *name).changed();
                }
            });
            let (name, about) = FLAT_MODES[self.flat_mode];
            ui.label(about);
            if name != "digraph" {
                changed |= ui
                    .checkbox(&mut self.hilbert, "Hilbert layout (regions become patches)")
                    .changed();
            }
            if name == "entropy" || name == "chi" {
                let slider = egui::Slider::new(&mut self.block, 16..=65536).logarithmic(true);
                changed |= ui.add(slider.text("bytes per pixel")).changed();
            }
            if name == "bit" {
                changed |= ui
                    .add(egui::Slider::new(&mut self.bit, 0..=7).text("bit (0 = lowest)"))
                    .changed();
            }
            ui.weak(&self.flat_note);
            ui.weak("Scroll to zoom, drag to move, double-click to reset.");
        } else {
            ui.horizontal_wrapped(|ui| {
                for mode in graph::Mode::ALL {
                    changed |= ui
                        .selectable_value(&mut self.cube_mode, mode, mode.label())
                        .changed();
                }
            });
            ui.label(self.cube_mode.about());
            ui.horizontal(|ui| {
                ui.label("Colour:");
                changed |= ui
                    .selectable_value(&mut self.by_position, false, "how often")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.by_position, true, "where in the file")
                    .changed();
            });
            ui.add(egui::Slider::new(&mut self.point, 1..=8).text("point size"));
            let slider = egui::Slider::new(&mut self.min_count, 1..=50).text("hide rarer than");
            if ui.add(slider).changed() {
                self.cloud = None;
                self.drawn = None;
            }
            ui.checkbox(&mut self.rotate, "Auto-rotate");
            if let (Some(tally), Some(cloud)) = (&self.tally, &self.cloud) {
                let what = match self.cube_mode {
                    graph::Mode::Tri => "triples",
                    graph::Mode::Height => "pairs",
                    graph::Mode::Layers => "pairs × layers",
                };
                let mut info = format!("{} distinct {what}", cloud.unique);
                if cloud.unique > MAX_POINTS {
                    info += &format!(" (showing {MAX_POINTS})");
                }
                info += &format!("\nmost common: {}×", cloud.max);
                if tally.stride > 1 {
                    info += &format!("\nlarge file: only 1 in {} positions counted", tally.stride);
                }
                ui.weak(info);
            }
            ui.weak("Drag to turn, scroll to zoom.");
        }
        if changed {
            self.changed();
        }

        ui.separator();
        if ui.button("Open files…").clicked() {
            self.open_dialog(ui.ctx());
        }
        ui.weak("or drop files anywhere on the window");
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, file) in self.files.iter().enumerate() {
                let card = egui::Frame::group(ui.style()).fill(if i == self.current {
                    ui.visuals().selection.bg_fill.gamma_multiply(0.4)
                } else {
                    Color32::TRANSPARENT
                });
                let inner = card.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Image::new(&file.thumb).fit_to_exact_size(egui::vec2(56.0, 56.0)),
                        );
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(&file.name).truncate());
                            ui.weak(&file.kind);
                            if ui.small_button("remove").clicked() {
                                action = Some((i, true));
                            }
                        });
                        ui.allocate_space(egui::vec2(ui.available_width(), 0.0));
                    });
                });
                if inner.response.interact(egui::Sense::click()).clicked() && action.is_none() {
                    action = Some((i, false));
                }
            }
        });
        match action {
            Some((i, true)) => {
                self.files.remove(i);
                let last = self.files.len().saturating_sub(1);
                self.select(if i < self.current {
                    self.current - 1
                } else {
                    self.current.min(last)
                });
            }
            Some((i, false)) => self.select(i),
            None => {}
        }
    }

    fn flat_view(&mut self, ui: &mut egui::Ui) {
        if self.flat.is_none() {
            self.render_flat(ui.ctx());
        }
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let Some(texture) = &self.flat else { return };
        if response.double_clicked() {
            (self.zoom, self.pan) = (1.0, egui::Vec2::ZERO);
        }
        self.pan += response.drag_delta();
        if let Some(pointer) = response.hover_pos() {
            let factor = (ui.input(|i| i.smooth_scroll_delta().y) / 200.0).exp();
            let from_centre = pointer - response.rect.center() - self.pan;
            self.pan -= from_centre * (factor - 1.0); // keep the spot under the pointer still
            self.zoom *= factor;
        }
        let size = texture.size_vec2();
        let fit = (response.rect.width() / size.x).min(response.rect.height() / size.y);
        let rect =
            egui::Rect::from_center_size(response.rect.center() + self.pan, size * fit * self.zoom);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }

    fn cube_view(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::drag());
        let Some(file) = self.files.get(self.current) else {
            return;
        };
        let tally = self
            .tally
            .get_or_insert_with(|| graph::tally(&file.bytes, self.cube_mode, MAX_SAMPLES));
        let cloud = self.cloud.get_or_insert_with(|| {
            graph::cloud(
                tally,
                self.cube_mode,
                self.by_position,
                self.min_count,
                MAX_POINTS,
            )
        });

        let drag = response.drag_delta();
        self.camera.yaw += drag.x * 0.01;
        self.camera.pitch = (self.camera.pitch + drag.y * 0.01).clamp(-1.5, 1.5);
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            self.camera.distance =
                (self.camera.distance * (-scroll / 300.0).exp()).clamp(0.8, 12.0);
        }
        if self.rotate && !response.dragged() {
            self.camera.yaw += 0.25 * ui.input(|i| i.stable_dt).min(0.1);
            ui.ctx().request_repaint();
        }

        let scale = ui.ctx().pixels_per_point();
        let size = [
            response.rect.width() * scale,
            response.rect.height() * scale,
        ]
        .map(|v| (v as usize).clamp(1, 2048));
        let wanted = (self.camera, size, self.point);
        if self.drawn != Some(wanted) || self.cube.is_none() {
            let px = graph::draw(cloud, &self.camera, size, self.point);
            let px = px
                .into_iter()
                .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                .collect();
            let picture = ColorImage::new(size, px);
            match &mut self.cube {
                Some(texture) => texture.set(picture, TextureOptions::LINEAR),
                None => {
                    self.cube = Some(
                        ui.ctx()
                            .load_texture("cube", picture, TextureOptions::LINEAR),
                    )
                }
            }
            self.drawn = Some(wanted);
        }
        if let Some(texture) = &self.cube {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), response.rect, uv, Color32::WHITE);
        }
        let mut at = response.rect.left_bottom() + egui::vec2(12.0, -12.0);
        for (axis, name) in self.cube_mode.axes().into_iter().enumerate() {
            let [r, g, b] = graph::AXIS_COLOURS[axis];
            let text = format!("{} = {name}", ["x", "y", "z"][axis]);
            let font = egui::FontId::proportional(13.0);
            let drawn = painter.text(
                at,
                egui::Align2::LEFT_BOTTOM,
                text,
                font,
                Color32::from_rgb(r, g, b),
            );
            at.x = drawn.right() + 16.0;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.take_new_files(ui.ctx());
        egui::Panel::left("controls")
            .exact_size(300.0)
            .show(ui, |ui| self.controls(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| match self.view {
                View::Flat => self.flat_view(ui),
                View::Cube => self.cube_view(ui),
            });
    }
}

type Created = Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>>;

fn create(cc: &eframe::CreationContext<'_>) -> Created {
    Ok(Box::new(App::new(&cc.egui_ctx)))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    eframe::run_native(
        "bin2bmp viewer",
        eframe::NativeOptions::default(),
        Box::new(create),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;
    wasm_bindgen_futures::spawn_local(async {
        let document = eframe::web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("viewer")
            .expect("no #viewer canvas");
        let canvas = canvas
            .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
            .expect("#viewer is not a canvas");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(create))
            .await;
        started.expect("failed to start the viewer");
    });
}
