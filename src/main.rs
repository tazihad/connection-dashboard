//! A fast, offline-friendly connection dashboard.

mod icons;
mod net;
mod theme;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::layout::Section;
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};

use icons::{Icon, Tone};
use net::Snapshot;
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
        net::spawn_monitor(Arc::clone(&snapshot), cc.egui_ctx.clone());

        Self {
            snapshot,
            latest: None,
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

    fn header(&mut self, ui: &mut Ui, palette: &Palette) {
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
        // Refresh loop wakes every 500ms; request repaint to keep UI live.
        ctx.request_repaint_after(Duration::from_millis(500));

        let palette = theme::palette(self.applied_theme == egui::Theme::Dark);
        let snapshot = self.latest.clone();

        egui::CentralPanel::default()
        .frame(
            Frame::new()
            .fill(palette.background)
            .inner_margin(Margin::symmetric(24, 18)),
        )
        .show(ui, |ui| {
            // One geometry for both sections, so they always line up.
            let metrics = Metrics::new(ui.available_width());
            self.header(ui, &palette);
            ui.add_space(14.0);
            cards(ui, metrics, snapshot.as_ref(), &palette);
            ui.add_space(16.0);
            details(ui, metrics, snapshot.as_ref(), &palette);
        });
    }
}

/// Horizontal geometry shared by the status cards and the details panel, so the two always line
/// up and both shrink together with the window.
#[derive(Clone, Copy)]
struct Metrics {
    card_width: f32,
    arrow_width: f32,
    /// Space between a card and the arrow next to it.
    gap: f32,
    /// Left padding that centres the group when the window is wider than the group needs.
    left_pad: f32,
    /// Exact width of the four cards, three arrows and the gaps around them.
    group_width: f32,
}

impl Metrics {
    const CARDS: f32 = 4.0;
    const MIN_CARD_WIDTH: f32 = 120.0;
    const MAX_CARD_WIDTH: f32 = 300.0;

    fn new(available: f32) -> Self {
        // Roomy spacing on wide windows, tighter on narrow ones, blended so nothing jumps.
        let roominess = ((available - 600.0) / 300.0).clamp(0.0, 1.0);
        let arrow_width = 22.0 + 12.0 * roominess;
        let gap = 8.0 + 4.0 * roominess;
        // 3 arrows, each with a gap on both sides.
        let spacing = 3.0 * arrow_width + 6.0 * gap;
        let card_width = ((available - spacing) / Self::CARDS)
        .clamp(Self::MIN_CARD_WIDTH, Self::MAX_CARD_WIDTH);
        let group_width = Self::CARDS * card_width + spacing;
        Self {
            card_width,
            arrow_width,
            gap,
            left_pad: ((available - group_width) / 2.0).max(0.0),
            group_width,
        }
    }
}

/// Smallest font size used when shrinking text to fit a narrow card.
const MIN_TEXT_SIZE: f32 = 8.0;

/// Largest font size up to `max_size` at which `text` fits in `max_width`, never below
/// [`MIN_TEXT_SIZE`].
fn fit_size(ui: &Ui, text: &str, max_size: f32, max_width: f32) -> f32 {
    // Leave a pixel of slack so rounding never pushes the last glyph onto a new line.
    let max_width = max_width - 1.0;
    let width_at = |size: f32| {
        ui.painter()
        .layout_no_wrap(
            text.to_owned(),
                        FontId::proportional(size),
                        Color32::PLACEHOLDER,
        )
        .size()
        .x
    };
    let full = width_at(max_size);
    if full <= max_width {
        return max_size;
    }
    // Glyphs snap to whole pixels, so width is not exactly linear in the font size: start from
    // the linear estimate and step down until the text really fits.
    let mut size = (max_size * max_width / full).clamp(MIN_TEXT_SIZE, max_size);
    while size > MIN_TEXT_SIZE && width_at(size) > max_width {
        size = (size - 0.5).max(MIN_TEXT_SIZE);
    }
    size
}

