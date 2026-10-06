# Animation and mocap

Animation nodes are in the "Animation & Mocap" sub-menu of the add-node menu. A clip holds a skeleton, per-frame transforms, a rational frame rate and timecode, including drop-frame.

## Timeline

The timeline has no range of its own. It takes range, rate and timecode from the clip of the selected node. Space plays, Left and Right step a frame, Home and End jump to the ends.

## Nodes

| Node | What it does |
|---|---|
| Load FBX | Reads the hierarchy and one take from an FBX file, baked per frame, converted to Y-up metres |
| Load FBX Folder | One file out of a folder, picked by index or from a dropdown of file names |
| Test Clip | A generated clip for trying things out |
| Rename Joints | Renames joints |
| Trim Clip | Cuts the clip to a range |
| Retime | Changes the frame rate |
| Set Timecode | Sets the start timecode |
| Split Characters | One output per character, by position or by root joint |
| Auto T-Pose | Rotations zeroed, root at the origin, optional hip height |
| Fix Pose | Manual per-joint rotation and position corrections |
| Proxy Skin | A sphere per bone and a cylinder per link, bound to the skeleton |
| Write FBX | Binary FBX with skeleton, animation, mesh, skin and bind pose |

## Batch export

Write FBX writes the current file or every file of the folder, in the background. The Templates menu has "Mocap split", which builds the whole graph for splitting a two-character take into animation files and skinned T-pose files.

Every path field has a folder button that opens the file browser. It reopens in the last folder visited.

## Graphs

Open and Save in the node graph header read and write the graph as JSON. The contents of ICE subnets are not saved.

## Not done yet

Meshes and skinning read from FBX, timeline zoom and pan, curve cleanup. Import into Unreal has not been tested.
