mod core;
mod types;
mod node_graph;
mod scene_graph;
mod properties;
mod viewport;
mod ice;
mod usd_loader;
mod usd_scene;
mod usda_text;
mod prim_inspector;
mod fbx_loader;
mod fbx_writer;
mod file_browser;
mod batch;
mod graph_io;
mod timeline;
mod theme;
mod modelling;
mod layout;
mod uv_editor;
mod uv_canvas;
mod examples;
mod templates;
mod ragdoll;
mod packed;
mod history;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

use types::{MainCamera, GeneratedMesh, GroundGrid, MeshData, SceneHierarchy, SubnetId, ViewportRect, PrimInspectorState};
use node_graph::NodeGraphState;
use scene_graph::OperatorStack;
use viewport::camera::{CameraOrbitState, camera_controller, focus_camera, draw_origin_label};
use ice::{SubnetStore, GraphNavigation, ui::{draw_subnet_graph, draw_breadcrumb}};
use node_graph::ui::draw_node_graph;
use scene_graph::ui::{draw_scene_explorer, draw_operator_stack};
use properties::ui::draw_properties_panel;
use prim_inspector::ui::draw_prim_inspector;
use types::NodeType;
use timeline::{Playback, TimelineState, resolve_source, ui::draw_timeline};
use properties::ui::{AnimContext, PanelAction, PanelIo};
use file_browser::{BrowseMode, BrowseTarget, FileBrowser};
use batch::BatchState;
use layout::{Layout, Pane};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "XMS | Imago".into(),
                // Window class on X11 and app id on Wayland, so the desktop
                // groups the window under its own name.
                name: Some("xms-imago".into()),
                // Opens on the primary screen. `close_splash` then makes it
                // fill that screen.
                position: WindowPosition::Centered(MonitorSelection::Primary),
                // Hidden until the splash closes.
                visible: false,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin)
        .init_resource::<NodeGraphState>()
        .init_resource::<CameraOrbitState>()
        .init_resource::<OperatorStack>()
        .init_resource::<SceneHierarchy>()
        .init_resource::<SubnetStore>()
        .init_resource::<GraphNavigation>()
        .init_resource::<ViewportRect>()
        .init_resource::<PrimInspectorState>()
        .init_resource::<Playback>()
        .init_gizmo_group::<modelling::PolyGizmos>()
        .init_resource::<FileBrowser>()
        .init_resource::<BatchState>()
        .init_resource::<GraphFile>()
        .init_resource::<modelling::PolyTool>()
        .init_resource::<Layout>()
        .init_resource::<uv_editor::UvEditorState>()
        .init_resource::<node_graph::GraphRevision>()
        .init_resource::<viewport::nav::NavSettings>()
        .init_resource::<history::History>()
        .add_systems(Startup, (open_splash, setup_scene, setup_egui_theme, setup_gizmos, modelling::setup_gizmos))
        .add_systems(Update, (
            dcc_ui,
            close_splash,
            set_window_icon,
            history_system.after(dcc_ui).after(modelling::pick_system),
            track_revision.after(history_system),
            update_operator_stack.after(track_revision),
            update_scene_hierarchy.after(track_revision),
            update_generated_meshes.after(track_revision),
            apply_viewport_rect.after(dcc_ui),
            camera_controller,
            focus_camera,
            draw_origin_label,
            draw_skeleton.after(dcc_ui),
            modelling::pick_system.after(dcc_ui),
            modelling::overlay_system.after(dcc_ui),
        ))
        .run();
}
// -- Egui theme setup ------------------------------------------------

fn setup_egui_theme(mut contexts: EguiContexts) {
    theme::init(contexts.ctx_mut());
}

// ── Icon and splash ──────────────────────────────────────────────────────────

fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

/// Give the window its icon, which the task bar shows. Tried every frame
/// until the window exists. X11 and Windows use it; Wayland and macOS take
/// the icon from the installed application instead.
fn set_window_icon(windows: NonSend<bevy::winit::WinitWindows>, mut done: Local<bool>) {
    if *done || windows.windows.is_empty() { return; }
    *done = true;
    let Some((rgba, w, h)) = decode_png(include_bytes!("../assets/icon.png")) else { return };
    let Ok(icon) = winit::window::Icon::from_rgba(rgba, w, h) else { return };
    for window in windows.windows.values() { window.set_window_icon(Some(icon.clone())); }
}

/// How long the splash stays, in seconds.
const SPLASH_HOLD: f64 = 3.0;
/// Width of the splash window, in logical pixels. The height follows the image.
const SPLASH_WIDTH: f32 = 740.0;
/// The splash is drawn on its own layer, so nothing else lands in its window.
const SPLASH_LAYER: usize = 31;

/// Everything that belongs to the splash window.
#[derive(Component)]
struct SplashPart;

/// The splash: a borderless window of its own, always on top, in the middle
/// of the primary screen. The main window stays hidden until it closes.
/// `XMS_NO_SPLASH` set to anything skips it.
fn open_splash(
    mut commands: Commands,
    mut images:   ResMut<Assets<Image>>,
    mut main:     Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    use bevy::render::{camera::RenderTarget, render_asset::RenderAssetUsages, view::RenderLayers};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::window::{WindowLevel, WindowRef, WindowResolution};

    let decoded = if std::env::var_os("XMS_NO_SPLASH").is_some() { None } else { decode_png(include_bytes!("../assets/splash.png")) };
    let Some((rgba, w, h)) = decoded else {
        // No splash: the main window shows at once.
        for mut window in &mut main { window.visible = true; window.set_maximized(true); }
        return;
    };
    let size = Vec2::new(SPLASH_WIDTH, SPLASH_WIDTH * h as f32 / w as f32);
    let image = images.add(Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default()));
    let window = commands.spawn((SplashPart, Window {
        title: "XMS | Imago".into(),
        name: Some("xms-imago".into()),
        resolution: WindowResolution::new(size.x, size.y),
        position: WindowPosition::Centered(MonitorSelection::Primary),
        decorations: false,
        resizable: false,
        window_level: WindowLevel::AlwaysOnTop,
        skip_taskbar: true,
        ..default()
    })).id();
    commands.spawn((SplashPart, RenderLayers::layer(SPLASH_LAYER), Camera2dBundle {
        camera: Camera { target: RenderTarget::Window(WindowRef::Entity(window)), clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        ..default()
    }));
    commands.spawn((SplashPart, RenderLayers::layer(SPLASH_LAYER), SpriteBundle {
        texture: image,
        sprite: Sprite { custom_size: Some(size), ..default() },
        ..default()
    }));
}

