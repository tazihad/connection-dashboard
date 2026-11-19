//! A fast, offline-friendly connection dashboard.

mod icons;
mod net;
mod theme;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::layout::Section;
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};

use icons::{Icon, Tone};
use theme::Palette;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Connection Dashboard")
            .with_inner_size([950.0, 600.0])
            .with_min_inner_size([700.0, 520.0])
            .with_icon(window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Connection Dashboard",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct App {
    snapshot: Arc<Mutex<Option<net::Snapshot>>>,
    latest: Option<net::Snapshot>,
    busy: Arc<AtomicBool>,
    refresh_tx: mpsc::Sender<()>,
    applied_theme: egui::Theme,
    icon_applied: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Follow the desktop's preference, but keep our own colours for either theme.
        cc.egui_ctx.set_theme(egui::ThemePreference::System);
        let applied_theme = cc.egui_ctx.theme();
        theme::install(&cc.egui_ctx, applied_theme == egui::Theme::Dark);

        let snapshot = Arc::new(Mutex::new(None));
        let busy = Arc::new(AtomicBool::new(true));
        let (refresh_tx, refresh_rx) = mpsc::channel();
        net::spawn_monitor(
            Arc::clone(&snapshot),
            Arc::clone(&busy),
            cc.egui_ctx.clone(),
            refresh_rx,
        );

        Self {
            snapshot,
            latest: None,
            busy,
            refresh_tx,
            applied_theme,
            icon_applied: false,
        }
    }

    /// eframe does not forward the viewport icon to winit on Linux, so set the X11
    /// `_NET_WM_ICON` property once the window exists. Wayland has no per-window icon;
    /// there the desktop entry installed from the AppImage provides it.
    fn apply_window_icon(&mut self, frame: &mut eframe::Frame) {
        if self.icon_applied {
            return;
        }
        let Some(window) = frame.winit_window() else {
            return;
        };
        self.icon_applied = true;
        if let Some(image) = load_icon_image() {
            // X11 caps a single property request at 65535 32-bit units, so the full
            // 512x512 image cannot be sent in one go. A 128x128 icon is ample for
            // taskbars and window switchers.
            const EDGE: u32 = 128;
            let image =
                image::imageops::resize(&image, EDGE, EDGE, image::imageops::FilterType::Lanczos3);
            if let Ok(icon) = winit::window::Icon::from_rgba(image.into_raw(), EDGE, EDGE) {
                window.set_window_icon(Some(icon));
            }
        }
    }

    fn poll_snapshot(&mut self) {
        // Never wait on the worker: if it is publishing, keep the previous frame's data.
        if let Ok(guard) = self.snapshot.try_lock()
            && let Some(snapshot) = guard.as_ref()
        {
            self.latest = Some(snapshot.clone());
        }
    }

    fn header(&mut self, ui: &mut Ui, busy: bool, palette: &Palette) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Connection Dashboard")
                        .size(24.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(1.0);
                ui.label(
                    RichText::new("Local link, gateway and internet reachability")
                        .size(12.5)
                        .color(palette.muted),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let theme_button =
                    icon_button(ui, "Switch color theme", Icon::Theme, palette, false);
                if theme_button
                    .on_hover_text("Toggle light and dark mode")
                    .clicked()
                {
                    let next = if self.applied_theme == egui::Theme::Dark {
                        egui::ThemePreference::Light
                    } else {
                        egui::ThemePreference::Dark
                    };
                    ui.ctx().set_theme(next);
                }

                let refresh_button =
                    icon_button(ui, "Refresh network status", Icon::Refresh, palette, false);
                if refresh_button.on_hover_text("Check again now").clicked() {
                    let _ = self.refresh_tx.send(());
                    ui.ctx().request_repaint();
                }

                ui.add_space(4.0);
                if busy {
                    ui.add(egui::Spinner::new().size(14.0).color(palette.muted));
                }
                let text = if self.latest.is_none() {
                    "Checking...".to_owned()
                } else {
                    age_text(self.latest.as_ref().map(|snapshot| snapshot.finished_at))
                };
                ui.label(RichText::new(text).size(12.5).color(palette.muted));
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_window_icon(frame);
        let resolved = ctx.theme();
        if resolved != self.applied_theme {
            self.applied_theme = resolved;
            theme::install(&ctx, resolved == egui::Theme::Dark);
        }

        self.poll_snapshot();
        if ctx.input(|input| input.key_pressed(egui::Key::F5)) {
            let _ = self.refresh_tx.send(());
        }
        // Keep "Updated 12s ago" and the spinner alive without a busy repaint loop.
        ctx.request_repaint_after(Duration::from_secs(1));

        let palette = theme::palette(self.applied_theme == egui::Theme::Dark);
        let busy = self.busy.load(Ordering::Relaxed);
        let snapshot = self.latest.clone();

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette.background)
                    .inner_margin(Margin::symmetric(24, 18)),
            )
            .show(ui, |ui| {
                self.header(ui, busy, &palette);
                ui.add_space(14.0);
                cards(ui, snapshot.as_ref(), &palette);
                ui.add_space(16.0);
                details(ui, snapshot.as_ref(), &palette);
            });
    }
}

