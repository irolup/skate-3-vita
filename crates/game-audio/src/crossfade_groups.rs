//! Copy of the reference's `game_audio/crossfade_groups.rs` (`2e16697`; `pub(crate)`): the layouts the
//! zone crossfades play, measured from the banks' programs and checked against them upstream.
//! See PROVENANCE.md.
//! What each `c_main_ambience_crossfade` group plays, measured by posting the program (w0 = w8 = 32767,
//! w9 = group) through the PoC evaluator (2026-10-02; local data). Four looping voices:
//! (bank sample, pan degrees, level incl. the rear 23000/32767 factor). The game now reads the
//! layouts from each bank's program (`crossfade_layouts.rs`); this table stays only as the test
//! oracle that the programs reproduce it exactly. Do not hand-edit.

/// (district bank, group, voices)
pub(crate) const GROUPS: &[(&str, u32, [(usize, f32, f32); 4])] = &[
    ("Main_Ambience_Crossfade_DT", 1, [(1, 45.0, 1.0000), (1, 225.0, 0.7019), (0, 135.0, 0.7019), (0, 315.0, 1.0000)]),
    ("Main_Ambience_Crossfade_DT", 2, [(1, 45.0, 1.0000), (1, 225.0, 0.7019), (0, 135.0, 0.7019), (0, 295.2, 1.0000)]),
    ("Main_Ambience_Crossfade_DT", 3, [(2, 295.2, 1.0000), (3, 45.0, 1.0000), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_DT", 4, [(5, 45.0, 1.0000), (5, 225.0, 0.7019), (4, 135.0, 0.7019), (4, 295.2, 1.0000)]),
    ("Main_Ambience_Crossfade_DT", 5, [(7, 45.0, 1.0000), (7, 225.0, 0.7019), (6, 135.0, 0.7019), (6, 295.2, 1.0000)]),
    ("Main_Ambience_Crossfade_DT", 6, [(9, 45.0, 1.0000), (9, 225.0, 0.7019), (8, 135.0, 0.7019), (8, 295.2, 1.0000)]),
    ("Main_Ambience_Crossfade_DT", 7, [(6, 295.2, 1.0000), (7, 45.0, 1.0000), (7, 225.0, 0.7019), (6, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Ind", 1, [(0, 135.0, 0.7019), (1, 225.0, 0.7019), (0, 315.0, 1.0000), (1, 45.0, 1.0000)]),
    ("Main_Ambience_Crossfade_Ind", 2, [(2, 295.2, 1.0000), (3, 45.0, 1.0000), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Ind", 3, [(4, 295.2, 1.0000), (5, 45.0, 1.0000), (4, 135.0, 0.7019), (5, 225.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Ind", 4, [(0, 135.0, 0.7019), (1, 225.0, 0.7019), (1, 45.0, 0.6104), (0, 295.2, 0.6104)]),
    ("Main_Ambience_Crossfade_Ind", 5, [(3, 225.0, 0.7019), (2, 135.0, 0.7019), (2, 295.2, 0.6104), (3, 45.0, 0.6104)]),
    ("Main_Ambience_Crossfade_Ind", 6, [(7, 225.0, 0.7019), (6, 135.0, 0.7019), (6, 295.2, 0.6104), (7, 45.0, 0.6104)]),
    ("Main_Ambience_Crossfade_Ind", 7, [(8, 295.2, 1.0000), (9, 45.0, 1.0000), (9, 225.0, 0.7019), (8, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Ind", 8, [(2, 295.2, 1.0000), (3, 45.0, 1.0000), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Ind", 9, [(1, 225.0, 0.7019), (0, 135.0, 0.7019), (0, 295.2, 0.6104), (1, 45.0, 0.6104)]),
    ("Main_Ambience_Crossfade_Uni", 1, [(2, 315.0, 1.0000), (3, 45.0, 1.0000), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Uni", 2, [(0, 295.2, 1.0000), (1, 45.0, 1.0000), (1, 225.0, 0.7019), (0, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Uni", 3, [(2, 295.2, 1.0000), (3, 45.0, 1.0000), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Uni", 4, [(2, 295.2, 0.6104), (3, 45.0, 0.6104), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
    ("Main_Ambience_Crossfade_Uni", 5, [(2, 295.2, 0.6104), (3, 45.0, 0.6104), (3, 225.0, 0.7019), (2, 135.0, 0.7019)]),
];