/// Close the splash when its time is up, or on a click or key in it, then
/// show the main window filling the primary screen.
fn close_splash(
    mut commands: Commands,
    time:         Res<Time<Real>>,
    parts:        Query<Entity, With<SplashPart>>,
    mut main:     Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mouse:        Res<ButtonInput<MouseButton>>,
    keys:         Res<ButtonInput<KeyCode>>,
    mut clock:    Local<(u32, f64)>,
    mut shown:    Local<u32>,
) {
    // After the splash: the window is shown, then maximised on the next
    // frame, once the desktop has it on screen.
    if *shown > 0 {
        if *shown == 2 { for mut window in &mut main { window.set_maximized(true); } }
        if *shown <= 2 { *shown += 1; }
        return;
    }
    if parts.is_empty() { *shown = 3; return; }
    // The first frames are slow while the renderer warms up: the clock
    // starts once they are through, so the splash is seen for its full time.
    let now = time.elapsed_seconds_f64();
    clock.0 += 1;
    if clock.0 <= 3 { clock.1 = now; }
    let dismissed = mouse.get_just_pressed().next().is_some() || keys.get_just_pressed().next().is_some();
    if now - clock.1 < SPLASH_HOLD && !dismissed { return; }
    for part in &parts { commands.entity(part).despawn_recursive(); }
    for mut window in &mut main { window.visible = true; }
    *shown = 1;
}

// ── UI ────────────────────────────────────────────────────────────────────────