fn age_text(last: Option<Instant>) -> String {
    let Some(last) = last else {
        return "Starting...".to_owned();
    };
    let seconds = last.elapsed().as_secs();
    match seconds {
        0..=1 => "Updated just now".to_owned(),
        2..=59 => format!("Updated {seconds}s ago"),
        60..=3599 => format!("Updated {}m ago", seconds / 60),
        _ => format!("Updated {}h ago", seconds / 3600),
    }
}

fn cards(ui: &mut Ui, snapshot: Option<&net::Snapshot>, palette: &Palette) {
    let available = ui.available_width();
    let spacing = 12.0;
    let arrow_width = 34.0;
    let card_width = ((available - 2.0 * arrow_width - 4.0 * spacing) / 3.0).clamp(170.0, 300.0);
    let group_width = 3.0 * card_width + 2.0 * arrow_width + 4.0 * spacing;
    let left_pad = ((available - group_width) / 2.0).max(0.0);
    let height = 200.0;
    let checking = snapshot.is_none();

    let (desktop, router, internet) = match snapshot {
        Some(snapshot) => (
            Some(snapshot.desktop_up),
            Some(snapshot.router_up),
            Some(snapshot.internet_up),
        ),
        None => (None, None, None),
    };
    let ip_text = |ip: Option<&str>| match ip {
        Some(ip) => format!("IP: {ip}"),
        None if checking => "IP: checking...".to_owned(),
        None => "IP: no address".to_owned(),
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        ui.add_space(left_pad);

        let cards = [
            (
                "Desktop",
                Icon::Desktop,
                snapshot.and_then(|snapshot| snapshot.local_ip.as_deref()),
                desktop,
            ),
            (
                "Router",
                Icon::Router,
                snapshot.and_then(|snapshot| snapshot.gateway_ip.as_deref()),
                router,
            ),
            (
                "Internet",
                Icon::Internet,
                snapshot.and_then(|snapshot| snapshot.public_ip.as_deref()),
                internet,
            ),
        ];

        for (index, (title, icon, ip, status)) in cards.into_iter().enumerate() {
            if index > 0 {
                arrow(ui, vec2(arrow_width, height), palette);
            }
            status_card(
                ui,
                vec2(card_width, height),
                title,
                icon,
                &ip_text(ip),
                status,
                palette,
            );
        }
    });
}

fn status_card(
    ui: &mut Ui,
    size: Vec2,
    title: &str,
    icon: Icon,
    ip: &str,
    connected: Option<bool>,
    palette: &Palette,
) {
    let margins = vec2(14.0, 16.0);
    Frame::new()
        .fill(palette.card)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin {
            left: margins.x as i8,
            right: margins.x as i8,
            top: margins.y as i8,
            bottom: margins.y as i8,
        })
        .show(ui, |ui| {
            ui.set_min_size(size - margins * 2.0);
            ui.set_max_width(size.x - margins.x * 2.0);
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                let (icon_rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                let icon_color = match connected {
                    Some(true) => palette.accent,
                    _ => palette.muted,
                };
                icons::paint(ui.painter(), icon_rect, icon, icon_color);
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(17.0).strong().color(palette.text));
                ui.add_space(2.0);
                ui.label(RichText::new(ip).size(13.0).color(palette.muted));
                ui.add_space(12.0);
                status_line(ui, connected, palette);
            });
        });
}

fn status_line(ui: &mut Ui, connected: Option<bool>, palette: &Palette) {
    let (text, color) = match connected {
        Some(true) => ("Connected", palette.green),
        Some(false) => ("Not connected", palette.red),
        None => ("Checking...", palette.muted),
    };
    let font = FontId::proportional(15.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let icon_width = 16.0;
    let gap = 7.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(icon_width + gap + galley.size().x, 22.0),
        Sense::hover(),
    );
    let icon_rect = egui::Rect::from_center_size(
        pos2(rect.left() + icon_width / 2.0, rect.center().y),
        vec2(icon_width, icon_width),
    );
    match connected {
        Some(ok) => icons::paint_result(ui.painter(), icon_rect, ok, color),
        None => {
            ui.painter()
                .circle_stroke(icon_rect.center(), 5.5, Stroke::new(1.5, color));
        }
    }
    let text_pos = pos2(
        rect.left() + icon_width + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, color);
}

