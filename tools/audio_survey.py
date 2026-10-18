#!/usr/bin/env python3
"""World-audio survey (retail audio A2 / S2): per map, the emitter records, the native banks they
play and the ambience beds, with their size as 4-bit IMA ADPCM (36 bytes per 64 frames: 0.28 of
the PCM16 bytes). Reads the user's own audio export; prints counts and sizes only.
usage: tools/audio_survey.py ../game_data/audio-export/audio"""
import collections, json, os, sys

root = sys.argv[1]
m = json.load(open(os.path.join(root, "audio_manifest.json")))
# The .ems files of each retail map (the reference's `map_audio::tests::old_ems_files`).
EMS = {
    "University": ["music_university", "sfx_university", "reverb_university", "speakers_university", "crowds_university"],
    "DownTown": ["music_downtown", "sfx_downtown", "reverb_downtown", "speakers_downtown", "crowds_downtown"],
    "Industrial": ["music_industrial", "sfx_industrial", "reverb_industrial", "speakers_industrial", "crowds_industrial"],
    "DownTownSkatePark": ["sfx_dt_skatepark"], "IndustrialSkatePark": ["sfx_ind_skatepark"], "MegaPark": ["sfx_mega_skatepark"],
    "MaloofMoneyCup": ["sfx_maloof_money_cup"], "StartPark": ["sfx_startpark"], "BlackBoxPark": ["sfx_blackbox_park"], "SkateSchool": ["skateschool"],
}
ADPCM = 36 / 128

def pcm(path):
    if os.path.isdir(path):
        return sum(os.path.getsize(os.path.join(path, f)) for f in os.listdir(path))
    return os.path.getsize(path) if os.path.exists(path) else 0

print(f"{'map':21}{'records':>8}{'w/o bank':>9}{'banks':>6}{'banks MB':>9}{'zones':>6}{'beds':>5}{'beds MB':>8}  largest banks (MB)")
for name, groups in EMS.items():
    recs = [r for g in groups for r in m["emitters"].get(g, [])]
    banks = sorted({r["bank"] for r in recs if r.get("bank")})
    sizes = sorted(((pcm(os.path.join(root, "banks", b)) * ADPCM / 2**20, b) for b in banks), reverse=True)
    keys = {k for layer, tiles in m["regions"].get(name, {}).items() if "ambience" in layer for t in tiles for k in t.get("keys", [])}
    beds = sorted({m["zones"][k]["bed"] for k in keys if k in m["zones"]})
    bed_mb = sum(pcm(os.path.join(root, "ambience", b + ".wav")) for b in beds) * ADPCM / 2**20
    top = ", ".join(f"{b} {s:.1f}" for s, b in sizes[:3])
    print(f"{name:21}{len(recs):>8}{sum(1 for r in recs if not r.get('bank')):>9}{len(banks):>6}{sum(s for s, _ in sizes):>9.1f}{len(keys):>6}{len(beds):>5}{bed_mb:>8.1f}  {top}")
print("\nall ambience beds:", len(m["ambience"]), f"{sum(pcm(os.path.join(root, 'ambience', b + '.wav')) for b in m['ambience']) * ADPCM / 2**20:.0f} MB ADPCM")
print("emitter kinds:", dict(collections.Counter(r.get("kind") for g in m["emitters"].values() for r in g)))
