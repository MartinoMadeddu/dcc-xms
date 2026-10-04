# dcc-xms, mocap fork

Fork of [MartinoMadeddu/dcc-xms](https://github.com/MartinoMadeddu/dcc-xms).

## Why this fork

To extend XMS into a mocap ingestion and cleanup tool: FBX import, a viewport, frame rate and timecode handling, renaming and trimming.

Animation is added as nodes, following the node design of the original project. The timeline has no range of its own: it adapts to the data in the selected node.

The work is proposed upstream in [pull request #3](https://github.com/MartinoMadeddu/dcc-xms/pull/3).

![Trim node selected, timeline showing the trimmed range over the incoming clip](docs/xms_anim_timeline.png)

## What is added

- Clip data type: skeleton, per-frame transforms, rational frame rate, timecode including drop-frame
- FBX import through ufbx: hierarchy and one take, baked per frame, converted to Y-up metres
- Nodes: Load FBX, Test Clip, Rename Joints, Trim Clip, Retime, Set Timecode
- Timeline panel that takes range, rate and timecode from the selected node
- Skeleton drawn in the viewport for the viewed node

## Not done yet

- FBX export, save, undo
- Meshes and skinning from FBX
- Timeline zoom and pan
- Curve cleanup
- Testing with real mocap files (so far: unit tests and two small hand-written ASCII FBX files)

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
