pub mod camera;pub mod nav;

use bevy_egui::egui;

/// True when the cursor is over the 3D viewport itself: inside its pane and
/// with no window, menu or other floating interface above it.
pub fn pointer_in_viewport(ctx: &egui::Context, rect: Option<egui::Rect>, cursor: Option<bevy::math::Vec2>) -> bool {
    let (Some(rect), Some(c)) = (rect, cursor) else { return false };
    let p = egui::pos2(c.x, c.y);
    // The panes are drawn on the background layer. Anything else under the
    // cursor is a window, a menu or an overlay.
    rect.contains(p) && ctx.layer_id_at(p).map(|l| l == egui::LayerId::background()).unwrap_or(true)
}