fn dcc_ui(
    mut contexts:   EguiContexts,
    mut graph:      ResMut<NodeGraphState>,
    mut stack:      ResMut<OperatorStack>,
    mut hierarchy:  ResMut<SceneHierarchy>,
    mut subnets:    ResMut<SubnetStore>,
    mut nav:        ResMut<GraphNavigation>,
    mut vp_rect:    ResMut<ViewportRect>,
    mut prim_state: ResMut<PrimInspectorState>,
    mut playback:   ResMut<Playback>,
    mut browser:    ResMut<FileBrowser>,
    mut graph_file: ResMut<GraphFile>,
    mut poly_tool:  ResMut<modelling::PolyTool>,
    mut nav_settings: ResMut<viewport::nav::NavSettings>,
    (mut layout, revision, mut uv_state, mut history): (ResMut<Layout>, Res<node_graph::GraphRevision>, ResMut<uv_editor::UvEditorState>, ResMut<history::History>),
    batch:          Res<BatchState>,
    time:           Res<Time>,
) {
    let ctx = contexts.ctx_mut();

    // ── Path chosen in the file browser ──────────────────────────────────────
    if let Some((target, path)) = browser.take_result() {
        let text = path.to_string_lossy().to_string();
        match target {
            BrowseTarget::Node(id) => {
                if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == id) {
                    match &mut node.node_type {
                        NodeType::LoadUsd { path } | NodeType::LoadFbx { path, .. } | NodeType::LoadFbxMesh { path } => *path = text,
                        NodeType::LoadFbxDir { dir, index, .. } => { *dir = text; *index = 0; }
                        // A folder was picked: keep the file name pattern.
                        NodeType::WriteFbx { path } => {
                            let name = path.rsplit(['/', '\\']).next().filter(|n| !n.is_empty())
                                .unwrap_or("{file}_{char}.fbx").to_string();
                            *path = std::path::Path::new(&text).join(name).to_string_lossy().to_string();
                        }
                        _ => {}
                    }
                }
            }
            BrowseTarget::OpenGraph => {
                graph_file.message = match graph_io::load(&mut graph, &path) {
                    Ok(())  => {
                        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        history::note(format!("Open {name}"));
                        format!("Opened {}", path.display())
                    }
                    Err(e)  => format!("Could not open {}: {e}", path.display()),
                };
            }
            BrowseTarget::OpenLayout => {
                graph_file.message = match layout.load_from(&path) {
                    Ok(())  => format!("Layout loaded from {}", path.display()),
                    Err(e)  => format!("Could not load layout {}: {e}", path.display()),
                };
            }
            BrowseTarget::SaveLayout => {
                graph_file.message = match layout.save_as(&path) {
                    Ok(())  => format!("Layout saved to {}", path.display()),
                    Err(e)  => format!("Could not save layout {}: {e}", path.display()),
                };
            }
            BrowseTarget::SaveGraph => {
                graph_file.message = match graph_io::save(&graph, &path) {
                    Ok(())  => format!("Saved {}", path.display()),
                    Err(e)  => format!("Could not save {}: {e}", path.display()),
                };
            }
        }
    }

    // Range, rate and timecode come from the clip of the selected node.
    let timeline_state = resolve_source(&graph);
    let keys_free      = !ctx.wants_keyboard_input() && !browser.is_open();

    // Clips around the selected node, for the properties panel.
    let selected_input = graph.selected_node
        .and_then(|id| graph.nodes.iter().find(|n| n.id == id))
        .and_then(|n| n.inputs.first())
        .and_then(|i| i.connected_output)
        .and_then(|(src, out)| graph.eval_anim_out(src, out));
    let anim_ctx = AnimContext {
        time:   playback.time,
        output: match &timeline_state {
            TimelineState::Source(s) if s.from_selection => Some(s.clip.clone()),
            _ => None,
        },
        input:  selected_input,
    };

    // ── Panes ────────────────────────────────────────────────────────────────
    // Every pane is a tab of one dock area: drag a tab to move it, stack it
    // with another, or pull it out as a window.
    vp_rect.0 = None;   // set again by the viewport pane if it is showing
    let locked = layout.locked;
    // ── Top bar ──────────────────────────────────────────────────────────────
    // Fixed line above the panes, for things that belong to the whole
    // program. It is not a pane: it cannot be moved, floated or closed.
    egui::TopBottomPanel::top("top_bar")
        .exact_height(26.0)
        .resizable(false)
        .frame(egui::Frame::none().fill(theme::c(58, 58, 58)).inner_margin(egui::Margin::symmetric(8.0, 3.0)))
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                draw_logo(ui);
                ui.label(egui::RichText::new("XMS | Imago").strong().color(theme::c(243, 243, 243)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Right to left: the lock sits at the far right, the menu before it.
                    {
                        let tip = if locked {
                            "Layout locked: panes cannot be moved, floated or closed. Dividers still resize. Click to unlock"
                        } else {
                            "Lock the layout"
                        };
                        if lock_button(ui, locked).on_hover_text(tip).clicked() { layout.locked = !locked; }
                    }
                        ui.menu_button("Panes ⏷", |ui| {
                            for pane in Pane::ALL {
                                let mut open = layout.is_open(pane);
                                let label = if layout.is_floating(pane) { format!("{} (floating)", pane.title()) } else { pane.title().to_string() };
                                if ui.add_enabled(!locked, egui::Checkbox::new(&mut open, label)).changed() {
                                    if open { layout.show(pane); } else { layout.hide(pane); }
                                }
                            }
                            ui.separator();
                            if ui.add_enabled(!locked && layout.any_floating(), egui::Button::new("Dock floating panes")).clicked() {
                                layout.dock_all();
                                ui.close_menu();
                            }
                            if ui.add_enabled(!locked, egui::Button::new("Reset layout")).clicked() {
                                layout.reset();
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Save layout...").on_hover_text("Write this layout to a file").clicked() {
                                browser.open(BrowseTarget::SaveLayout, BrowseMode::Save, "Save layout", &["json"], "layout.json");
                                ui.close_menu();
                            }
                            if ui.add_enabled(!locked, egui::Button::new("Load layout...")).on_hover_text("Replace this layout with one from a file").clicked() {
                                browser.open(BrowseTarget::OpenLayout, BrowseMode::File, "Load layout", &["json"], "");
                                ui.close_menu();
                            }
                            if locked { ui.label(egui::RichText::new("Unlock the layout to change it.").small()); }
                        });
                        ui.menu_button("Theme ⏷", theme::menu);
                });
            });
        });
    theme::editor(ctx);

    let mut toggle_float = None;
    let mut dock_rect = ctx.screen_rect();
    let mut tab_pressed = None;
    {
        let mut panes = Panes {
            graph: &mut graph, stack: &mut stack, hierarchy: &mut hierarchy, subnets: &mut subnets,
            nav: &mut nav, vp_rect: &mut vp_rect, prim_state: &mut prim_state, playback: &mut playback,
            browser: &mut browser, graph_file: &mut graph_file, poly_tool: &mut poly_tool,
            batch: &batch, anim_ctx: &anim_ctx, timeline_state: &timeline_state,
            dt: time.delta_seconds_f64(), keys_free,
            space_plays: nav_settings.style != viewport::nav::NavStyle::Houdini,
            revision: revision.0, locked, toggle_float: &mut toggle_float, tab_pressed: &mut tab_pressed,
            uv_state: &mut uv_state, history: &mut history,
        };
        let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());
        style.tab_bar.fill_tab_bar = true;
        // Tab titles: readable when idle, brightest on the tab in front.
        style.tab.inactive.text_color = theme::c(223, 223, 223);
        style.tab.hovered.text_color  = theme::c(250, 250, 250);
        style.tab.active.text_color   = theme::c(250, 250, 250);
        style.tab.focused.text_color  = theme::c(245, 245, 245);
        layout.size_new_windows();
        // What is left under the top bar.
        let screen = ctx.available_rect();
        dock_rect = screen;
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            // Locked: tabs stay where they are. Dividers resize either way.
            egui_dock::DockArea::new(&mut layout.dock)
                .style(style)
                .window_bounds(screen)
                .draggable_tabs(!locked)
                .show_close_buttons(!locked)
                // A floating window is closed by the cross on its tab.
                .show_window_close_buttons(false)
                .show_window_collapse_buttons(false)
                .tab_context_menus(!locked)
                .show_inside(ui, &mut panes);
        });
    }

    if let Some(pane) = toggle_float { layout.toggle_float(pane); }

    // ── Docking along a whole edge of the window ─────────────────────────────
    // The dock area only offers places beside single panes. While a tab is
    // dragged, the four edges of the window are drop places too: the pane
    // then spans that whole side, as the timeline does by default.
    // A drag is the pointer travelling, not a long press: holding a tab
    // still never docks it.
    let (down, moving, let_go) = ctx.input(|i| {
        let travelled = match (i.pointer.press_origin(), i.pointer.interact_pos()) { (Some(a), Some(b)) => a.distance(b) > 8.0, _ => false };
        (i.pointer.primary_down(), i.pointer.is_decidedly_dragging() && travelled, i.pointer.primary_released())
    });
    if let Some(pane) = tab_pressed { if layout.tab_drag.is_none() { layout.tab_drag = Some((pane, false)); } }
    if let Some((pane, moved)) = layout.tab_drag {
        let released = let_go || !down;
        if released { layout.tab_drag = None; } else if moving { layout.tab_drag = Some((pane, true)); }
        let moved = moved || moving;
        let screen = dock_rect;
        let edge = ctx.input(|i| i.pointer.interact_pos()).and_then(|p| layout::Edge::near(screen, p, 26.0));
        if !moved {
            // A click on a tab, not a drag.
        } else if released {
            if let Some(edge) = edge { layout.dock_to_edge(pane, edge); }
        } else {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("edge_dock")));
            let tint = egui::Color32::from_rgb(120, 170, 255);
            for e in [layout::Edge::Left, layout::Edge::Right, layout::Edge::Top, layout::Edge::Bottom] {
                // A thin bar marks each edge; the one under the cursor shows where the pane will land.
                let mut bar = screen;
                match e {
                    layout::Edge::Left   => bar.set_right(screen.left() + 5.0),
                    layout::Edge::Right  => bar.set_left(screen.right() - 5.0),
                    layout::Edge::Top    => bar.set_bottom(screen.top() + 5.0),
                    layout::Edge::Bottom => bar.set_top(screen.bottom() - 5.0),
                }
                painter.rect_filled(bar, 0.0, tint.gamma_multiply(0.6));
                if edge == Some(e) {
                    painter.rect_filled(e.strip(screen), 0.0, tint.gamma_multiply(0.35));
                    painter.rect_stroke(e.strip(screen).shrink(1.0), 0.0, egui::Stroke::new(2.0_f32, tint));
                }
            }
        }
    }

    // Not while a button is down: a drag changes the layout on every frame.
    if !ctx.input(|i| i.pointer.any_down()) { layout.save_if_changed(); }

    // ── File browser (on top of everything) ───────────────────────────────────
    browser.show(ctx);

    // ── Viewport navigation menu and help ─────────────────────────────────────
    let Some(vp) = vp_rect.0 else { return };
    let before = *nav_settings;
    let mut chosen = before;
    // The key list is hidden until the question mark is clicked.
    let help_id = egui::Id::new("viewport_nav_help");
    let mut nav_help = ctx.data(|d| d.get_temp::<bool>(help_id)).unwrap_or(false);
    // Background order: floating panes go over the menu, not under it.
    egui::Area::new("viewport_nav_menu".into())
        .order(egui::Order::Background)
        .fixed_pos(vp.min + egui::vec2(10.0, 8.0))
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(theme::c(90, 90, 90).gamma_multiply(0.92))
                .stroke(egui::Stroke::new(1.0_f32, theme::c(70, 70, 70)))
                .rounding(4.0)
                .inner_margin(3.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.menu_button(format!("Navigation: {} ⏷", chosen.style.label()), |ui| {
                            for style in viewport::nav::NavStyle::ALL {
                                if ui.radio_value(&mut chosen.style, style, style.label()).clicked() { ui.close_menu(); }
                            }
                            ui.separator();
                            ui.checkbox(&mut chosen.invert_zoom, "Invert zoom drag");
                        });
                        if ui.selectable_label(nav_help, "?").on_hover_text("Show the navigation keys").clicked() {
                            nav_help = !nav_help;
                        }
                        let shown = viewport::display::mode();
                        ui.menu_button(format!("{} ⏷", shown.label()), |ui| {
                            for m in viewport::display::DisplayMode::ALL {
                                if ui.radio(shown == m, m.label()).clicked() { viewport::display::set_mode(m); ui.close_menu(); }
                            }
                        }).response.on_hover_text("How meshes are drawn");
                        let loading = viewport::textures::pending();
                        if loading > 0 {
                            ui.label(egui::RichText::new(format!("loading {loading}")).small());
                            ui.ctx().request_repaint();
                        }
                    });
                });
        });
    if chosen != before {
        *nav_settings = chosen;
        chosen.save();
    }

    ctx.data_mut(|d| d.insert_temp(help_id, nav_help));
    if nav_help {
    egui::Area::new("viewport_label".into())
        .order(egui::Order::Background)
        .fixed_pos(vp.min + egui::vec2(10.0, 40.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(theme::c(90, 90, 90).gamma_multiply(0.86))
                .stroke(egui::Stroke::new(1.0_f32, theme::c(70, 70, 70)))
                .rounding(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    let play = if chosen.style == viewport::nav::NavStyle::Houdini { "Tap Space: play  |  Left/Right: step" } else { "Space: play  |  Left/Right: step" };
                    let nav_lines = chosen.style.help();
                    for line in nav_lines.iter().chain(["Scroll: zoom", "F: focus", play].iter()) {
                        ui.label(egui::RichText::new(*line)
                            .color(egui::Color32::from_rgb(190, 190, 190)));
                    }
                });
        });
    }
}