fn cards(ui: &mut Ui, metrics: Metrics, snapshot: Option<&Snapshot>, palette: &Palette) {
    let height = 200.0;
    let checking = snapshot.is_none();

    let (desktop, router, provider_gateway, internet) = match snapshot {
        Some(snapshot) => (
            Some(snapshot.desktop_up),
                           Some(snapshot.router_up),
                           Some(snapshot.provider_gateway_up),
                           Some(snapshot.internet_up),
        ),
        None => (None, None, None, None),
    };
    let ip_text = |ip: Option<&str>| match ip {
        Some(ip) => format!("IP: {ip}"),
        None if checking => "IP: checking...".to_owned(),
        None => "IP: no address".to_owned(),
    };

    ui.horizontal(|ui| {
        // All spacing is added by hand below, so the row is exactly `left_pad + group_width`.
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(metrics.left_pad);

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
                "Provider Gateway",
             Icon::ProviderGateway,
             snapshot.and_then(|snapshot| snapshot.provider_gateway_ip.as_deref()),
             provider_gateway,
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
                ui.add_space(metrics.gap);
                arrow(ui, vec2(metrics.arrow_width, height), palette);
                ui.add_space(metrics.gap);
            }
            status_card(
                ui,
                vec2(metrics.card_width, height),
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
    const STROKE: f32 = 1.0;
    // Narrow cards get slimmer side margins so the text has room.
    let margin_x = (size.x * 0.085).clamp(8.0, 14.0).round();
    let margin_y = 16.0;
    // The frame's stroke counts towards its outer size, so subtract it as well to land on `size`.
    let inner = vec2(
        size.x - 2.0 * (margin_x + STROKE),
                     size.y - 2.0 * (margin_y + STROKE),
    );
    Frame::new()
    .fill(palette.card)
    .stroke(Stroke::new(STROKE, palette.border))
    .corner_radius(CornerRadius::same(12))
    .inner_margin(Margin {
        left: margin_x as i8,
        right: margin_x as i8,
        top: margin_y as i8,
        bottom: margin_y as i8,
    })
    .show(ui, |ui| {
        ui.set_min_size(inner);
        ui.set_max_width(inner.x);
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            let (icon_rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            let icon_color = match connected {
                Some(true) => palette.accent,
                             _ => palette.muted,
            };
            icons::paint(ui.painter(), icon_rect, icon, icon_color);
            ui.add_space(6.0);
            // Both labels are shrunk to fit; truncation is only a last-resort safety net.
            let title_size = fit_size(ui, title, 17.0, inner.x);
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                    .size(title_size)
                    .strong()
                    .color(palette.text),
                )
                .truncate(),
            );
            ui.add_space(2.0);
            let ip_size = fit_size(ui, ip, 13.0, inner.x);
            ui.add(
                egui::Label::new(RichText::new(ip).size(ip_size).color(palette.muted))
                .truncate(),
            );
            ui.add_space(12.0);
            status_line(ui, connected, palette, inner.x);
        });
    });
}

fn status_line(ui: &mut Ui, connected: Option<bool>, palette: &Palette, max_width: f32) {
    let (text, color) = match connected {
        Some(true) => ("Connected", palette.green),
        Some(false) => ("Not connected", palette.red),
        None => ("Checking...", palette.muted),
    };
    let icon_width = 16.0;
    let gap = 7.0;
    let text_size = fit_size(ui, text, 15.0, max_width - icon_width - gap);
    let font = FontId::proportional(text_size);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
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
    // Shrink the arrow along with its slot so it never touches the neighbouring cards.
    let half = (size.x * 0.5 - 4.0).clamp(4.0, 13.0);
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

fn details(ui: &mut Ui, metrics: Metrics, snapshot: Option<&net::Snapshot>, palette: &Palette) {
    // Same geometry as the cards, so the panel spans exactly the card group.
    let item_spacing = ui.spacing().item_spacing;

    let frame = Frame::new()
    .fill(palette.card)
    .stroke(Stroke::new(1.0, palette.border))
    .corner_radius(CornerRadius::same(12))
    .inner_margin(Margin::symmetric(16, 12));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(metrics.left_pad);

        Frame::new().inner_margin(0.0).show(ui, |ui| {
            ui.set_min_width(metrics.group_width);
            ui.set_max_width(metrics.group_width);
            // The row above zeroed the spacing to place the panel; restore it for the contents.
            ui.spacing_mut().item_spacing = item_spacing;

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
                    let (icon_rect, _) =
                    ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::paint_tone(ui.painter(), icon_rect, tone, color);
                    // Wrap long messages instead of running past the panel on narrow windows.
                    ui.add(
                        egui::Label::new(
                            RichText::new(message).size(14.5).strong().color(color),
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    );
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
        });
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
        ui.spacing_mut().item_spacing.x = 8.0;
        let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        icons::paint(ui.painter(), icon_rect, icon, palette.muted);
        ui.label(
            RichText::new(format!("{} ({})", interface.kind.label(), interface.name))
            .size(13.5)
            .color(palette.text),
        );
        // Status aligned left (after the name), not pushed to far right
        ui.label(
            RichText::new(interface.state_text())
            .size(13.5)
            .strong()
            .color(color),
        );
    });
}

fn summary(snapshot: &Snapshot, palette: &Palette) -> (Tone, Color32, &'static str) {
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
    } else if !snapshot.provider_gateway_up {
        (
            Tone::Warn,
         palette.amber,
         "Connected to router, but cannot reach ISP gateway. Last-mile / ISP CPE issue.",
        )
    } else if !snapshot.internet_up {
        (
            Tone::Warn,
         palette.amber,
         "Connected to ISP gateway, but no internet. ISP backbone / peering issue.",
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
