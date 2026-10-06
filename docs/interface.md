# Interface

![Default layout with the Panes menu open and the padlock at the top right](xms_layout.png)

## Panes

Seven panes: Viewport, Node Graph, Scene Explorer, Operator Stack, Properties, Primitive Inspector, Timeline. Each one is a tab.

| To | Do this |
|---|---|
| Move a pane | Drag its tab by the title. Drop it on one of the five squares that appear over the pane under the cursor: the middle one stacks it as a tab, the others split that pane on that side |
| Float a pane | Drag its tab and drop it anywhere that is not one of the squares, or double-click the tab |
| Dock a floating pane | Drag its tab (the title inside the window, not the bar above it) onto one of the squares, or double-click the tab, or use "Dock floating panes" in the Panes menu |
| Move a floating window | Drag the bar at its top |
| Close a pane | The cross on its tab. The cross on a floating window closes every pane in it |
| Show a closed pane | Tick it in the Panes menu, top right. It comes back as a tab of the main area |
| Resize | Drag the line between two panes, or the corner of a floating window |
| Start over | "Reset layout" in the Panes menu |

Right-clicking a tab offers the same close and float actions.

### Lock

The padlock at the top right locks the layout. While it is locked, tabs cannot be moved, floated or closed, and the Panes menu is greyed out. The lines between panes still resize. Click the padlock again to unlock.

### Saved layout

The layout, the lock and the place of every floating window are saved when they change and restored at the next start. They are kept in `layout.json` in the config folder (`~/.config/xms` on Linux, `%APPDATA%\xms` on Windows). A file that cannot be read is ignored and the default layout is used.

When the Viewport pane is closed or hidden behind another tab, the 3D view is not drawn.

## Theme

Light by default. "Dark mode" in the node graph header switches, and the choice is kept.

## Primitive Inspector

A spreadsheet of the mesh of the selected node: one tab per attribute class (Vertex, Uniform, FaceVarying, Constant).

The mesh is cooked and the table built only when the graph changes or another node is selected. Moving the camera, or anything else in the viewport that leaves the graph alone, does not refresh it. Each frame draws only the rows in view.

## When the graph is cooked

The viewport meshes, the scene explorer, the operator stack and the primitive inspector are rebuilt when the content of the graph changes: a node added or removed, a parameter edited, a wire changed, the view flag or the selection moved. Moving nodes about, panning the graph and moving the camera do not cook anything. Animated meshes are also rebuilt when the playhead moves.

While an ICE subnet is open, every click and key press counts as a change.
