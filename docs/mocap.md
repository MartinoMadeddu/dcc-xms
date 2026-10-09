# Animation and mocap

Animation nodes are in the "Animation & Mocap" sub-menu of the add-node menu. A clip holds a skeleton, per-frame transforms, a rational frame rate and timecode, including drop-frame.

## Timeline

The timeline has no range of its own. It takes range, rate and timecode from the clip of the selected node. Space plays, Left and Right step a frame, Home and End jump to the ends.

## Nodes

| Node | What it does |
|---|---|
| Load FBX | Reads the hierarchy and one take from an FBX file, baked per frame, converted to Y-up metres. A mesh bound to the skeleton comes with it, with its weights |
| Load FBX Folder | One file out of a folder, picked by index or from a dropdown of file names |
| Test Clip | A generated clip for trying things out |
| Rename Joints | Renames joints. Find is a regular expression, picked from the joint list or typed. Replace may use `$1`, `$2` for its groups |
| Trim | Cuts the clip to a range |
| Retime | Changes the frame rate |
| Set Timecode | Sets the start timecode |
| Split Characters | One output per character, by position or by root joint |
| Auto T-Pose | Rotations zeroed, root at the origin, optional hip height |
| Fix Pose | Manual rotation and position corrections. Each correction takes a joint pattern and applies to every joint it matches. An exact joint name applies to that joint only |
| Proxy Skin | A sphere per bone and a cylinder per link, bound to the skeleton |
| Write FBX | Binary FBX with skeleton and animation, and with "Mesh" ticked the mesh, skin and bind pose too. A clip from a file goes out with that file's skeleton |

## Clip tools

In the "Mocap tools" section of the same sub-menu.

| Node | What it does |
|---|---|
| Mirror | Swaps left and right joints and mirrors the motion across X |
| Smooth | Averages rotations, and optionally translations, over a radius in frames |
| In Place | Removes horizontal travel of the root. Optionally keeps the height, or moves the travel to a root joint |
| Blend | Plays the first clip, then the second, with a blend in frames. Can align the second clip to where the first ends |
| Loop | Blends the end of a clip into its start |
| Retarget | Puts the motion of the first input on the skeleton of the second. Joints are matched by the part of the body they are, so a Mixamo take drives an Unreal skeleton; joints with no part (fingers, helpers) by name, ignoring prefixes and namespaces. Bones are aligned in the rest pose, so the two skeletons may rest differently |
| Characterize | Says which joint is which part of a human. See below |
| Time Warp | Changes speed, or reverses |
| Prune Joints | Removes joints whose names match a pattern, with everything below them |
| Floor | Moves the clip so its lowest point sits at a height |

![The Mocap tools template, with the Joint tab of the Primitive Inspector](xms_mocap_tools.png)

![Retarget onto the test skeleton](xms_retarget.png)

Retarget copies rotations and root motion. It has no IK of its own, so feet can slide when proportions differ.

## Skeletons and Characterize

Body Collide and Retarget need to know which joint is the hips, the chest, the neck, the head, and for each side the clavicle, upper arm, forearm, hand, thigh, calf, foot and toe. They read it from the joint names and the hierarchy. A **Characterize** node (Mocap menu) shows what was read and lets any part be set by hand, from a list of the joints coming in; Auto goes back to the names. What it sets travels with the clip to every node after it.

![Characterize on the Body Collide template: an Unreal 5 / MetaHuman skeleton, 20 of 20 parts, twist joints beside the limbs](xms_characterize.png)

A name is cut into words (`mixamorig:LeftForeArm` is left, fore, arm; `Bip001 L UpperArm` is l, upper, arm; `lShldrBend` is l, shldr, bend), the side and the part are read from the words, and the hierarchy decides the rest:

- The main joint of a limb is the highest one of its part above the next part: `upperarm01.L` over `upperarm02.L`, `lShldrBend` over `lShldrTwist`, `lwrist` over `lhand`.
- The hips are where the legs and the chest hang from, named as hips if one such joint is. Daz `hip` rather than `pelvis`, which only carries the legs; MakeHuman `root`.
- The chest is where both arms hang from. The spine is what lies between the hips and the chest, the neck what lies between the chest and the head, however many joints each has.
- Some conventions name joint places, not bones (SMPL, Kinect, some BVH): there the shoulder joint turns the upper arm, the hip joint the thigh, the foot joint the toes. A skeleton with elbow and wrist joints and no forearm is read that way.
- Twist joints are found by their words (twist, roll) and by where they are. In line: between two main joints, as HumanIK roll joints, Daz twist joints, MakeHuman and Rigify numbered segments. Beside: hanging off a main joint next to the limb, as Unreal and MetaHuman twist joints, Character Creator twist bones, HumanIK leaf joints, Biped twist bones.
- Correctives, helpers and controls are left out: `_correctiveRoot`, `_bck`, `_fwd`, `_in`, `_out`, `twistCor`, `ik_`, `ORG-`, `MCH-`, `J_Sec_`, `ShareBone`, end and nub joints.

Read from the names, with tests on each:

| Convention | Examples |
|---|---|
| Unreal 4, Unreal 5 and MetaHuman | `pelvis`, `spine_01` to `spine_05`, `clavicle_l`, `upperarm_l`, `lowerarm_l`, `hand_l`, `thigh_l`, `calf_l`, `foot_l`, `ball_l`, `lowerarm_twist_01_l` beside |
| Mixamo | `mixamorig:Hips`, `LeftShoulder`, `LeftArm`, `LeftForeArm`, `LeftUpLeg`, `LeftLeg`, `LeftToeBase` |
| HumanIK (Maya, MotionBuilder) | `Character1_Hips`, `LeftArmRoll` in line, `LeafLeftArmRoll1` beside |
| 3ds Max Biped | `Bip001 Pelvis`, `Bip001 L UpperArm`, `Bip001 L Forearm`, `Bip001 L Toe0`, `Bip001 LUpArmTwist` beside |
| Blender Rigify | `DEF-spine`, `DEF-upper_arm.L`, `DEF-forearm.L`, `DEF-shin.L`, head found as the end of the trunk |
| Unity humanoid, VRM | `Hips`, `UpperChest`, `LeftUpperArm`, `LeftLowerArm`, `LeftUpperLeg`, `LeftToes`; `leftUpperArm` |
| VRoid | `J_Bip_C_Hips`, `J_Bip_L_UpperArm`, `J_Bip_L_ToeBase` |
| Character Creator | `CC_Base_Hip`, `CC_Base_L_Upperarm`, `CC_Base_NeckTwist01`, `CC_Base_L_ForearmTwist01` beside |
| Daz Genesis 8 | `hip`, `lShldrBend`, `lShldrTwist` in line, `lForearmBend`, `lThighBend`, `lShin` |
| Daz Genesis 9 | `hip`, `l_upperarm`, `l_forearm`, `l_shin`, `l_toes` |
| MakeHuman | `root`, `upperarm01.L`, `upperarm02.L` in line, `wrist.L`, `upperleg01.L` |
| SMPL | `pelvis`, `left_collar`, `left_shoulder`, `left_elbow`, `left_wrist`, `left_hip`, `left_knee`, `left_ankle`, `left_foot` |
| Xsens MVN | `Pelvis`, `L5`, `T8`, `RightShoulder`, `RightUpperArm`, `RightForeArm`, `RightUpperLeg`, `RightToe` |
| OptiTrack Motive | `Hip`, `Ab`, `LShoulder`, `LUArm`, `LFArm`, `LThigh`, `LShin`, `LToe` |
| Rokoko | `hip`, `leftUpperArm`, `leftUpLeg`, `leftLeg`, `leftToe` |
| Apple ARKit | `hips_joint`, `left_shoulder_1_joint`, `left_arm_joint`, `left_upLeg_joint` |
| Source | `ValveBiped.Bip01_Pelvis`, `ValveBiped.Bip01_L_UpperArm`, `ValveBiped.Bip01_L_Toe0` |
| Bandai Namco | `Hips`, `Shoulder_L`, `UpperArm_L`, `LowerArm_L`, `UpperLeg_L`, `Toes_L` |
| Roblox R15 | `LowerTorso`, `UpperTorso`, `LeftUpperArm`, `LeftLowerLeg` |
| Azure Kinect | `PELVIS`, `CLAVICLE_LEFT`, `SHOULDER_LEFT`, `ELBOW_LEFT`, `WRIST_LEFT`, `HIP_LEFT`, `ANKLE_LEFT` |
| CMU | ASF `lhumerus`, `lradius`, `lfemur`, `ltibia`; BVH `lShldr`, `lForeArm`, `lThigh`, `lShin` and `LHipJoint`, `LeftUpLeg` |
| DeepMotion, LaFAN1 | `l_arm_JNT`, `l_upleg_JNT`; `LeftForeArm`, `LeftToe` |