/// The program's mark: two crossing wires with a node at each end, for the X
/// of XMS and the node graph it is built on.
fn draw_logo(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    // The program icon, loaded once and kept by egui.
    let id = egui::Id::new("logo_texture");
    let texture = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).or_else(|| {
        let (rgba, w, h) = decode_png(include_bytes!("../assets/icon.png"))?;
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        let texture = ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    });
    if let Some(texture) = texture {
        ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    response
}

/// Padlock button, drawn by hand: closed when locked, shackle swung open
/// when not. The emoji font's padlock is unreadable at this size.
fn lock_button(ui: &mut egui::Ui, locked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 18.0), egui::Sense::click());
    let painter = ui.painter();
    if locked || response.hovered() {
        let fill = if locked { theme::c(80, 95, 115) } else { theme::raised(100, 100, 100) };
        painter.rect_filled(rect, 3.0, fill);
    }
    let ink = if locked { egui::Color32::from_rgb(240, 240, 240) } else { theme::c(233, 233, 233) };
    let c = rect.center();
    let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 3.0), egui::vec2(10.0, 7.0));
    painter.rect_filled(body, 1.5, ink);
    // Shackle: a half ring above the body. Open: one leg lifted clear of it.
    let (r, top) = (3.0, body.top() - 3.0);
    let lift = if locked { 0.0 } else { 3.0 };
    let mut pts = vec![egui::pos2(c.x - r, body.top())];
    for k in 0..=8 {
        let a = std::f32::consts::PI * (1.0 - k as f32 / 8.0);
        pts.push(egui::pos2(c.x + r * a.cos(), top - r * a.sin() * 0.9));
    }
    pts.push(egui::pos2(c.x + r, body.top() - lift));
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.6_f32, ink)));
    response
}

/// Everything the panes draw from, borrowed for one frame.
struct Panes<'a> {
    graph:          &'a mut NodeGraphState,
    stack:          &'a mut OperatorStack,
    hierarchy:      &'a mut SceneHierarchy,
    subnets:        &'a mut SubnetStore,
    nav:            &'a mut GraphNavigation,
    vp_rect:        &'a mut ViewportRect,
    prim_state:     &'a mut PrimInspectorState,
    playback:       &'a mut Playback,
    browser:        &'a mut FileBrowser,
    graph_file:     &'a mut GraphFile,
    poly_tool:      &'a mut modelling::PolyTool,
    batch:          &'a BatchState,
    anim_ctx:       &'a AnimContext,
    timeline_state: &'a TimelineState,
    dt:             f64,
    keys_free:      bool,
    space_plays:    bool,
    revision:       u64,
    locked:         bool,
    /// Pane whose tab was double-clicked this frame.
    toggle_float:   &'a mut Option<Pane>,
    /// Pane whose tab the mouse button went down on this frame.
    tab_pressed:    &'a mut Option<Pane>,
    uv_state:       &'a mut uv_editor::UvEditorState,
    history:        &'a mut history::History,
}

