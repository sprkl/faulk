use super::*;
use engine::devices::REGISTRY;
use proptest::prelude::*;

fn synth() -> &'static DeviceDescriptor {
    REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == "faulk.osc_synth")
        .unwrap()
        .descriptor
}

fn preset(params: &[(&str, f64)]) -> Preset {
    Preset {
        device: "faulk.osc_synth".into(),
        version: 1,
        name: "Test".into(),
        category: None,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}

#[test]
fn the_file_is_canonical_json_and_reads_back_the_same() {
    let mut p = preset(&[("attack", 0.004), ("waveform", 2.0), ("gain", -0.0)]);
    p.category = Some("Bass".into());
    let bytes = p.to_file().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(
        text,
        "{\n  \"category\": \"Bass\",\n  \"device\": \"faulk.osc_synth\",\n  \"format\": 1,\n  \"name\": \"Test\",\n  \"params\": {\n    \"attack\": 0.004,\n    \"gain\": 0.0,\n    \"waveform\": 2.0\n  },\n  \"version\": 1\n}\n"
    );
    let back = Preset::parse(&bytes).unwrap();
    assert_eq!(back.to_file().unwrap(), bytes);
    // No category: the field is left out.
    let plain = preset(&[]).to_file().unwrap();
    assert!(!String::from_utf8(plain).unwrap().contains("category"));
}

#[test]
fn a_file_that_is_not_a_preset_is_refused_by_kind_never_with_a_panic() {
    let ok = String::from_utf8(preset(&[("gain", 0.5)]).to_file().unwrap()).unwrap();
    let corrupt = |text: &str| match Preset::parse(text.as_bytes()) {
        Err(PresetError::Corrupt { .. }) => {}
        other => panic!("{text}: {other:?}"),
    };
    corrupt("");
    corrupt("not json");
    corrupt("[]");
    corrupt(&ok.replace("\"version\": 1", "\"version\": 1, \"extra\": 2"));
    corrupt(&ok.replace("\"name\": \"Test\",", ""));
    corrupt(&ok.replace("0.5", "\"loud\""));
    corrupt(&ok.replace("0.5", "1e999"));
    corrupt(&ok.replace("0.5", "NaN"));
    corrupt(&ok.replace("\"gain\": 0.5", "\"gain\": 0.5, \"gain\": 0.6"));
    corrupt(&ok.replace("\"format\": 1", "\"format\": 0"));
    corrupt(&ok.replace("\"version\": 1", "\"version\": -1"));
    assert_eq!(
        Preset::parse(ok.replace("\"format\": 1", "\"format\": 2").as_bytes()),
        Err(PresetError::NewerFormat { format: 2 })
    );
    // A newer format is told apart even when the rest changed.
    assert_eq!(
        Preset::parse(br#"{"format": 9, "shape": "unknown"}"#),
        Err(PresetError::NewerFormat { format: 9 })
    );
    let big = vec![b' '; MAX_PRESET_BYTES + 1];
    assert_eq!(
        Preset::parse(&big),
        Err(PresetError::TooLarge {
            bytes: MAX_PRESET_BYTES + 1
        })
    );
}

#[test]
fn a_file_that_breaks_a_rule_is_refused() {
    let invalid = |p: Preset, field: &str| {
        // `to_file` refuses it, and a file written by hand is refused on read.
        assert!(matches!(p.to_file(), Err(PresetError::Invalid { field: f, .. }) if f == field));
        let text = serde_json::json!({
            "format": 1, "device": p.device, "version": p.version, "name": p.name,
            "category": p.category, "params": p.params,
        });
        let mut text = text.to_string();
        if p.category.is_none() {
            text = text
                .replace(",\"category\":null", "")
                .replace("\"category\":null,", "");
        }
        match Preset::parse(text.as_bytes()) {
            Err(PresetError::Invalid { field: f, .. }) if f == field => {}
            other => panic!("{field}: {other:?} for {text}"),
        }
    };
    let mut p = preset(&[]);
    p.name = "  ".into();
    invalid(p, "name");
    let mut p = preset(&[]);
    p.name = " padded ".into();
    invalid(p, "name");
    let mut p = preset(&[]);
    p.name = "x".repeat(MAX_NAME_CHARS + 1);
    invalid(p, "name");
    let mut p = preset(&[]);
    p.category = Some(String::new());
    invalid(p, "category");
    let mut p = preset(&[]);
    p.category = Some("c".repeat(MAX_CATEGORY_CHARS + 1));
    invalid(p, "category");
    let mut p = preset(&[]);
    p.device = String::new();
    invalid(p, "device");
    let mut p = preset(&[]);
    p.params = (0..=MAX_PRESET_PARAMS)
        .map(|i| (format!("p{i}"), 0.0))
        .collect();
    invalid(p, "params");
    let p = preset(&[(&"k".repeat(MAX_PARAM_KEY_LEN + 1), 0.0)]);
    invalid(p, "params");
    assert!(matches!(
        preset(&[("gain", f64::NAN)]).to_file(),
        Err(PresetError::Invalid { .. })
    ));
}

#[test]
fn resolving_ignores_unknown_values_clamps_the_rest_and_defaults_what_is_missing() {
    let p = preset(&[
        ("waveform", 2.4),
        ("attack", 99.0),
        ("gain", -1.0),
        ("sustain", 0.5),
        ("cutoff", 1000.0),
    ]);
    let r = resolve(&p, synth()).unwrap();
    assert_eq!(r.params["waveform"], 2.0);
    assert_eq!(r.params["attack"], 30.0);
    assert_eq!(r.params["gain"], 0.0);
    assert_eq!(r.params["sustain"], 0.5);
    assert_eq!(r.params["decay"], 0.2);
    assert_eq!(r.params["release"], 0.3);
    assert_eq!(r.ignored, ["cutoff"]);
    assert_eq!(r.clamped, ["waveform", "attack", "gain"]);
    assert_eq!(r.defaulted, ["decay", "release"]);
    assert_eq!(r.migrated_from, None);
    // A value for every declared parameter, nothing else.
    let keys: Vec<&str> = synth().params.iter().map(|p| p.key).collect();
    assert_eq!(r.params.len(), keys.len());
    // Empty values: every parameter at its default.
    let r = resolve(&preset(&[]), synth()).unwrap();
    for param in synth().params {
        assert_eq!(r.params[param.key], param.default);
    }
}

#[test]
fn a_preset_of_another_device_or_a_newer_version_is_refused_with_a_clear_message() {
    let mut p = preset(&[]);
    p.device = "faulk.filter".into();
    assert_eq!(
        resolve(&p, synth()),
        Err(PresetError::OtherDevice {
            preset: "faulk.filter".into(),
            device: "faulk.osc_synth".into()
        })
    );
    let mut p = preset(&[]);
    p.version = 2;
    let error = resolve(&p, synth()).unwrap_err();
    assert_eq!(
        error,
        PresetError::NewerVersion {
            device: "Oscillator".into(),
            preset: 2,
            current: 1
        }
    );
    assert_eq!(
        error.to_string(),
        "the preset was made with version 2 of Oscillator, newer than this Faulk's version 1: update Faulk to load it"
    );
}

#[test]
fn an_older_preset_is_migrated_then_completed_with_defaults() {
    // A device at version 3 whose version 1 called the gain `level` and whose version 2 added
    // nothing: the steps run in order, a version without a step keeps its values.
    let mut descriptor = *synth();
    descriptor.version = 3;
    fn rename_level(values: &mut BTreeMap<String, f64>) {
        if let Some(v) = values.remove("level") {
            values.insert("gain".into(), v);
        }
    }
    fn nan_sustain(values: &mut BTreeMap<String, f64>) {
        values.insert("sustain".into(), f64::NAN);
    }
    let table = [
        Migration {
            device: "faulk.osc_synth",
            from: 1,
            apply: rename_level,
        },
        Migration {
            device: "faulk.filter",
            from: 2,
            apply: nan_sustain,
        },
    ];
    let mut p = preset(&[("level", 0.25), ("attack", 1.0)]);
    p.version = 1;
    let r = resolve_with(&p, &descriptor, &table).unwrap();
    assert_eq!(r.params["gain"], 0.25);
    assert_eq!(r.params["attack"], 1.0);
    assert_eq!(r.params["sustain"], 0.7);
    assert_eq!(r.migrated_from, Some(1));
    assert!(r.ignored.is_empty());
    // A step that leaves a NaN behind: the default, never a panic.
    let table = [Migration {
        device: "faulk.osc_synth",
        from: 2,
        apply: nan_sustain,
    }];
    let r = resolve_with(&p, &descriptor, &table).unwrap();
    assert_eq!(r.params["sustain"], 0.7);
    assert_eq!(r.clamped, ["sustain"]);
    // The shipped table is empty: every built-in is at its first version.
    assert!(MIGRATIONS.is_empty());
    assert!(REGISTRY.iter().all(|b| b.descriptor.version == 1));
}

#[test]
fn every_factory_file_is_canonical_and_sets_every_parameter_of_its_device_within_range() {
    let files = factory::files();
    assert_eq!(
        factory::all().len(),
        files.len(),
        "a factory file does not read"
    );
    let mut ids = std::collections::BTreeSet::new();
    for (id, text) in files {
        assert!(ids.insert(*id), "{id} is there twice");
        let preset = Preset::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(
            preset.to_file().unwrap(),
            text.as_bytes(),
            "{id} is not in the canonical form"
        );
        let (kind, slug) = id.split_once('/').unwrap();
        assert_eq!(kind, preset.device, "{id}");
        assert!(
            !slug.is_empty() && slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{id}"
        );
        let builtin = REGISTRY
            .iter()
            .find(|b| b.descriptor.kind == kind)
            .unwrap_or_else(|| panic!("{id}: no device {kind}"));
        assert_eq!(preset.version, builtin.descriptor.version, "{id}");
        let r = resolve(&preset, builtin.descriptor).unwrap();
        assert!(r.ignored.is_empty(), "{id}: unknown {:?}", r.ignored);
        assert!(r.clamped.is_empty(), "{id}: out of range {:?}", r.clamped);
        assert!(r.defaulted.is_empty(), "{id}: missing {:?}", r.defaulted);
        assert!(
            builtin
                .descriptor
                .params
                .iter()
                .any(|p| r.params[p.key] != p.default),
            "{id} is the device's defaults"
        );
        assert!(preset.category.is_some(), "{id} has no category");
    }
}

#[test]
fn every_built_in_device_has_at_least_two_factory_presets() {
    for builtin in REGISTRY {
        let kind = builtin.descriptor.kind;
        let count = factory::all()
            .iter()
            .filter(|f| f.preset.device == kind)
            .count();
        assert!(count >= 2, "{kind} has {count} factory presets");
    }
    assert_eq!(
        factory::get("faulk.osc_synth/round-bass").map(|f| f.preset.name.as_str()),
        Some("Round Bass")
    );
    assert!(factory::get("faulk.osc_synth/none").is_none());
}

fn arb_preset() -> impl Strategy<Value = Preset> {
    let name = "[A-Za-z0-9][A-Za-z0-9 éß'-]{0,30}[A-Za-z0-9]";
    (
        "faulk\\.[a-z_]{1,20}",
        0u32..5,
        name,
        proptest::option::of("[A-Z][a-z]{0,10}"),
        proptest::collection::btree_map("[a-z_]{1,20}", any::<f64>(), 0..20),
    )
        .prop_map(|(device, version, name, category, params)| Preset {
            device,
            version,
            name,
            category,
            params: params
                .into_iter()
                .map(|(k, v)| (k, if v.is_finite() { v } else { 0.0 }))
                .collect(),
        })
}

proptest! {
    #[test]
    fn write_read_write_gives_the_same_bytes(p in arb_preset()) {
        let bytes = p.to_file().unwrap();
        let back = Preset::parse(&bytes).unwrap();
        prop_assert_eq!(back.to_file().unwrap(), bytes);
        // The values are those written (`-0` reads back as `0`).
        for (k, v) in &p.params {
            prop_assert_eq!(back.params[k], *v);
        }
    }
}
