//! Unit tests for the animation bake pipeline (axis-swap, SNORM decoding, time-format 0x42,
//! bone blend-mode classification, clip serialisation correctness).
//!
//! 15 focused tests covering the most regression-prone parts of the animation codec.

    use super::*;
    use super::pose::{quat_mul_wxyz, quat_rotate_vec_wxyz};
    use super::codec::{read_snorm_packed_positions, read_time_keys};

    #[test]
    fn bone_hash_matches_known_values() {
        // Verified externally via Python `zlib.crc32` (case preserved).
        assert_eq!(bone_name_hash("BONE_Back_Right_Foot_Main"), 0xC1571A1A);
    }

    #[test]
    fn quat_mul_identity() {
        let id = [1.0, 0.0, 0.0, 0.0];
        let q = [0.7071068, 0.7071068, 0.0, 0.0];
        let out = quat_mul_wxyz(id, q);
        for i in 0..4 {
            assert!((out[i] - q[i]).abs() < 1e-6, "{:?}", out);
        }
    }

    #[test]
    fn quat_rotate_basis() {
        // 90° about Z (wxyz): w=cos45, z=sin45
        let q = [0.7071068, 0.0, 0.0, 0.7071068];
        let v = [1.0, 0.0, 0.0];
        let r = quat_rotate_vec_wxyz(q, v);
        assert!((r[0] - 0.0).abs() < 1e-5, "{:?}", r);
        assert!((r[1] - 1.0).abs() < 1e-5, "{:?}", r);
        assert!(r[2].abs() < 1e-5, "{:?}", r);
    }

    #[test]
    fn clip_to_json_position_axis_swap_matches_static_import() {
        // Pin the CryEngine Y-up → Blender Z-up axis swap for animation
        // position keyframes. This MUST match the static-import convention
        // used by the addon's `_scene_position_to_blender` in
        // `blender_addon/starbreaker_addon/runtime/importer/utils.py`,
        // which maps (cry_x, cry_y, cry_z) → (cry_x, -cry_z, cry_y). If
        // the two diverge, animation deltas land in a different basis than
        // the bone's bind pose and the result is the inverted X-shape
        // failure documented in `docs/StarBreaker/animation-research.md`
        // (Scorpius wing-deploy kinematics).
        let clip = AnimationClip {
            name: "test_clip".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xDEADBEEF,
                rotations: vec![],
                positions: vec![Keyframe {
                    time: 0.0,
                    value: [1.0, 2.0, 3.0],
                }],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
            start_rotation: None,
            start_position: None,
        };

        let json = clip_to_json(&clip);
        let bones = json["bones"].as_object().unwrap();
        let entry = bones.values().next().unwrap();
        let pos = entry["position"].as_array().unwrap();
        let kf = pos[0].as_array().unwrap();
        assert_eq!(kf[0].as_f64().unwrap(), 1.0, "Blender X must be cry_x");
        assert_eq!(kf[1].as_f64().unwrap(), -3.0, "Blender Y must be -cry_z");
        assert_eq!(kf[2].as_f64().unwrap(), 2.0, "Blender Z must be cry_y");
        let pos_times = entry["position_time"].as_array().unwrap();
        assert_eq!(pos_times[0].as_f64().unwrap(), 0.0, "Position key time must survive JSON export");
    }

    #[test]
    fn clip_to_json_preserves_rotation_times() {
        let clip = AnimationClip {
            name: "timed_clip".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xDEADBEEF,
                rotations: vec![Keyframe {
                    time: 12.5,
                    value: [0.0, 0.0, 0.0, 1.0],
                }],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
            start_rotation: None,
            start_position: None,
        };

        let json = clip_to_json(&clip);
        let bones = json["bones"].as_object().unwrap();
        let entry = bones.values().next().unwrap();
        let rotation_times = entry["rotation_time"].as_array().unwrap();
        assert_eq!(rotation_times[0].as_f64().unwrap(), 12.5);
    }

    #[test]
    fn clip_to_json_preserves_duplicate_hash_channels_as_variants() {
        let hash = 0xDEADBEEF_u32;
        let clip = AnimationClip {
            name: "duplicate_hash_clip".to_string(),
            fps: 30.0,
            channels: vec![
                BoneChannel {
                    bone_hash: hash,
                    rotations: vec![],
                    positions: vec![Keyframe {
                        time: 0.0,
                        value: [1.0, 2.0, 3.0],
                    }],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
                BoneChannel {
                    bone_hash: hash,
                    rotations: vec![],
                    positions: vec![Keyframe {
                        time: 0.0,
                        value: [4.0, 5.0, 6.0],
                    }],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
            ],
            start_rotation: None,
            start_position: None,
        };

        let json = clip_to_json(&clip);
        let key = format!("0x{:X}", hash);
        let entry = &json["bones"][key];
        let variants = entry.as_array().expect("duplicate hash entry must be a variant array");
        assert_eq!(variants.len(), 2, "both channels must be preserved under one hash key");

        let first = variants[0]["position"][0].as_array().expect("first variant position sample");
        let second = variants[1]["position"][0].as_array().expect("second variant position sample");
        // Axis swap check for both samples: (x,y,z) -> (x,-z,y)
        assert_eq!(first[0].as_f64().unwrap(), 1.0);
        assert_eq!(first[1].as_f64().unwrap(), -3.0);
        assert_eq!(first[2].as_f64().unwrap(), 2.0);
        assert_eq!(second[0].as_f64().unwrap(), 4.0);
        assert_eq!(second[1].as_f64().unwrap(), -6.0);
        assert_eq!(second[2].as_f64().unwrap(), 5.0);
    }

    /// Phase 53: clips that originate from DBA metadata expose
    /// `start_rotation` / `start_position` as top-level JSON fields, in
    /// the same Blender Z-up convention as the per-sample emission.
    /// Clips without metadata (e.g. from `parse_caf`) must omit both.
    #[test]
    fn clip_to_json_emits_start_metadata_in_blender_convention() {
        // CryEngine xyzw=(1,2,3,4) → Blender wxyz=(4, 1, -3, 2).
        // CryEngine XYZ=(7,8,9) → Blender XYZ=(7, -9, 8).
        let clip_with_meta = AnimationClip {
            name: "deploy".to_string(),
            fps: 30.0,
            channels: vec![],
            start_rotation: Some([1.0, 2.0, 3.0, 4.0]),
            start_position: Some([7.0, 8.0, 9.0]),
        };
        let json = clip_to_json(&clip_with_meta);
        let sr = json["start_rotation"].as_array().expect("start_rotation");
        assert_eq!(sr[0].as_f64().unwrap(), 4.0, "wxyz[0] = cry_w");
        assert_eq!(sr[1].as_f64().unwrap(), 1.0, "wxyz[1] = cry_x");
        assert_eq!(sr[2].as_f64().unwrap(), -3.0, "wxyz[2] = -cry_z");
        assert_eq!(sr[3].as_f64().unwrap(), 2.0, "wxyz[3] = cry_y");
        let sp = json["start_position"].as_array().expect("start_position");
        assert_eq!(sp[0].as_f64().unwrap(), 7.0, "blender_x = cry_x");
        assert_eq!(sp[1].as_f64().unwrap(), -9.0, "blender_y = -cry_z");
        assert_eq!(sp[2].as_f64().unwrap(), 8.0, "blender_z = cry_y");

        // CAF-style clip omits both fields entirely.
        let clip_caf = AnimationClip {
            name: "caf_clip".to_string(),
            fps: 30.0,
            channels: vec![],
            start_rotation: None,
            start_position: None,
        };
        let json_caf = clip_to_json(&clip_caf);
        assert!(json_caf.get("start_rotation").is_none(), "CAF clips must omit start_rotation");
        assert!(json_caf.get("start_position").is_none(), "CAF clips must omit start_position");
    }

    #[test]
    fn cry_xyzw_to_blender_wxyz_axis_swap_matches_position_swap() {
        // The quaternion's vector component must transform under the same
        // basis change as positions: CryEngine (cx, cy, cz) → Blender
        // (cx, -cz, cy). For an input quaternion (qx=1, qy=2, qz=3, qw=4)
        // the Blender WXYZ form must be (4, 1, -3, 2). If this drifts from
        // the position swap (e.g. picks up the legacy (cy, -cz, cx)
        // convention), animation rotations land in a basis 90° away from
        // their position deltas and the wing-deploy X-shape collapses.
        let q = [1.0_f32, 2.0, 3.0, 4.0]; // CryEngine xyzw
        let blender = cry_xyzw_to_blender_wxyz(q);
        assert_eq!(blender[0], 4.0, "Blender W = cry_w");
        assert_eq!(blender[1], 1.0, "Blender X axis = cry_x axis");
        assert_eq!(blender[2], -3.0, "Blender Y axis = -cry_z axis");
        assert_eq!(blender[3], 2.0, "Blender Z axis = cry_y axis");
    }

    #[test]
    fn sanitize_clip_filename_replaces_unsafe_chars() {
        assert_eq!(sanitize_clip_filename("landing_gear_extend"), "landing_gear_extend");
        assert_eq!(sanitize_clip_filename("Animations/canopy.caf"), "Animations_canopy.caf");
        assert_eq!(sanitize_clip_filename("foo bar/baz\\qux"), "foo_bar_baz_qux");
        assert_eq!(sanitize_clip_filename(""), "clip");
        assert_eq!(sanitize_clip_filename("   "), "clip");
        assert_eq!(sanitize_clip_filename("clip-1.0_v2"), "clip-1.0_v2");
    }

    #[test]
    fn split_clip_for_sidecar_extracts_bones_and_records_sidecar() {
        let clip = AnimationClip {
            name: "landing_gear_extend".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xCAFEBABE,
                rotations: vec![Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] }],
                positions: vec![Keyframe { time: 0.0, value: [1.0, 2.0, 3.0] }],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
            start_rotation: None,
            start_position: None,
        };
        let mut full = clip_to_json(&clip);
        // Mimic fragment annotation by adding a fragments key.
        full["fragments"] = serde_json::json!([{"tags": "Deploy"}]);

        let sidecar_rel = "animations/landing_gear_extend.json";
        let (index, body) = split_clip_for_sidecar(&full, sidecar_rel);

        // Index keeps lightweight metadata + sidecar reference, drops bones.
        assert_eq!(index["name"].as_str().unwrap(), "landing_gear_extend");
        assert_eq!(index["fps"].as_u64().unwrap(), 30);
        assert!(index["frame_count"].is_number());
        assert_eq!(index["sidecar"].as_str().unwrap(), sidecar_rel);
        assert_eq!(index["fragments"], serde_json::json!([{"tags": "Deploy"}]));
        assert!(index.get("bones").is_none(), "Index must not carry bones");

        // Body is the full clip, including bones.
        assert_eq!(body["name"].as_str().unwrap(), "landing_gear_extend");
        assert!(body.get("bones").is_some(), "Sidecar body must carry bones");
        let bones = body["bones"].as_object().unwrap();
        assert_eq!(bones.len(), 1);
    }

    #[test]
    fn classify_bone_blend_modes_marks_outlier_bones_override() {
        // additive bone: bind sits inside the AABB of CAF samples.
        let additive_hash = 0xAAAA_AAAA_u32;
        // override bone: bind is far outside the AABB on at least one axis.
        let override_hash = 0xBBBB_BBBB_u32;
        // bone with no position samples — must be omitted from result.
        let unsampled_hash = 0xCCCC_CCCC_u32;

        let clips = vec![AnimationClip {
            name: "deploy".to_string(),
            fps: 30.0,
            channels: vec![
                BoneChannel {
                    bone_hash: additive_hash,
                    rotations: vec![],
                    positions: vec![
                        Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
                        Keyframe { time: 1.0, value: [1.0, 1.0, 1.0] },
                    ],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
                BoneChannel {
                    bone_hash: override_hash,
                    rotations: vec![],
                    positions: vec![
                        Keyframe { time: 0.0, value: [10.0, 0.0, 0.0] },
                        Keyframe { time: 1.0, value: [11.0, 1.0, 0.0] },
                    ],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
                BoneChannel {
                    bone_hash: unsampled_hash,
                    rotations: vec![Keyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }],
                    positions: vec![],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
            ],
            start_rotation: None,
            start_position: None,
        }];

        let mut binds = std::collections::HashMap::new();
        // Additive bind sits inside the AABB.
        binds.insert(additive_hash, [0.5_f32, 0.5, 0.5]);
        // Override bind sits 8m off the AABB on X.
        binds.insert(override_hash, [2.0_f32, 0.0, 0.0]);
        // Unsampled bone has a bind but no samples — must be omitted.
        binds.insert(unsampled_hash, [0.0_f32, 0.0, 0.0]);

        let modes = classify_bone_blend_modes(&clips, &binds);
        assert_eq!(modes.get(&additive_hash), Some(&BoneBlendMode::Additive));
        assert_eq!(modes.get(&override_hash), Some(&BoneBlendMode::Override));
        assert!(
            !modes.contains_key(&unsampled_hash),
            "Bones without position samples must not be classified"
        );

        // Round-trip through the JSON annotator.
        let mut clips_json =
            database_to_animations_json(&AnimationDatabase { clips: clips.clone() });
        annotate_animations_json_with_blend_modes(&mut clips_json, &modes);
        let bones = clips_json[0]["bones"].as_object().unwrap();
        assert_eq!(
            bones[&format!("0x{additive_hash:X}")]["blend_mode"]
                .as_str()
                .unwrap(),
            "additive"
        );
        assert_eq!(
            bones[&format!("0x{override_hash:X}")]["blend_mode"]
                .as_str()
                .unwrap(),
            "override"
        );
    }

    /// Phase 45 regression: SNORM-packed (`0xC2`) position channels with two
    /// active axes use **planar (axis-major)** layout, not interleaved
    /// (key-major). The decoder previously produced correct results only for
    /// single-active-axis channels (where planar ≡ interleaved); multi-axis
    /// channels (e.g. `Wing_Grabber_Main_Bottom_Right` in Scorpius
    /// `wings_deploy`) were catastrophically misaligned, causing
    /// `BR[i] ≈ BL[2*i]` for the first 22 keys and a flatline thereafter.
    /// See [`docs/StarBreaker/todo.md`] Phase 45 for the byte-level evidence.
    #[test]
    fn snorm_packed_two_active_axes_uses_planar_layout() {
        // Synthesize a 4-key channel with X inactive (FLT_MAX), Y and Z
        // active. Planar layout: [Y0,Y1,Y2,Y3 as 8 bytes][Z0,Z1,Z2,Z3 as 8
        // bytes]. With Y u16s = [0, 1000, 2000, 3000] and Z u16s =
        // [10000, 20000, 30000, 40000], scale_y=1.0, scale_z=0.001, the
        // expected decoded last key is (offset_x, 3000.0+offset_y,
        // 40.0+offset_z). If the old interleaved decode were used, the last
        // key would consume bytes 24..28 (= Z stream bytes 0..4) and produce
        // a totally different value pair.
        let mut bytes = Vec::new();
        // 24-byte header: scale Vec3 + offset Vec3
        bytes.extend_from_slice(&f32::MAX.to_le_bytes()); // scale_x = FLT_MAX (inactive)
        bytes.extend_from_slice(&1.0f32.to_le_bytes());   // scale_y = 1.0
        bytes.extend_from_slice(&0.001f32.to_le_bytes()); // scale_z = 0.001
        bytes.extend_from_slice(&100.0f32.to_le_bytes()); // offset_x = 100
        bytes.extend_from_slice(&200.0f32.to_le_bytes()); // offset_y = 200
        bytes.extend_from_slice(&300.0f32.to_le_bytes()); // offset_z = 300
        // Planar Y stream (4 keys × u16):
        for v in [0u16, 1000, 2000, 3000] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        // Planar Z stream (4 keys × u16):
        for v in [10000u16, 20000, 30000, 40000] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let positions = read_snorm_packed_positions(&bytes, 0, 4).expect("decode");
        assert_eq!(positions.len(), 4);
        // X is inactive — value is the offset directly.
        for p in &positions {
            assert_eq!(p[0], 100.0, "X must equal offset for inactive axis");
        }
        // Y values: u16 * 1.0 + 200
        let expected_y = [200.0, 1200.0, 2200.0, 3200.0];
        // Z values: u16 * 0.001 + 300
        let expected_z = [310.0, 320.0, 330.0, 340.0];
        for i in 0..4 {
            assert!(
                (positions[i][1] - expected_y[i]).abs() < 1e-3,
                "Y[{i}] = {} (want {})",
                positions[i][1],
                expected_y[i]
            );
            assert!(
                (positions[i][2] - expected_z[i]).abs() < 1e-3,
                "Z[{i}] = {} (want {})",
                positions[i][2],
                expected_z[i]
            );
        }
    }

    /// Single-active-axis `0xC2` channels must continue to decode identically
    /// to the pre-Phase-45 behaviour (planar ≡ interleaved when n_active=1).
    #[test]
    fn snorm_packed_single_active_axis_unchanged() {
        let mut bytes = Vec::new();
        // X and Z inactive, Y active.
        bytes.extend_from_slice(&f32::MAX.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes()); // scale_y = 2.0
        bytes.extend_from_slice(&f32::MAX.to_le_bytes());
        bytes.extend_from_slice(&(-5.0f32).to_le_bytes()); // offset_x = -5
        bytes.extend_from_slice(&10.0f32.to_le_bytes());   // offset_y = 10
        bytes.extend_from_slice(&7.0f32.to_le_bytes());    // offset_z = 7
        for v in [0u16, 100, 200, 300] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let positions = read_snorm_packed_positions(&bytes, 0, 4).expect("decode");
        let expected_y = [10.0, 210.0, 410.0, 610.0];
        for i in 0..4 {
            assert_eq!(positions[i][0], -5.0);
            assert_eq!(positions[i][2], 7.0);
            assert!((positions[i][1] - expected_y[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn dump_database_bone_filter_excludes_unmatched_and_unresolved() {
        // Build a minimal in-memory database with three bones to validate
        // that `bone_filter` keeps only resolved channels whose name
        // contains the substring (case-insensitive).
        let wing_left_hash = bone_name_hash("Wing_Mechanism_Bottom_Left");
        let wing_right_hash = bone_name_hash("Wing_Mechanism_Bottom_Right");
        let other_hash = bone_name_hash("Some_Other_Bone");
        let unresolved_hash: u32 = 0xDEADBEEF;

        let make_ch = |hash: u32| BoneChannel {
            bone_hash: hash,
            rotations: vec![Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] }],
            positions: vec![],
            rot_format_flags: 0,
            pos_format_flags: 0,
        };

        let db = AnimationDatabase {
            clips: vec![AnimationClip {
                name: "wings_deploy".to_string(),
                fps: 30.0,
                channels: vec![
                    make_ch(wing_left_hash),
                    make_ch(wing_right_hash),
                    make_ch(other_hash),
                    make_ch(unresolved_hash),
                ],
                start_rotation: None,
                start_position: None,
            }],
        };
        let mut hash_to_name = std::collections::HashMap::new();
        hash_to_name.insert(wing_left_hash, "Wing_Mechanism_Bottom_Left".to_string());
        hash_to_name.insert(wing_right_hash, "Wing_Mechanism_Bottom_Right".to_string());
        hash_to_name.insert(other_hash, "Some_Other_Bone".to_string());

        // No bone_filter: all 4 channels pass through.
        let no_filter =
            dump_database_to_json(&db, &hash_to_name, None, None, false);
        assert_eq!(no_filter["clips"][0]["channels"].as_array().unwrap().len(), 4);

        // bone_filter="wing_mechanism" (case-insensitive): only the two wings.
        let wings =
            dump_database_to_json(&db, &hash_to_name, None, Some("wing_mechanism"), false);
        let chans = wings["clips"][0]["channels"].as_array().unwrap();
        assert_eq!(chans.len(), 2);
        for ch in chans {
            assert!(ch["bone_name"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains("wing_mechanism"));
        }

        // bone_filter without a skeleton (empty hash_to_name) excludes everything.
        let no_skel = dump_database_to_json(
            &db,
            &std::collections::HashMap::new(),
            None,
            Some("wing_mechanism"),
            false,
        );
        assert_eq!(
            no_skel["clips"][0]["channels"].as_array().unwrap().len(),
            0,
            "channels with unresolved hashes must be excluded when bone_filter is set"
        );
    }

    #[test]
    fn time_format_0x42_decodes_per_frame_keyframe_bitmap() {
        // Phase 47: time format 0x02/0x42 is a per-frame keyframe bitmap of
        // (end - start + 1) bits, LSB-first per byte. Each set bit at index
        // `b` indicates a keyframe at frame `start + b`. The first 4 bytes
        // of the bitmap occupy the slot historically misread as a u32
        // "marker"; the rest follows immediately after.
        //
        // Sample below is the Scorpius `wings_deploy.caf` Top-Right wing
        // mechanism (bone hash 0x5F3AF303). num_rot = 24, end = 75, so the
        // bitmap is 76 bits = 10 bytes. Byte sequence (incl. start/end u16
        // pair) verified empirically by `dump_dba_time_stream` against the
        // shipped Scorpius DBA.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes()); // start
        bytes.extend_from_slice(&75u16.to_le_bytes()); // end
        // 10 bytes of bitmap, LSB-first per byte:
        bytes.extend_from_slice(&[
            0xa5, 0x92, 0x72, 0x8a, // first 4 bytes (was the "marker")
            0x25, 0x59, 0x0a, 0x00, 0x00, 0x08, // remaining 6 bytes
        ]);

        let times = read_time_keys(&bytes, 0, 24, 0x8242).expect("decode bitmap");
        assert_eq!(times.len(), 24, "expected 24 keys, got {}", times.len());
        // Verify a few: first set bit in 0xa5 (= 1010 0101 LSB-first) is
        // bit 0 → frame 0, then bit 2 → frame 2, bit 5 → frame 5, bit 7
        // → frame 7.
        assert_eq!(times[0], 0.0);
        assert_eq!(times[1], 2.0);
        assert_eq!(times[2], 5.0);
        assert_eq!(times[3], 7.0);
        // Last key must reach frame 75 (the end of the bitmap), since
        // 0x08 in byte 9 has bit 3 set → frame 9*8+3 = 75.
        assert_eq!(*times.last().unwrap(), 75.0);
    }

    #[test]
    fn time_format_0x42_count_mismatch_falls_back_to_uniform() {
        // If the encoded bitmap's set-bit count disagrees with the
        // controller's `num_rot_keys`, fall back to uniform stretch so the
        // export still yields something playable. We do NOT silently
        // truncate or pad.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&7u16.to_le_bytes());
        bytes.extend_from_slice(&[0xff]); // 8 bits set → 8 keys
        let times = read_time_keys(&bytes, 0, 5, 0x0042).expect("decode");
        assert_eq!(times.len(), 5);
        // Uniform fallback: 0, 1.75, 3.5, 5.25, 7.0
        assert!((times[0] - 0.0).abs() < 1e-5);
        assert!((times[4] - 7.0).abs() < 1e-5);
    }