impl egui_dock::TabViewer for Panes<'_> {
    type Tab = Pane;

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText { tab.title().into() }
    // Closing hides the pane. The Panes menu shows it again.
    fn closeable(&mut self, _tab: &mut Pane) -> bool { !self.locked }
    // Double-click a tab to float it, or to dock it back.
    fn on_tab_button(&mut self, tab: &mut Pane, response: &egui::Response) {
        if self.locked { return; }
        if response.double_clicked() { *self.toggle_float = Some(*tab); }
        // The dock area takes over the tab while it is dragged, so only the
        // press is seen here. The drag is followed from the pointer.
        if response.is_pointer_button_down_on() { *self.tab_pressed = Some(*tab); }
    }
    // Panes scroll their own content where they need to.
    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] { [false, false] }
    // The 3D view is drawn by the camera behind the interface.
    fn clear_background(&self, tab: &Pane) -> bool { *tab != Pane::Viewport }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::Viewport => {
                self.vp_rect.0 = Some(ui.max_rect());
            }

            Pane::Timeline => {
                draw_timeline(ui, self.playback, self.timeline_state, self.dt, self.keys_free, self.space_plays);
            }

            Pane::Properties => {
                // A scroll area that never shrinks keeps the pane at the size
                // it was given: text wraps, anything still too big scrolls.
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    let poly_input = self.graph.selected_node.and_then(|id| {
                        let subnets = &*self.subnets;
                        let eval = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
                            subnets.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
                        };
                        let is_edit_poly = self.graph.nodes.iter()
                            .any(|n| n.id == id && matches!(n.node_type, NodeType::EditPoly { .. }));
                        if is_edit_poly { modelling::input_mesh(self.graph, id, &eval) } else { None }
                    });
                    let mut tool = self.poly_tool.tool;
                    let mut io = PanelIo { browser: self.browser, batch: self.batch, action: None, poly_input, tool: &mut tool };
                    draw_properties_panel(ui, self.graph, &*self.stack, self.subnets, self.nav, self.anim_ctx, &mut io);
                    if let Some(PanelAction::Write { targets, all_files }) = io.action {
                        batch::start(self.batch, self.graph, targets, all_files);
                    }
                    if tool != self.poly_tool.tool { self.poly_tool.tool = tool; }
                });
            }

            Pane::SceneExplorer => {
                egui::ScrollArea::both().id_source("scene_explorer_scroll").auto_shrink([false, false])
                    .show(ui, |ui| draw_scene_explorer(ui, self.hierarchy, self.graph));
            }

            Pane::History => {
                history::draw(ui, self.history, self.graph.selected_node);
            }

            Pane::OperatorStack => {
                egui::ScrollArea::both().id_source("operator_stack_scroll").auto_shrink([false, false])
                    .show(ui, |ui| draw_operator_stack(ui, self.stack, self.graph));
            }

            Pane::UvEditor => {
                let subnets = &*self.subnets;
                let get_mesh = |g: &NodeGraphState| -> Option<MeshData> {
                    let id = g.selected_node?;
                    let eval = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
                        subnets.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
                    };
                    g.eval_node(id, &mut std::collections::HashMap::new(), &eval).map(|r| r.into_mesh())
                };
                uv_editor::draw_uv_editor(ui, self.graph, self.uv_state, self.revision, &get_mesh);
            }

            Pane::PrimInspector => {
                let get_mesh = |g: &NodeGraphState| -> Option<MeshData> {
                    let id = g.selected_node?;
                    let mut cache = std::collections::HashMap::new();
                    let eval_subnet = |_sid: SubnetId, mesh: &MeshData, _template: Option<&MeshData>| mesh.clone();
                    g.eval_node(id, &mut cache, &eval_subnet).map(|r| r.into_mesh())
                };
                let get_clip = |g: &NodeGraphState| g.selected_node.and_then(|id| g.eval_anim(id));
                draw_prim_inspector(ui, self.graph, self.prim_state, self.revision, self.playback.time, &get_mesh, &get_clip);
            }

            Pane::NodeGraph => match self.nav.current_subnet {
                Some(sid) => {
                    if let Some(sg) = self.subnets.get_mut(sid) {
                        let subnet_name = sg.name.clone();
                        if draw_breadcrumb(ui, &subnet_name) {
                            self.nav.current_subnet = None;
                        } else {
                            ui.label("Right-click: add  |  Shift+drag: pan  |  Esc: cancel wire");
                            ui.separator();
                            // Canvas in its own child Ui: see the note below.
                            let rect = ui.available_rect_before_wrap();
                            let mut canvas = ui.child_ui(rect, *ui.layout(), None);
                            canvas.set_clip_rect(rect.intersect(ui.clip_rect()));
                            draw_subnet_graph(&mut canvas, sg);
                            ui.allocate_rect(rect, egui::Sense::hover());
                        }
                    } else {
                        self.nav.current_subnet = None;
                    }
                }
                None => {
                    // Wraps when the pane is narrow.
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("📂 Open").on_hover_text("Load a saved graph").clicked() {
                            self.browser.open(BrowseTarget::OpenGraph, BrowseMode::File, "Open graph", &["json"], "");
                        }
                        if ui.button("💾 Save").on_hover_text("Save this graph").clicked() {
                            self.browser.open(BrowseTarget::SaveGraph, BrowseMode::Save, "Save graph", &["json"], "graph.json");
                        }
                        if ui.button("Tidy").on_hover_text("Lay the nodes out in rows, top to bottom, and bring them into view").clicked() {
                            self.graph.auto_layout();
                        }
                        if ui.button("Frame").on_hover_text("Bring every node into view (F or A over the graph)").clicked() {
                            self.graph.frame_request = true;
                        }
                        ui.small_button("?").on_hover_text(
                            "Right-click or Tab: add a node\nShift+drag: pan\nEsc: cancel a wire\nDouble-click a subnet: dive in\nRight-click an output: add a node under it, wired\nF or A: frame every node\nRing at the left of a node: bypass\nEye at the right: show in the viewport");
                        // Ready-made graphs. Picking one replaces the current graph.
                        ui.menu_button("Templates", |ui| {
                            for group in templates::GROUPS {
                                ui.menu_button(group, |ui| {
                                    for t in templates::TEMPLATES.iter().filter(|t| t.group == group) {
                                        if ui.button(t.name).on_hover_text(t.hint).clicked() {
                                            self.graph_file.message = t.load(self.graph, self.subnets);
                                            history::note(format!("Template: {}", t.name));
                                            self.nav.current_subnet = None;
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }
                        });
                    });
                    if !self.graph_file.message.is_empty() {
                        ui.label(egui::RichText::new(&self.graph_file.message).small().color(theme::c(230, 230, 230)));
                    }
                    ui.separator();

                    // The canvas gets its own child Ui. Nodes are widgets placed
                    // at arbitrary positions; drawn straight into the pane, a
                    // node dragged or panned past the edge would stretch it.
                    let rect = ui.available_rect_before_wrap();
                    let mut canvas = ui.child_ui(rect, *ui.layout(), None);
                    canvas.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let dive = draw_node_graph(&mut canvas, self.graph);
                    ui.allocate_rect(rect, egui::Sense::hover());

                    for node in self.graph.nodes.iter_mut() {
                        if let NodeType::Subnet { id, name } = &mut node.node_type {
                            if *id == SubnetId(usize::MAX) {
                                let new_id = self.subnets.create_subnet(name.clone());
                                *id = new_id;
                            }
                        }
                    }
                    if let Some(sid) = dive {
                        if sid != SubnetId(usize::MAX) {
                            self.nav.current_subnet = Some(sid);
                        }
                    }
                }
            },
        }
    }
}