fn arrow(ui: &mut Ui, size: Vec2, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let center = rect.center();
    let half = 13.0;
    let stroke = Stroke::new(1.6, palette.muted);
    ui.painter().line_segment(
        [
            pos2(center.x - half, center.y),
            pos2(center.x + half, center.y),
        ],
        stroke,
    );
    let tip = pos2(center.x + half, center.y);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, tip + vec2(-7.0, -4.5), tip + vec2(-7.0, 4.5)],
        palette.muted,
        Stroke::NONE,
    ));
}

fn details(ui: &mut Ui, snapshot: Option<&net::Snapshot>, palette: &Palette) {
    let frame = Frame::new()
        .fill(palette.card)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 12));

    Section::new(frame)
        .heading(
            RichText::new("Details")
                .size(14.0)
                .strong()
                .color(palette.text),
        )
        .show(ui, |ui| {
            let Some(snapshot) = snapshot else {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(palette.muted));
                    ui.label(
                        RichText::new("Checking network status...")
                            .size(14.0)
                            .color(palette.muted),
                    );
                });
                return;
            };

            let (tone, color, message) = summary(snapshot, palette);
            ui.horizontal(|ui| {
                let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                icons::paint_tone(ui.painter(), icon_rect, tone, color);
                ui.label(RichText::new(message).size(14.5).strong().color(color));
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(3.0);

            let links: Vec<&net::Interface> = snapshot
                .interfaces
                .iter()
                .filter(|interface| interface.kind != net::LinkKind::Other)
                .collect();
            if links.is_empty() {
                ui.label(
                    RichText::new("No Ethernet or Wi-Fi interfaces were found.")
                        .size(13.0)
                        .color(palette.muted),
                );
            } else {
                for interface in links {
                    interface_row(ui, interface, palette);
                }
            }
        });
}

fn interface_row(ui: &mut Ui, interface: &net::Interface, palette: &Palette) {
    let connected = interface.connected();
    let color = if connected {
        palette.green
    } else {
        palette.red
    };
    let icon = match interface.kind {
        net::LinkKind::Ethernet => Icon::Ethernet,
        net::LinkKind::Wifi => Icon::Wifi,
        net::LinkKind::Other => Icon::Internet,
    };
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        icons::paint(ui.painter(), icon_rect, icon, palette.muted);
        ui.label(
            RichText::new(format!("{} ({})", interface.kind.label(), interface.name))
                .size(13.5)
                .color(palette.text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(interface.state_text())
                    .size(13.5)
                    .strong()
                    .color(color),
            );
        });
    });
}

fn summary(snapshot: &net::Snapshot, palette: &Palette) -> (Tone, Color32, &'static str) {
    if !snapshot.desktop_up {
        (
            Tone::Error,
            palette.red,
            "Problem with the desktop network adapter or configuration.",
        )
    } else if !snapshot.router_up {
        (
            Tone::Warn,
            palette.amber,
            "Problem connecting to the router. Check the router and the cable/Wi-Fi.",
        )
    } else if !snapshot.internet_up {
        (
            Tone::Warn,
            palette.amber,
            "Connected to the router, but there is no internet. The issue may be with the ISP.",
        )
    } else {
        (Tone::Ok, palette.green, "Network is working fine.")
    }
}

fn icon_button(
    ui: &mut Ui,
    label: &str,
    icon: Icon,
    palette: &Palette,
    selected: bool,
) -> egui::Response {
    let style = IconButtonStyle {
        normal: ButtonVisuals {
            fill: Color32::TRANSPARENT,
            stroke: Stroke::NONE,
            icon: palette.muted,
        },
        hovered: ButtonVisuals {
            fill: palette.button_hover,
            stroke: Stroke::NONE,
            icon: palette.text,
        },
        pressed: Some(ButtonVisuals {
            fill: palette.button_active,
            stroke: Stroke::NONE,
            icon: palette.text,
        }),
        selected: ButtonVisuals {
            fill: palette.button,
            stroke: Stroke::NONE,
            icon: palette.text,
        },
        corner_radius: CornerRadius::same(7),
        focus_stroke: Stroke::new(1.0, palette.accent),
    };

    IconButton::new(label, Vec2::splat(30.0), &style)
        .selected(selected)
        .show(ui, |ui, rect, tint| {
            icons::paint(ui.painter(), rect.shrink(7.0), icon, tint);
        })
}

fn load_icon_image() -> Option<image::RgbaImage> {
    Some(
        image::load_from_memory(include_bytes!("../icon.png"))
            .ok()?
            .to_rgba8(),
    )
}

fn window_icon() -> egui::IconData {
    load_icon_image()
        .map(|image| egui::IconData {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
        .unwrap_or(egui::IconData {
            rgba: vec![0, 0, 0, 255],
            width: 1,
            height: 1,
        })
}