Kinect v2 (`ShoulderLeft`, `ElbowLeft`) is read the same way as Azure Kinect. A skeleton whose names say nothing (`bone_12`) is set by hand: pick the hips and the main joints of each limb, and the spine, neck and twist joints follow from the hierarchy.

## Choosing joints

Joint fields take patterns, described in [Interface](interface.md#name-patterns): comma-separated regular expressions with a picker of the joints coming in. For example `thumb, index, middle, ring, pinky` or `^Left.*(Arm|Hand)$`.

## Joints and bones

With a clip node selected, the Primitive Inspector shows a Joint tab (name, parent, position and rotation at the current frame) and a Bone tab (one row per bone: the joint it starts at, the joint it points to, and its length).

## Writing FBX for an engine

A clip read from a file is written back with the skeleton it came in with, after any process: trims, retimes, mocap tools, a Body Collide solve. Same joint names and hierarchy, same kinds (an FBX "Root" stays a Root, a "LimbNode" a LimbNode), the same axes and unit as the source (Z up in centimetres for a file from Unreal), and the same local values on every joint that was not changed. An engine that imported the source as a skeleton sees the file as an animation of that skeleton.

The properties of the Write node say which space a file goes out in, such as "Z up, cm, as the source file". A clip made in the program (Test Clip) goes out Y up in centimetres.

"Mesh" adds the skinned mesh, its skin and bind pose. Leave it off to import motion onto a skeleton the engine already has: the file then holds the skeleton and its animation only. New Write nodes start with it off; graphs saved before the switch existed keep writing the mesh.

How this is checked: the take of the Body Collide template, written as it ships and written after the shipped solve, is compared with the source FBX read without any conversion. Names, parents, kinds, axes and unit match on all 342 joints. As shipped, the largest difference in a local rotation is 0.003 degrees (the packed clip stores rotations in 16 bits); written straight from the FBX it is 0.00003 degrees, and 0 in position. Import into Unreal itself has not been tried here.

Earlier builds wrote every file Y up, put the source's axis turn and a 0.01 scale on the top joint, and wrote the other joints a hundred times too long, so an engine saw a different skeleton. Rotations within a degree of 90 on Y could also be written half a degree out. Both are fixed.

## Batch export

Write FBX writes the current file or every file of the folder, in the background. The Templates menu has "Mocap split", which builds the whole graph for splitting a two-character take into animation files and skinned T-pose files.

Every path field has a folder button that opens the file browser. It reopens in the last folder visited.

## Graphs

Open, Recent, Save and Save as in the node graph header read and write the graph as JSON. See [Interface](interface.md#files). The contents of ICE subnets are not saved.

## Not done yet

Timeline zoom and pan, curve cleanup, foot planting, IK in Retarget. The human IK pass is part of Body Collide only. Written files are checked against their source, not yet by importing them into Unreal.

## Moving a clip

Transform, the same node that moves meshes, moves a clip: Translate, Rotate and one Scale for the whole skeleton. It moves the joints at the top of the hierarchy and everything under them follows. A mesh skinned to the skeleton is not moved itself: it follows its joints when it is posed, so it moves once, with them. Graphs saved with the old Transform Clip node open with a Transform node in its place, turning the clip the same way.

## Body Collide

Keeping a captured character out of a set and out of itself has a page of its own: [Body Collide](body-collide.md).