/// Undo history: turn the changes of this frame into a step once the
/// gesture making them is over, and carry out undo, redo and jumps.
/// Ctrl+Z undoes, Ctrl+Shift+Z or Ctrl+Y redoes (Cmd on macOS), except while
/// a text field has the keyboard: there they edit the text.
fn history_system(
    mut contexts: EguiContexts,
    mut graph:    ResMut<NodeGraphState>,
    mut history:  ResMut<history::History>,
    mut nav:      ResMut<GraphNavigation>,
    mouse:        Res<ButtonInput<MouseButton>>,
    browser:      Res<FileBrowser>,
) {
    let ctx = contexts.ctx_mut();
    let typing = ctx.wants_keyboard_input();
    if !typing && !browser.is_open() {
        let (redo, undo) = ctx.input_mut(|i| {
            use egui::{Key, Modifiers};
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
            let undo = !redo && i.consume_key(Modifiers::COMMAND, Key::Z);
            (redo, undo)
        });
        if redo { history.request = Some(history::Request::Redo); }
        if undo { history.request = Some(history::Request::Undo); }
    }
    let busy = typing || browser.is_open()
        || mouse.get_pressed().next().is_some()
        || ctx.is_using_pointer()
        || ctx.input(|i| i.pointer.any_down());
    // Look without touching: only an actual change marks the graph changed.
    if history.update(graph.bypass_change_detection(), busy) {
        graph.set_changed();
        // Leave a subnet whose node is gone.
        if let Some(sid) = nav.current_subnet {
            let there = graph.nodes.iter().any(|n| matches!(&n.node_type, NodeType::Subnet { id, .. } if *id == sid));
            if !there { nav.current_subnet = None; }
        }
    }
}

/// Bump the graph revision when the graph's content changed this frame.
/// Cooking systems run after this and only when the revision moved.
fn track_revision(
    graph:        Res<NodeGraphState>,
    nav:          Res<GraphNavigation>,
    mouse:        Res<ButtonInput<MouseButton>>,
    keys:         Res<ButtonInput<KeyCode>>,
    mut revision: ResMut<node_graph::GraphRevision>,
    mut last:     Local<Option<u64>>,
) {
    let hash = graph.content_hash();
    // Subnet contents are not part of the hash. While one is open, any
    // click or key press counts as a change.
    let in_subnet = nav.current_subnet.is_some()
        && (mouse.get_pressed().next().is_some() || mouse.get_just_released().next().is_some()
            || keys.get_just_pressed().next().is_some());
    // A solve that finished on its thread changes what its node puts out.
    if *last != Some(hash) || in_subnet || ragdoll::take_changed() {
        *last = Some(hash);
        revision.0 = revision.0.wrapping_add(1);
    }
}

/// Feedback line for graph open / save.
#[derive(Resource, Default)]
struct GraphFile {
    message: String,
}

// ── Systems ───────────────────────────────────────────────────────────────────

fn update_operator_stack(
    graph:     Res<NodeGraphState>,
    revision:  Res<node_graph::GraphRevision>,
    mut stack: ResMut<OperatorStack>,
) {
    if revision.is_changed() {
        stack.rebuild(&graph.nodes, &graph.connections);
        if let Some(sel) = graph.selected_node {
            stack.selected_entry = Some(sel);
        }
    }
}

fn update_scene_hierarchy(
    graph:         Res<NodeGraphState>,
    subnets:       Res<SubnetStore>,
    revision:      Res<node_graph::GraphRevision>,
    mut hierarchy: ResMut<SceneHierarchy>,
) {
    if !revision.is_changed() { return; }

    // Update the closure to accept template parameter
    let eval_subnet = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
        subnets
            .get(sid)
            .map(|sg| sg.evaluate(mesh, template))
            .unwrap_or_else(|| mesh.clone())
    };

    let entries = graph.evaluate_for_scene(&eval_subnet);
    hierarchy.rebuild(entries);
}

