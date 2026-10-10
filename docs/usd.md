# USD

Load USD and Write USD are in the "File" sub-menu of the add-node menu. Pick, Prune and Unpack are in "Primitives".

## Nodes

| Node | What it does |
|---|---|
| Load USD | Reads a `.usda`, `.usdc` or `.usdz` file, composed. Gives one packed primitive per mesh, curves or points prim |
| Pick Primitives | Marks the packed primitives whose path matches a pattern as picked |
| Prune Primitives | Removes the packed primitives whose path matches a pattern, or keeps only those |
| Unpack | Merges every packed primitive into one mesh |
| Write USD | Writes what the network changed in its stages as a USD override layer |

## Composed stages

A file is read as a stage, with its composition resolved: sublayers, references, payloads, inherits, specializes and variants, in text and binary layers alike. What you see is what usdview would show.

Every prim keeps what the file says about it. Geometry stays in its own space, as authored, and each primitive is placed by its world transform. Nothing is baked into the points, so what is written back matches the file.

|---|---|
|---|---|
| Formats | Text (`.usda`), binary (`.usdc`) and `.usdz` |
| Composition | Sublayers, references, payloads, inherits, specializes, variants |
| Transforms | The whole `xformOpOrder`, `!resetXformStack!` included |
| Units and axes | Z-up and `metersPerUnit` become a correction on the placement: the points stay as authored |
| Meshes | Points, polygons of any size, left-handed orientation, subdivision scheme |
| Curves | `BasisCurves`: control points, counts, basis (linear, Bézier, B-spline, Catmull-Rom) and wrap, kept as authored |
| Points | `Points`, with their widths |
| Primvars | Every numeric primvar, normals and widths included, with its interpolation and indices |
| Instancing | Native instances and point instancers: each prototype's geometry is held once and shared by every copy |
| Purpose | `render`, `proxy` and `guide`, inherited down the hierarchy |
| Materials | Listed in the node's Properties: colour, roughness, metallic, opacity, and the texture behind each input. UsdPreviewSurface, OpenPBR and Standard Surface |
| Cameras | Listed: projection, focal length, aperture, clipping range, position |
| Skeletons, lights | Listed by name |
| Time samples | Read at the stage's start time |

A file is read once and kept until it changes on disk. A `.usdz` is unpacked beside itself so its textures are files the viewport can read.

The Properties of a Load USD node show the stage: primitive and triangle counts (every instanced copy counted), up axis and unit, a count of prims by type, the materials with their textures, the cameras, skeletons and lights, and a "Not read in full" list for whatever the file holds that is not handled yet.

## Packed primitives

A stage comes in as packed primitives: each mesh, curves or points prim stays a separate piece, with its path, material binding and purpose, and is passed from node to node without being copied. Nothing is merged until a node needs one mesh.

Pick Primitives chooses which pieces the following nodes work on. After a pick:

- Transform moves each picked primitive on its own, by its placement. The geometry is not copied.
- Edit Poly, UV Unwrap, UV Transform and UV Edit on one picked primitive edit it where it is drawn, and it stays itself: same path, same material, same placement. Only what the node changed is replaced: a UV node leaves the points shared with the file.
- Several picked primitives go through Edit Poly and the UV nodes merged, as one primitive in the place of the first.
- Every other primitive passes through untouched.

An ICE node runs its tree on each primitive (or each picked one) in the primitive's own space, as an ICE tree runs on an object in Softimage.

