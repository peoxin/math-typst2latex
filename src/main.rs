use clipboard_rs::{Clipboard, ClipboardContext};
use eframe::egui;
use mathjax_svg;
use resvg;
use std::io::Write;
use std::process::{Command, Stdio};
use tiny_skia;
use tiny_skia_path;
use usvg;

fn convert_typst_to_latex(input: &str) -> Result<String, String> {
    let mut child = Command::new("pandoc")
        .arg("-f")
        .arg("typst")
        .arg("-t")
        .arg("latex")
        .arg("--")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Failed to execute pandoc. Do you have it installed?")?;

    child
        .stdin
        .take()
        .ok_or("Failed to open stdin")?
        .write_all(format!("$\n{}\n$", input).as_bytes()) // Add delimiters to treat input as math.
        .map_err(|_| "Failed to write to stdin")?;
    let output = child
        .wait_with_output()
        .map_err(|_| "Failed to read stdout and stderr")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_start_matches(r"\[")
            .trim_end_matches(r"\]") // Remove LaTeX math delimiters.
            .trim()
            .to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).to_string();
        Err(error
            .split_once(":") // Remove line and column number from error message.
            .unwrap_or_else(|| ("", &error))
            .1
            .trim()
            .to_string())
    }
}

fn invert_pixmap_color(pixmap: &mut tiny_skia::Pixmap) {
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        pixel[0] = 255 - pixel[0];
        pixel[1] = 255 - pixel[1];
        pixel[2] = 255 - pixel[2];
    }
}

fn svg_to_texture(
    ctx: &egui::Context,
    svg: &str,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())?;
    let scale = 5.0;
    let width = tree.size().width() * scale;
    let height = tree.size().height() * scale;
    let mut pixmap =
        tiny_skia::Pixmap::new(width as u32, height as u32).ok_or("Failed to create pixmap")?;
    resvg::render(
        &tree,
        tiny_skia_path::Transform::from_scale(scale * 0.9, scale * 0.9),
        &mut pixmap.as_mut(),
    );

    // Invert symbol color to white if dark mode is enabled.
    if ctx.style().visuals.dark_mode {
        invert_pixmap_color(&mut pixmap);
    }

    let image =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], pixmap.data());
    Ok(ctx.load_texture("latex_svg", image, Default::default()))
}

struct MyApp {
    input: String,
    output: String,
    texture: Option<egui::TextureHandle>,
    clipboard: Option<ClipboardContext>,
    copy_enabled: bool,
}

impl MyApp {
    fn new() -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            texture: None,
            clipboard: ClipboardContext::new().ok(),
            copy_enabled: false,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Set font size.
            ui.style_mut().override_font_id = Some(egui::FontId {
                size: 16.0,
                family: egui::FontFamily::Proportional,
            });

            ui.add_space(10.0);
            let input_response = egui::ScrollArea::both()
                .id_salt("input_scroll_area")
                .auto_shrink([false, true])
                .max_height(100.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.input)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;

            let output_to_texture = |obj: &mut Self| {
                if obj.output.starts_with("Error") || obj.output.is_empty() {
                    return;
                }
                if let Ok(svg_data) = mathjax_svg::convert_to_svg(&obj.output) {
                    if let Ok(texture) = svg_to_texture(ctx, &svg_data) {
                        obj.texture = Some(texture);
                        obj.copy_enabled = true;
                    } else {
                        eprintln!("Failed to convert SVG to texture");
                    }
                } else {
                    eprintln!("Failed to convert LaTeX to SVG");
                }
            };
            if input_response.changed() {
                self.texture = None;
                self.copy_enabled = false;
                match convert_typst_to_latex(&self.input) {
                    Ok(result) => {
                        self.output = result;
                        output_to_texture(self);
                    }
                    Err(err) => {
                        self.output = format!("Error: {}", err);
                    }
                }
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_space(145.0);
                if ui
                    .add_enabled(self.copy_enabled, egui::Button::new("Copy LaTeX"))
                    .clicked()
                {
                    match &self.clipboard {
                        Some(clipboard) => {
                            if clipboard.set_text(self.output.clone()).is_err() {
                                eprintln!("Failed to copy to clipboard");
                            }
                        }
                        None => eprintln!("Failed to initialize clipboard support"),
                    }
                }
                if ui.button("Clear").clicked() {
                    self.input.clear();
                    self.output.clear();
                    self.texture = None;
                    self.copy_enabled = false;
                }
            });

            ui.add_space(10.0);
            let output_response = egui::ScrollArea::both()
                .id_salt("output_scroll_area")
                .auto_shrink([false, true])
                .max_height(100.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.output)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;
            if output_response.changed() {
                self.texture = None;
                self.copy_enabled = false;
                output_to_texture(self);
            }

            ui.add_space(10.0);
            if let Some(texture) = &self.texture {
                let available_width = ui.available_width();
                let size = texture.size_vec2();
                let scale = f32::min(available_width / size.x * 0.9, 1.0);
                let scaled_size = egui::vec2(size.x * scale, size.y * scale);
                ui.centered_and_justified(|ui| {
                    ui.image((texture.id(), scaled_size));
                });
            }
        });
    }
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_resizable(false)
            .with_inner_size([450.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Typst to LaTeX Math Converter",
        native_options,
        Box::new(|_cc| Ok(Box::new(MyApp::new()))),
    )
}