fn update_generated_meshes(
    graph:        Res<NodeGraphState>,
    subnets:      Res<SubnetStore>,
    playback:     Res<Playback>,
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
    query:        Query<Entity, (With<GeneratedMesh>, Without<types::PosedMesh>)>,
    posed:        Query<Entity, With<types::PosedMesh>>,
    revision:     Res<node_graph::GraphRevision>,
    mut shown:    Local<Option<(f64, u64)>>,
    mut images:   ResMut<Assets<Image>>,
    mut textures: Local<std::collections::HashMap<std::path::PathBuf, Handle<Image>>>,
    clear:        Res<ClearColor>,
) {
    use viewport::display::DisplayMode;
    let mode = viewport::display::mode();
    // Cook again when the graph changed, the playhead moved, the theme
    // switched or a texture finished loading; not when the camera moves.
    let now = (playback.time, theme::revision() ^ viewport::textures::generation().rotate_left(32));
    if !revision.is_changed() && *shown == Some(now) { return; }
    // When only the playhead moved, only what follows it is made again. A
    // heavy model standing next to a character is left alone.
    let only_time = !revision.is_changed() && shown.map(|s| s.1) == Some(now.1);
    *shown = Some(now);
    for e in posed.iter() { commands.entity(e).despawn(); }

    // Meshes bound to a skeleton, posed at the playhead.
    for clip in graph.display_clips() {
        let Some(skin) = &clip.skin else { continue };
        let (pos, nrm) = skin.deformed(&clip.world_pose(clip.index_at(playback.time)));
        let mut md = MeshData {
            vertices: pos.iter().map(|p| p.to_array()).collect(),
            normals:  nrm.iter().map(|n| n.to_array()).collect(),
            ..Default::default()
        };
        for f in &skin.faces {
            md.indices.extend([f[0], f[1], f[2]]);
            if !core::anim::SkinMesh::is_tri(f) { md.indices.extend([f[0], f[2], f[3]]); }
        }
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(mesh_data_to_bevy(&md)),
                material: mats.add(StandardMaterial {
                    base_color: Color::srgb(0.55, 0.62, 0.7),
                    perceptual_roughness: 0.6,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
                ..default()
            },
            GeneratedMesh,
            types::PosedMesh,
        ));
    }
    if only_time { return; }
    for e in query.iter() { commands.entity(e).despawn(); }

    let eval_subnet = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
        subnets
            .get(sid)
            .map(|sg| sg.evaluate(mesh, template))
            .unwrap_or_else(|| mesh.clone())
    };

    // A selected Edit Poly node takes over the viewport: it shows the mesh
    // being edited, with the selected polygons tinted.
    let stage = modelling::stage(&graph, &eval_subnet);
    if let Some(hl) = stage.as_ref().and_then(modelling::highlight_mesh) {
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(mesh_data_to_bevy(&hl)),
                material: mats.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 0.25, 0.15, 0.45),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
                ..default()
            },
            GeneratedMesh,
        ));
    }
    // Packed primitives with materials: one mesh per material, textured.
    let packed = if stage.is_none() { graph.evaluate_for_viewport_packed(&eval_subnet) } else { None };
    if let Some(types::EvalResult::Named(prims)) = &packed {
        if mode == DisplayMode::Textured && prims.iter().any(|p| p.look.is_some()) {
            let mut groups: Vec<(Option<std::sync::Arc<types::Look>>, Vec<&MeshData>)> = vec![];
            for p in prims {
                match groups.iter_mut().find(|(look, _)| look.as_ref().map(std::sync::Arc::as_ptr) == p.look.as_ref().map(std::sync::Arc::as_ptr)) {
                    Some((_, list)) => list.push(&p.mesh),
                    None => groups.push((p.look.clone(), vec![&p.mesh])),
                }
            }
            let mut texture = |path: &Option<std::path::PathBuf>| -> Option<(Handle<Image>, bool)> {
                let path = path.as_ref()?;
                let decoded = viewport::textures::request(path)?;
                let handle = textures.entry(path.clone()).or_insert_with(|| images.add(texture_image(&decoded))).clone();
                Some((handle, decoded.has_alpha))
            };
            for (look, parts) in groups {
                let md = node_graph::nodes::merge_all(&parts);
                if md.vertices.is_empty() { continue; }
                let material = match &look {
                    Some(look) => {
                        let color_map = texture(&look.color_map);
                        let emissive_map = texture(&look.emissive_map);
                        let see_through = look.opacity < 0.999;
                        let cut = look.cutout && color_map.as_ref().map(|c| c.1).unwrap_or(false);
                        StandardMaterial {
                            base_color: Color::srgba(look.color[0], look.color[1], look.color[2], look.opacity),
                            base_color_texture: color_map.map(|c| c.0),
                            emissive: if emissive_map.is_some() { LinearRgba::WHITE } else { LinearRgba::BLACK },
                            emissive_texture: emissive_map.map(|c| c.0),
                            perceptual_roughness: look.roughness.clamp(0.089, 1.0),
                            metallic: look.metallic.clamp(0.0, 1.0),
                            alpha_mode: if see_through { AlphaMode::Blend } else if cut { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque },
                            double_sided: true,
                            cull_mode: None,
                            ..default()
                        }
                    }
                    None => StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), metallic: 0.1, perceptual_roughness: 0.5, ..default() },
                };
                commands.spawn((PbrBundle { mesh: meshes.add(mesh_data_to_bevy_textured(&md)), material: mats.add(material), ..default() }, GeneratedMesh));
            }
            return;
        }
    }
    // With a clip that was kept out of a collider, the collider is drawn
    // too, see-through so the character shows inside it.
    let collider = if stage.is_none() && matches!(packed, Some(types::EvalResult::Anim(_))) { graph.display_collider() } else { None };
    let ghostly = collider.is_some();
    let shown: Option<std::sync::Arc<MeshData>> = match (&stage, collider) {
        (Some(s), _) => Some(std::sync::Arc::new(s.mesh.to_mesh())),
        (None, Some(c)) => Some(c),
        (None, None) => packed.map(|r| std::sync::Arc::new(r.into_mesh())),
    };
    if let Some(md) = shown {
        let md = &*md;
        if md.vertices.is_empty() { return; }
        let lines = matches!(mode, DisplayMode::HiddenLine | DisplayMode::Wireframe);
        if lines {
            // Polygon edges as a line mesh, pulled a little towards the
            // camera so they win against the surface they lie on.
            let mut wire = Mesh::new(
                bevy::render::mesh::PrimitiveTopology::LineList,
                bevy::render::render_asset::RenderAssetUsages::default(),
            );
            wire.insert_attribute(Mesh::ATTRIBUTE_POSITION, md.vertices.clone());
            wire.insert_indices(bevy::render::mesh::Indices::U32(viewport::display::wire_edges(&md)));
            let ink = theme::c(225, 225, 225);
            commands.spawn((
                PbrBundle {
                    mesh: meshes.add(wire),
                    material: mats.add(StandardMaterial {
                        base_color: Color::srgb_u8(ink.r(), ink.g(), ink.b()),
                        unlit: true,
                        depth_bias: 8.0,
                        ..default()
                    }),
                    ..default()
                },
                GeneratedMesh,
                bevy::pbr::NotShadowCaster,
            ));
        }
        // Hidden line removal: the surface is drawn in the colour of the
        // background, so it shows nothing but hides the edges behind it.
        let material = match mode {
            DisplayMode::Wireframe => None,
            DisplayMode::HiddenLine => Some(StandardMaterial {
                base_color: clear.0, unlit: true, double_sided: true, cull_mode: None, ..default()
            }),
            _ if ghostly => Some(StandardMaterial {
                base_color: Color::srgba(0.62, 0.66, 0.72, 0.32), alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 0.7, double_sided: true, cull_mode: None, ..default()
            }),
            _ => Some(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), metallic: 0.1, perceptual_roughness: 0.5, ..default() }),
        };
        if let Some(material) = material {
            let mut entity = commands.spawn((
                PbrBundle { mesh: meshes.add(mesh_data_to_bevy(&md)), material: mats.add(material), ..default() },
                GeneratedMesh,
            ));
            if lines { entity.insert((bevy::pbr::NotShadowCaster, bevy::pbr::NotShadowReceiver)); }
        }
    }
}

// ── Skeleton display ──────────────────────────────────────────────────────────
// Draws the clip of the viewed node (view flag, else Output) at the playhead.

fn setup_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line_width = 2.5;
    config.depth_bias = -1.0;   // draw over the ground grid and meshes
}

fn draw_skeleton(
    graph:      Res<NodeGraphState>,
    playback:   Res<Playback>,
    mut gizmos: Gizmos,
) {
    for clip in graph.display_clips() {
        draw_clip_skeleton(&clip, playback.time, &mut gizmos);
    }
}

