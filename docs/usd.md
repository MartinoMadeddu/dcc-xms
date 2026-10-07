# USD

USD nodes are in the "USD" sub-menu of the add-node menu.

## Nodes

| Node | What it does |
|---|---|
| Load USD | Reads a `.usda`, `.usdc` or `.usdz` file. Gives one packed primitive per mesh prim |
| Pick Primitives | Marks the packed primitives whose path matches a pattern as picked |
| Prune Primitives | Removes the packed primitives whose path matches a pattern, or keeps only those |
| Unpack | Merges every packed primitive into one mesh |

## Packed primitives

A USD file comes in as packed primitives: each mesh prim stays a separate piece, with its path in the file and its material binding, and is passed from node to node without being copied. Nothing is merged until a node needs one mesh.

Pick Primitives chooses which pieces the following nodes work on. After a pick:

- Edit Poly, Transform, UV Unwrap, UV Transform and UV Edit work on the picked primitives only.
- The picked primitives come out of that node as one primitive, still picked, in the place of the first of them.
- Every other primitive passes through untouched.

So Edit Poly on one wheel of a 460,000 triangle car handles the 10,000 vertices of the wheel, not the car. Without a pick these nodes merge everything and work on the whole, as they do for any mesh.

The pattern is a name pattern as described in [Interface](interface.md#name-patterns): comma-separated regular expressions matched against the full prim path, with a "Pick" list of the paths coming in.

![One wheel picked out of a 461,595 triangle model and moved](xms_usd_pick.png)

## What Load USD reads

| | |
|---|---|
| Formats | Text (`.usda`), binary (`.usdc`), and the root layer of a `.usdz` |
| Transforms | The whole `xformOpOrder`: translate, scale, rotate about one axis, the six three-axis rotations, orient, transform, suffixed ops, inverted ops, `!resetXformStack!` |
| Units and axes | Z-up becomes Y-up. When the layer states `metersPerUnit`, the scene is scaled to metres |
| Meshes | Points, polygons of any size, left-handed winding and mirrored transforms |
| UVs | `primvars:st` and the other usual names, or the first 2D primvar: per vertex or per face corner, indexed or not |
| Materials | Listed in the node's Properties: colour, roughness, metallic, opacity, and the texture file behind each input of a UsdPreviewSurface. Each primitive carries the path of its material |
| Cameras | Listed: projection, focal length, aperture, clipping range, position |
| Skeletons and lights | Listed by name |
| Variants | The selected variant of each variant set, or the first when none is selected (text format) |
| Time samples | Read at their first sample |

A file is read once and kept until it changes on disk.

The Properties of a Load USD node show the stage: primitive and triangle counts, up axis and unit, time range, a count of prims by type, the materials with their textures, the cameras, skeletons and lights, and a "Not read in full" list for whatever the file holds that is not handled yet.

## Not done yet

- Showing materials and textures in the viewport
- Looking through a USD camera
- Animation: time samples beyond the first, skeletal animation, skinning, blend shapes
- Composition across files: references, payloads, sublayers, inherits. Variants in the binary format
- Instancing and point instancers
- Per-face material subsets, curves, points, NURBS, volumes
- Normals from the file (they are computed), and primvars other than UVs
- Writing USD
- Nodes to edit materials, cameras or the hierarchy

## Example file

`examples/DeLorean.usdz`: 43 mesh prims, 461,595 triangles, 30 materials, 40 texture files. By VTX (https://sketchfab.com/VTX_car), CC BY-NC-SA 4.0. See `examples/CREDITS.md`.
