# dcc-xms, mocap and modelling fork

Fork of [MartinoMadeddu/dcc-xms](https://github.com/MartinoMadeddu/dcc-xms).

## Why this fork

To extend XMS into a mocap ingestion and cleanup tool: FBX import, a viewport, frame rate and timecode handling, renaming and trimming. Polygon modelling is being added alongside it.

Animation is added as nodes, following the node design of the original project. The timeline has no range of its own: it adapts to the data in the selected node.

The work is proposed upstream in [pull request #3](https://github.com/MartinoMadeddu/dcc-xms/pull/3).

![Trim node selected, timeline showing the trimmed range over the incoming clip](docs/xms_anim_timeline.png)

## What is added

Animation
- Clip data type: skeleton, per-frame transforms, rational frame rate, timecode including drop-frame
- FBX import through ufbx: hierarchy and one take, baked per frame, converted to Y-up metres
- Nodes: Load FBX, Test Clip, Rename Joints, Trim Clip, Retime, Set Timecode
- Timeline panel that takes range, rate and timecode from the selected node
- Skeleton drawn in the viewport for the viewed node

Batch and export
- Load FBX Folder: one file out of a folder, picked by index or from a dropdown of file names
- Split Characters: one output per character, by position or by root joint, any number of outputs
- Auto T-Pose: rotations zeroed, root at the origin, optional hip height
- Fix Pose: manual per-joint rotation and position corrections
- Proxy Skin: sphere per bone and cylinder per link, bound to the skeleton
- Write FBX: binary FBX with skeleton, animation, mesh, skin and bind pose. Writes the current file or the whole folder, in the background
- Templates menu with "Mocap split": builds the whole graph for splitting a two-character take into animation and skinned T-pose files for Unreal
- File browser on every path field, reopening in the last folder visited
- Graph open and save as JSON

Modelling
- Edit Poly: one node holding an ordered list of polygon operations, each with its own selection, modelled on the Edit Poly modifier of 3ds Max
- Selection levels: vertex, edge, border, polygon, element. Picked in the viewport (click, box, Ctrl adds, Shift removes) or by rule (all, by normal, in box), with grow, shrink, invert, loop and ring
- Operations: extrude, bevel, inset, bridge, flip, detach, tessellate, connect, remove, cap, weld, collapse, break, delete, transform, make planar, relax, subdivide
- Move, rotate and scale manipulators in the viewport (Q, W, E, R). Each drag is stored as a Transform operation
- Collapsing: operations stay live until collapsed one by one or all at once, or auto-collapse freezes earlier operations whenever one is added. Collapsed operations still replay when the incoming mesh changes

![Edit Poly node: collapsed and live operations, move manipulator on the selected polygons](docs/xms_edit_poly.png)

Viewport
- Navigation menu at the top of the viewport with seven styles: Maya (default), Houdini, XSI, Blender, Max, Modo, Unreal. The choice is kept between sessions. The question mark next to it lists the keys
- Navigation follows the cursor position, so it works through remote desktops and mouse sharing tools

Interface
- Every pane is a movable, dockable tab: drag to rearrange, stack or float. The layout is kept between sessions
- The graph is cooked only when its content changes, not on every frame. The primitive inspector refreshes on graph changes only and draws just the rows in view
- Light theme by default, dark mode on a button
- Animation and mocap nodes sit in their own "Animation & Mocap" sub-menu of the add-node menu
- Panels keep the size they are dragged to, whatever they contain

![Default layout of the dockable panes](docs/xms_layout.png)

## Not done yet

- Undo
- Edit Poly: chamfer, cut and slice, hinge, soft selection, smoothing groups, material IDs, attach
- Navigation styles and Edit Poly viewport interaction have not been tested by hand with a mouse
- Meshes and skinning read from FBX
- Timeline zoom and pan
- Curve cleanup
- ICE subnet contents in saved graphs
- Import into Unreal has not been tested. Written files were checked by reading them back and against a reference script, on one OptiTrack Motive take

## Documentation

[docs/](docs/README.md): interface, Edit Poly, viewport navigation, animation and mocap, builds and releases.

## Download

Binaries for Linux, macOS and Windows are built from every change to `main` and published on the [Releases page](https://github.com/srlegrand/dcc-xms/releases). See [docs/releases.md](docs/releases.md).

## Build

    cargo run --release

On Ubuntu or Debian, first:

    sudo apt install build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-x11-0

---

## Original README

WIP 3d application written in Rust as part of learning the language -
The project currently has big chunks and sometimes entire modules written with AI which will be replaced moving forward.

XMS (Cross-data Manipulation System ) UI sccreengrab
<img width="2055" height="1286" alt="xms_main_ui1" src="https://github.com/user-attachments/assets/33300420-038c-4770-bd21-e664a7c00303" />
