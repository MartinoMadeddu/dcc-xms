# Interface

![Default layout: viewport, primitive inspector, node graph, scene explorer, operator stack, properties, timeline](xms_layout.png)

## Panes

Seven panes: Viewport, Node Graph, Scene Explorer, Operator Stack, Properties, Primitive Inspector, Timeline. Each one is a tab.

- Drag a tab by its title to move it. Drop it on the edge of another pane to split that pane, or on its centre to stack the two as tabs.
- Drag a tab out to make it a floating window. Drag it back onto a pane to dock it again.
- Drag the line between two panes to resize them.
- Panes cannot be closed.

The layout is saved when it changes and restored at the next start. It is kept in `layout.json` in the config folder (`~/.config/xms` on Linux, `%APPDATA%\xms` on Windows). "Reset layout" in the node graph header puts every pane back.

When the Viewport tab is hidden behind another tab, the 3D view is not drawn.

## Theme

Light by default. "Dark mode" in the node graph header switches, and the choice is kept.

## Primitive Inspector

A spreadsheet of the mesh of the selected node: one tab per attribute class (Vertex, Uniform, FaceVarying, Constant).

The mesh is cooked and the table built only when the graph changes or another node is selected. Moving the camera, or anything else in the viewport that leaves the graph alone, does not refresh it. Each frame draws only the rows in view.

## When the graph is cooked

The viewport meshes, the scene explorer, the operator stack and the primitive inspector are rebuilt when the content of the graph changes: a node added or removed, a parameter edited, a wire changed, the view flag or the selection moved. Moving nodes about, panning the graph and moving the camera do not cook anything. Animated meshes are also rebuilt when the playhead moves.

While an ICE subnet is open, every click and key press counts as a change.