fn draw_clip_skeleton(clip: &core::anim::AnimData, time: f64, gizmos: &mut Gizmos) {
    if clip.joints.is_empty() { return; }

    let pose = clip.world_pose(clip.index_at(time));
    let pos: Vec<Vec3> = pose.iter().map(|m| m.w_axis.truncate()).collect();

    // Marker size follows the skeleton, so centimetre and metre rigs both read.
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &pos { lo = lo.min(*p); hi = hi.max(*p); }
    let r = ((hi - lo).max_element() * 0.012).max(1e-4);

    let bone_col  = Color::srgb(0.95, 0.82, 0.35);
    let joint_col = Color::srgb(0.95, 0.95, 0.95);
    for (j, joint) in clip.joints.iter().enumerate() {
        match joint.parent {
            // A helper above the bones (a character root at the origin) is
            // not a bone: no link is drawn from it.
            Some(p) => { if clip.joints[p].is_bone { gizmos.line(pos[p], pos[j], bone_col); } }
            None => {
                // Root: small axis tripod showing its orientation.
                let m = pose[j];
                let l = r * 5.0;
                gizmos.line(pos[j], pos[j] + m.x_axis.truncate().normalize_or_zero() * l, Color::srgb(1.0, 0.2, 0.2));
                gizmos.line(pos[j], pos[j] + m.y_axis.truncate().normalize_or_zero() * l, Color::srgb(0.2, 1.0, 0.2));
                gizmos.line(pos[j], pos[j] + m.z_axis.truncate().normalize_or_zero() * l, Color::srgb(0.3, 0.5, 1.0));
            }
        }
        gizmos.sphere(pos[j], Quat::IDENTITY, r, joint_col).resolution(8);
    }
}

fn apply_viewport_rect(
    vp_rect:   Res<ViewportRect>,
    windows:   Query<&Window>,
    mut cam_q: Query<&mut Camera, With<MainCamera>>,
) {
    let Ok(mut cam) = cam_q.get_single_mut() else { return };
    // The viewport pane is hidden behind another tab: draw nothing.
    let Some(rect) = vp_rect.0 else {
        if cam.is_active { cam.is_active = false; }
        return;
    };
    let Ok(window) = windows.get_single() else { return };
    let scale = window.scale_factor();
    let (win_w, win_h) = (window.physical_width(), window.physical_height());
    if win_w < 2 || win_h < 2 { return; }
    if !cam.is_active { cam.is_active = true; }

    // Kept inside the window: a viewport that sticks out is an error for the renderer.
    let x      = ((rect.min.x * scale).max(0.0) as u32).min(win_w - 1);
    let y      = ((rect.min.y * scale).max(0.0) as u32).min(win_h - 1);
    let width  = ((rect.width()  * scale) as u32).clamp(1, win_w - x);
    let height = ((rect.height() * scale) as u32).clamp(1, win_h - y);

    cam.viewport = Some(bevy::render::camera::Viewport {
        physical_position: UVec2::new(x, y),
        physical_size:     UVec2::new(width, height),
        ..default()
    });
}

fn mesh_data_to_bevy(d: &MeshData) -> Mesh {
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    // Polygon meshes get hard edges where faces meet sharply; see `render_buffers`.
    let (positions, normals, indices) = d.render_buffers();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_indices(bevy::render::mesh::Indices::U32(indices));
    m
}

/// A mesh with its texture coordinates. Each triangle corner gets a vertex
/// of its own, since a vertex can have a different UV in each face. Meshes
/// without UVs go through `mesh_data_to_bevy`.
fn mesh_data_to_bevy_textured(d: &MeshData) -> Mesh {
    if d.uvs.len() != d.indices.len() || d.uvs.is_empty() { return mesh_data_to_bevy(d); }
    let mut normals = d.normals.clone();
    if normals.len() != d.vertices.len() {
        let mut with = d.clone();
        with.compute_normals();
        normals = with.normals;
    }
    let corner = |i: &u32| *i as usize;
    let positions: Vec<[f32; 3]> = d.indices.iter().map(|i| d.vertices[corner(i)]).collect();
    let normals: Vec<[f32; 3]> = d.indices.iter().map(|i| normals.get(corner(i)).copied().unwrap_or([0.0, 1.0, 0.0])).collect();
    // USD puts v = 0 at the bottom of the image, the GPU at the top.
    let uvs: Vec<[f32; 2]> = d.uvs.iter().map(|uv| [uv[0], 1.0 - uv[1]]).collect();
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m
}

/// A decoded texture as a GPU image: its smaller copies included, repeating
/// in both directions, filtered between pixels and between sizes.
fn texture_image(d: &viewport::textures::Decoded) -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::texture::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    let mut image = Image::new(
        Extent3d { width: d.width, height: d.height, depth_or_array_layers: 1 },
        TextureDimension::D2, d.pixels[..(d.width * d.height * 4) as usize].to_vec(),
        TextureFormat::Rgba8UnormSrgb, bevy::render::render_asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.data = d.pixels.clone();
    image.texture_descriptor.mip_level_count = d.levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

// ── Scene setup ───────────────────────────────────────────────────────────────

fn setup_scene(
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_xyz(5.0, 5.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        MainCamera,
    ));
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 10000.0, shadows_enabled: true, ..default()
        },
        transform: Transform::from_rotation(
            Quat::from_euler(EulerRot::XYZ, -0.5, 0.5, 0.0)),
        ..default()
    });
    commands.insert_resource(AmbientLight { color: Color::WHITE, brightness: 300.0 });

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(create_grid_mesh(20, 1.0)),
            material: mats.add(StandardMaterial {
                base_color: Color::srgba(0.35, 0.35, 0.35, 0.6),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            }),
            ..default()
        },
        GroundGrid,
    ));

    for (dir, col) in [
        (Vec3::X, Color::srgb(1.0, 0.0, 0.0)),
        (Vec3::Y, Color::srgb(0.0, 1.0, 0.0)),
        (Vec3::Z, Color::srgb(0.0, 0.0, 1.0)),
    ] {
        commands.spawn(PbrBundle {
            mesh: meshes.add(create_axis_mesh(Vec3::ZERO, dir)),
            material: mats.add(StandardMaterial {
                base_color: col, unlit: true, ..default()
            }),
            ..default()
        });
    }
}

fn create_axis_mesh(start: Vec3, end: Vec3) -> Mesh {
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![start.to_array(), end.to_array()]);
    m.insert_indices(bevy::render::mesh::Indices::U32(vec![0, 1]));
    m
}

fn create_grid_mesh(size: usize, spacing: f32) -> Mesh {
    let half = (size as f32 * spacing) / 2.0;
    let mut verts = Vec::new();
    for i in 0..=size {
        let p = i as f32 * spacing - half;
        verts.push([p, 0.0, -half]); verts.push([p, 0.0,  half]);
        verts.push([-half, 0.0, p]); verts.push([ half, 0.0, p]);
    }
    let idx: Vec<u32> = (0..verts.len() as u32).collect();
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    m.insert_indices(bevy::render::mesh::Indices::U32(idx));
    m
}