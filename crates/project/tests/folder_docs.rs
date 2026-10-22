//! The files, checked against the examples of the format's documentation
//! (`docs/architecture/02-modele-projet.md` § 2.1 to § 2.5), written by hand here and never
//! produced by the writer under test: a writer that drifts from the documented format fails
//! here even when its own reference folders were blessed from it.

use project::file::*;

mod common;
use common::reference::{LAYOUT, id, song};

fn options() -> SaveOptions {
    SaveOptions {
        layout: LAYOUT,
        support_files: false,
        write_cache: false,
    }
}

fn text(fs: &MemFs, path: &str) -> String {
    String::from_utf8(
        fs.read(path)
            .unwrap_or_else(|_| panic!("{path} is not written")),
    )
    .unwrap()
}

fn t(n: u8) -> String {
    id(n).to_string()
}

#[test]
fn the_files_are_the_ones_the_documentation_shows() {
    let mut fs = MemFs::new();
    save(&mut fs, &song(), &mut SaveState::new(), None, &options()).unwrap();
    let (track, audio) = (format!("tracks/{}", t(50)), format!("tracks/{}", t(51)));

    // § 2.3, notes: the four columns, sorted.
    assert_eq!(
        text(&fs, &format!("{track}/clips/{}.notes.tsv", t(20))),
        "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\n0\t240\t36\t110\n0\t240\t48\t96\n480\t240\t36\t100\n960\t120\t43\t100\n"
    );
    // § 2.3, the optional columns come after the base ones, and every row carries them.
    assert_eq!(
        text(&fs, &format!("{track}/clips/{}.notes.tsv", t(21))),
        "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\tcents\tchance\trepeat\n480\t240\t36\t100\t0\t50\t1\n960\t120\t40\t64\t-12\t100\t4\n"
    );
    // § 2.3, a lane of points.
    assert_eq!(
        text(&fs, &format!("{track}/automation/{}.tsv", t(40))),
        format!(
            "# faulk-automation 1\ttarget=device:{}/cutoff\n0\t800\tlinear\n3840\t2400\tlinear\n7680\t2400\tstep\n",
            t(11)
        )
    );
    // § 2.1: past the threshold the header names a blob instead of listing the points.
    let lane = text(&fs, &format!("{track}/automation/{}.tsv", t(41)));
    assert!(
        lane.starts_with("# faulk-automation 1\ttarget=mixer:pan\tblob="),
        "{lane}"
    );
    assert!(lane.ends_with("\tpoints=6\n"));
    // § 2.3, an arrangement, in ticks for a MIDI track and in frames for an audio one.
    assert_eq!(
        text(&fs, &format!("{track}/arrangement.tsv")),
        format!(
            "# faulk-arrangement 1\tid\tstart\tlength\tclip\toffset\n{}\t0\t15360\t{}\t0\n{}\t15360\t1920\t{}\t480\n",
            t(30),
            t(20),
            t(31),
            t(21)
        )
    );
    assert_eq!(
        text(&fs, &format!("{audio}/arrangement.tsv")),
        format!(
            "# faulk-arrangement 1\tid\tstart\tlength\tclip\toffset\n{}\t3840\t44100\t{}\t1000\n",
            t(32),
            t(22)
        )
    );
    // § 2.5: silence is `null`; a device file is sorted JSON, one value a line.
    let track_json = text(&fs, &format!("{audio}/track.json"));
    assert!(track_json.contains("\"gain_db\": null"), "{track_json}");
    assert_eq!(
        text(&fs, &format!("{track}/devices/{}.json", t(10))),
        "{\n  \"enabled\": true,\n  \"order\": \"a0\",\n  \"params\": {\n    \"attack\": 0.01,\n    \"wave\": 2.0\n  },\n  \"role\": \"instrument\",\n  \"source\": {\n    \"builtin\": {\n      \"id\": \"faulk.osc_synth\",\n      \"version\": 1\n    }\n  },\n  \"state\": null\n}\n"
    );
    // § 2.2: the clip header holds no notes; the audio reference carries its metadata.
    let clip = text(&fs, &format!("{track}/clips/{}.json", t(20)));
    assert!(!clip.contains("notes"), "{clip}");
    let clip = text(&fs, &format!("{audio}/clips/{}.json", t(22)));
    assert!(clip.contains("\"sample_rate\": 44100") && clip.contains("\"channels\": 2"));
    // The project file holds no track: they are folders.
    assert!(!text(&fs, "project.json").contains("tracks"));
    common::rules::check(&fs);
}

#[test]
fn a_note_with_a_non_default_value_only_adds_that_column() {
    // The second example of § 2.3: the chance column alone, the default value on every
    // line that has no other.
    use project::*;
    let mut p = Project::new(ProjectId(id(1)), "P");
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    let mk = |start, chance| Note {
        chance,
        ..Note::new(Tick(start), 36, Tick(240), 110)
    };
    track.clips.insert(
        ClipId(id(20)),
        Clip {
            name: "C".into(),
            color: None,
            slot: None,
            launch_quantize: None,
            content: ClipContent::Midi {
                length: Tick(960),
                looping: None,
                notes: [mk(0, 100), mk(480, 50)].into(),
            },
        },
    );
    p.tracks.insert(TrackId(id(50)), track);
    let mut fs = MemFs::new();
    save(&mut fs, &p, &mut SaveState::new(), None, &options()).unwrap();
    assert_eq!(
        text(&fs, &format!("tracks/{}/clips/{}.notes.tsv", t(50), t(20))),
        "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\tchance\n0\t240\t36\t110\t100\n480\t240\t36\t110\t50\n"
    );
}