The pattern is a name pattern as described in [Interface](interface.md#name-patterns): comma-separated regular expressions matched against the full prim path, with a "Pick" list of the paths coming in.

![One wheel picked out of a 461,595 triangle model and moved](xms_usd_pick.png)

## Instancing

Native instances and point instancers keep their instancing. The geometry of a prototype is held once and each copy is a placement: a native instance gives one primitive per mesh, under the instance's own path; a point instancer gives one primitive per prototype mesh, holding all of its points.

- Transform moves the placements, not the points, so moving a forest is moving matrices.
- A node that changes geometry (Edit Poly, the UV nodes) on one instance makes that instance a mesh of its own. The other copies stay shared.
- The viewport draws a mesh with many copies in one draw call (GPU instancing), in its material's colour. A mesh with 64 copies or fewer is drawn copy by copy with its full material and textures.

## The Scene Explorer and the viewport

The Scene Explorer lists a stage as its hierarchy: groups, meshes, curves, points, materials, cameras, lights and skeletons, each with its icon. Pruned primitives leave the list, and so do groups left empty.

What is drawn follows the explorer, as in Gaffer:

- A closed prim is drawn as the box around everything below it. The island first shows as one box per root prim.
- An open prim hands the decision to its children.
- A mesh, curves or points prim whose parent is open is drawn as geometry.
- The square at the right of a row draws that prim and everything below it as geometry, however far it is opened.
- "Show all geometry" in the explorer's header draws everything.

Opening and closing does not cook the graph again. The boxes come from the primitives as they reach the viewport, so they follow a Transform or a Prune upstream. A point instancer's box covers every copy, and an open instancer shows one box per prototype.

The **guide**, **proxy** and **render** toggles in the explorer's header choose which purposes are drawn. By default guides are hidden, proxies are shown, and render geometry is shown where its asset has no proxy (where it has one, the proxy stands in for it). A prim with no purpose is always drawn.

Curves are drawn as lines through their basis, points as small crosses sized by their width.

## Materials in the viewport

Packed primitives are drawn with their materials: colour, roughness, metallic, opacity, the colour texture and the emissive texture. Textures packed in a `.usdz` are used from where it was unpacked.

![The example model with its textures](xms_usd_textured.png)

- Texture files are decoded in the background. A surface shows its plain colour until its texture is ready, and "loading N" shows next to the viewport menu meanwhile.
- Images larger than 2048 pixels on a side are reduced to that. Each gets its chain of smaller copies, so distant surfaces do not shimmer. Textures repeat outside the unit square.
- A material with opacity below one is blended. A material whose opacity comes from its colour texture is cut out by that texture's alpha.
- "Textures" in the viewport menu turns all of this off and shows plain grey.

## Write USD

Write USD writes what the network changed as a `.usda` override layer. The layer sublayers the file each stage was read from and holds only opinions over it, so opened on its own (in usdview, rray, or Imago) it gives what Imago shows. It repeats the file's `upAxis` and `metersPerUnit`, which USD reads from the layer that is opened.

Set a path and press **Write**. The panel then says what was written, and lists what could not be, with the reason.

| Changed in Imago | Written |
|---|---|
| A prim moved | Its own transform (`xformOp:transform`), so it still follows its parents |
| Points moved | `points`, and `extent` to match |
| Polygons or curves changed | `faceVertexCounts` and `faceVertexIndices`, `orientation`; for curves their counts, type, basis and wrap |
| Primvars written or added | `primvars:` with their USD type (`texCoord2f`, `color3f`, `normal3f`…), interpolation and indices |
| Attributes lost | Blocked (`= None`), so the file's value is not seen through |
| Several prims merged into one | The whole geometry, in the prim's own space |
| A prim pruned | `active = false` |
| Material binding or purpose changed | `material:binding`, `purpose` |
| A prim made in Imago | A new prim (`def`) |

Only what changed is written: a stage nothing touched gives a layer with its header alone. What changed is found by comparison with the stage as it was loaded (see [Geometry](geometry.md)).

Not written yet, and listed by the panel:

- Changes to prims inside a native instance. USD keeps instance proxies read-only: the prototype has to be edited, or the instance made a prim of its own.
- Changes to a point instancer's prototypes, which go through the instancer's positions, orientations and scales.

Values are written at the current frame, as single values.

## Not done yet

- Write USD: time samples over a frame range, editing inside instances, a flattened single file
- Normal, roughness, metallic and occlusion maps in the viewport
- Looking through a USD camera
- Animation: time samples beyond the start time, skeletal animation, skinning, blend shapes
- Per-face material subsets, creases and holes (kept in the file, not shown), NURBS, volumes, implicit shapes (sphere, cube, cylinder…)
- Nested point instancers
- Nodes to edit materials, cameras or the hierarchy

## Example file

`examples/DeLorean.usdz`: 43 mesh prims, 461,595 triangles, 30 materials, 40 texture files. By VTX (<https://sketchfab.com/VTX_car>), CC BY-NC-SA 4.0. See `examples/CREDITS.md`.
