# Files of the Body Collide template

| File | What |
|---|---|
| `into_the_car.xmsclip` | The take: a skinned character of 342 joints and 32,334 vertices, 12,227 frames at 30 fps. An actor walks through a car and sits in it |
| `car.xmsmesh` | The set: the car, 1,743,007 triangles |
| `solved/` | The take solved against the car with the default settings |

The take and the car are Simon Legrand's. As FBX they are 205 MB and 99 MB, more than GitHub takes in one file. Here they are in the program's own compact formats, 4 MB and 15 MB. Joint positions differ from the FBX by less than a tenth of a millimetre.

To make them again from the FBX files:

    XMS_PACK_CLIP=take.fbx XMS_PACK_SET=set.fbx cargo test pack_ragdoll_example -- --ignored --nocapture
