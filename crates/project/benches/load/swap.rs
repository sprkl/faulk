//! The cost of the engine's structure swaps (§ 2.3) on a heavy project: the command layer's
//! snapshot of the project, run message by message through the real `Preparer` and `Engine`
//! (the native target; the web target runs the same Rust in the shipped worklet).

use crate::alloc;
use crate::stats::*;
use commands::engine_msgs::{self, EngineStream};
use engine::device::DeviceSetup;
use engine::device::testing::factory;
use engine::engine::channel;
use engine::msg::ControlMsg;
use project::*;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Default)]
struct Series {
    n: usize,
    items: usize,
    prepare: Vec<f64>,
    apply: Vec<f64>,
    drop: Vec<f64>,
}

fn items(msg: &ControlMsg) -> usize {
    match msg {
        ControlMsg::ClipNotesUpload { notes, .. } => notes.len(),
        ControlMsg::AutomationLaneUpload { points, .. } => points.len(),
        ControlMsg::TrackPlacementsUpload { placements, .. } => placements.len(),
        _ => 0,
    }
}

pub struct Sent {
    pub notes: usize,
    pub points: usize,
    pub messages: usize,
}

/// Counts what the snapshot sends to the engine, against what the project holds: nothing is
/// silently left out of the measure.
pub fn sent(msgs: &[ControlMsg]) -> Sent {
    let mut s = Sent {
        notes: 0,
        points: 0,
        messages: msgs.len(),
    };
    for m in msgs {
        match m {
            ControlMsg::ClipNotesUpload { notes, .. } => s.notes += notes.len(),
            ControlMsg::AutomationLaneUpload { points, .. } => s.points += points.len(),
            _ => {}
        }
    }
    s
}

pub fn counts(project: &Project) -> (usize, usize) {
    let mut notes = 0;
    let mut points = 0;
    for t in project.tracks.values() {
        for c in t.clips.values() {
            if let ClipContent::Midi { notes: n, .. } = &c.content {
                notes += n.len();
            }
        }
        points += t.automation.values().map(|l| l.points.len()).sum::<usize>();
    }
    (notes, points)
}

/// Runs the whole synchronisation of `project` and prints the costs.
pub fn run(project: &Project, label: &str) {
    println!("== engine swaps: {label}");
    let (notes, points) = counts(project);
    let (t_snap, d_snap) = {
        let (r, d) = time(|| engine_msgs::snapshot(project, &SessionState::default()));
        (r, d)
    };
    let (msgs, slots) = t_snap;
    let s = sent(&msgs);
    println!(
        "snapshot of the project into engine messages: {:.1} ms, {} messages, {} notes of {} in the project, {} points of {}, slots overflow {}",
        ms(d_snap),
        s.messages,
        s.notes,
        notes,
        s.points,
        points,
        slots.overflow
    );
    println!(
        "  (the command layer sends no audio clip and no audio placement yet: engine_msgs.rs; the audio is not part of the swap)"
    );

    let mut stream = EngineStream::new(1);
    let (bytes, d_enc) = time(|| stream.encode(&msgs).expect("encode"));
    println!(
        "encoded stream: {:.1} MiB in {:.1} ms (the bytes the page sends to the prepare side)",
        mib(bytes.len() as u64),
        ms(d_enc)
    );
    drop(bytes);

    // Path A: message by message, the prepare, the apply and the garbage drop timed apart.
    // The messages are made again with the counter on, so that what they hold is counted and
    // released as the engine takes it (this second snapshot is not timed).
    drop(msgs);
    alloc::start();
    let (msgs, _) = engine_msgs::snapshot(project, &SessionState::default());
    let msgs_heap = alloc::live();
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).expect("channel");
    let mut by_tag: BTreeMap<String, Series> = BTreeMap::new();
    let (mut prep_total, mut apply_total) = (0f64, 0f64);
    let wall = Instant::now();
    let mut rejected_before = p.rejected();
    for msg in msgs {
        let tag = format!("{:?}", msg.tag());
        let n_items = items(&msg);
        let entry = by_tag.entry(tag).or_default();
        entry.n += 1;
        entry.items += n_items;
        let t0 = Instant::now();
        let rt = p.prepare(msg);
        let t1 = Instant::now();
        let rt = match rt {
            Ok(rt) => rt,
            Err(error) => {
                println!("  prepare refused a message: {error:?}");
                rejected_before += 1;
                None
            }
        };
        let prep = us(t1 - t0);
        entry.prepare.push(prep);
        prep_total += prep;
        if let Some(rt) = rt {
            let a0 = Instant::now();
            e.apply(rt);
            let a1 = Instant::now();
            p.collect();
            let a2 = Instant::now();
            entry.apply.push(us(a1 - a0));
            entry.drop.push(us(a2 - a1));
            apply_total += us(a1 - a0);
        }
    }
    let wall = wall.elapsed();
    let engine_heap = alloc::live();
    let peak = alloc::peak();
    alloc::stop();
    println!(
        "synchronisation, one thread, prepare then apply: {:.1} ms wall; prepare total {:.1} ms, apply total {:.3} ms; refused: {}; rejected counter {}; engine skipped {}",
        ms(wall),
        prep_total / 1000.0,
        apply_total / 1000.0,
        rejected_before,
        p.rejected(),
        e.skipped()
    );
    println!(
        "heap held by the engine after it: {:.1} MiB (the messages alone held {:.1} MiB before); peak during the run (messages, staging and engine together): {:.1} MiB",
        mib(engine_heap as u64),
        mib(msgs_heap as u64),
        mib(peak as u64)
    );
    println!(
        "tag | n | items | prepare µs (median / p99 / max) | apply µs (median / p99 / max) | drop µs (max)"
    );
    let mut worst_apply = ("".to_owned(), 0f64);
    for (tag, s) in &by_tag {
        println!(
            "{tag} | {} | {} | {:.1} / {:.1} / {:.1} | {:.2} / {:.2} / {:.2} | {:.1}",
            s.n,
            s.items,
            median(&s.prepare),
            percentile(&s.prepare, 0.99),
            max(&s.prepare),
            median(&s.apply),
            percentile(&s.apply, 0.99),
            max(&s.apply),
            max(&s.drop)
        );
        if max(&s.apply) > worst_apply.1 {
            worst_apply = (tag.clone(), max(&s.apply));
        }
    }
    println!(
        "longest single apply: {} at {:.2} µs (budget of MO-2: 500 µs for the swap of a table at its limit)",
        worst_apply.0, worst_apply.1
    );
}

/// Path B: what the audio thread sees while the project arrives: blocks of 128 frames, at
/// most 128 messages applied per block (`MAX_APPLY_PER_BLOCK`), the prepare side feeding the
/// ring as room frees. Prints the number of blocks the synchronisation takes and the cost of
/// the most expensive one.
pub fn blocks(project: &Project) {
    let (msgs, _) = engine_msgs::snapshot(project, &SessionState::default());
    let total = msgs.len();
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).expect("channel");
    let mut left = vec![0f32; 128];
    let mut right = vec![0f32; 128];
    let mut costs = Vec::new();
    let mut blocks = 0usize;
    let wall = Instant::now();
    let mut queue: std::collections::VecDeque<ControlMsg> = msgs.into();
    while !queue.is_empty() || p.pending() > 0 {
        // The control side hands over as many messages as the engine has room for.
        while p.pending() < 1024
            && let Some(m) = queue.pop_front()
        {
            if let Err(error) = p.handle(m) {
                println!("  refused: {error:?}");
            }
        }
        p.flush();
        let t = Instant::now();
        e.process(&mut left, &mut right);
        costs.push(us(t.elapsed()));
        blocks += 1;
        p.flush();
    }
    // Drain what is in flight: until three blocks in a row apply nothing.
    let mut idle = 0;
    while idle < 3 {
        let before = e.applied();
        let t = Instant::now();
        e.process(&mut left, &mut right);
        costs.push(us(t.elapsed()));
        blocks += 1;
        p.flush();
        idle = if e.applied() == before { idle + 1 } else { 0 };
    }
    blocks -= 3;
    let applied_blocks = costs.len() - 3;
    let busy: Vec<f64> = costs[..applied_blocks].to_vec();
    // The idle cost of a block of this engine, for comparison.
    let idle_costs: Vec<f64> = (0..200)
        .map(|_| {
            let t = Instant::now();
            e.process(&mut left, &mut right);
            us(t.elapsed())
        })
        .collect();
    println!(
        "  idle block of the same engine: median {:.1} µs, p99 {:.1}, max {:.1} (100 tracks, nothing playing)",
        median(&idle_costs),
        percentile(&idle_costs, 0.99),
        max(&idle_costs)
    );
    let costs = busy;
    println!(
        "block loop: {} messages brought to the engine in {} blocks of 128 frames ({:.1} ms of audio at 48 kHz, {:.1} ms wall); block cost µs: median {:.1}, p99 {:.1}, max {:.1}; applied {}, skipped {}, rejected {}",
        total,
        blocks,
        blocks as f64 * 128.0 / 48.0,
        ms(wall.elapsed()),
        median(&costs),
        percentile(&costs, 0.99),
        max(&costs),
        e.applied(),
        e.skipped(),
        p.rejected()
    );
}

/// Writes the engine stream of the heavy project (built-in devices only) in the format of
/// `crates/engine-wasm/tests/fixtures` (a push is `0`, its length as a `u32`, its bytes), cut
/// into pushes of whole messages of about 2 MiB at most, for the browser bench
/// (`web/tests/bench/engine-swap.bench.ts`).
pub fn dump_stream(out: &str) {
    let (project, _) = crate::synth::generate(&crate::synth::Profile::heavy_builtin(), 0);
    let (msgs, _) = engine_msgs::snapshot(&project, &SessionState::default());
    let s = sent(&msgs);
    let mut stream = EngineStream::new(0);
    let mut file: Vec<u8> = Vec::new();
    let (mut group, mut size, mut pushes) = (Vec::new(), 0usize, 0usize);
    let mut flush = |group: &mut Vec<ControlMsg>, file: &mut Vec<u8>, pushes: &mut usize| {
        if group.is_empty() {
            return;
        }
        let bytes = stream.encode(group).expect("encode");
        file.push(0);
        file.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        file.extend_from_slice(&bytes);
        group.clear();
        *pushes += 1;
    };
    for msg in msgs {
        size += items(&msg) * 24 + 64;
        group.push(msg);
        if size >= 2 << 20 {
            flush(&mut group, &mut file, &mut pushes);
            size = 0;
        }
    }
    flush(&mut group, &mut file, &mut pushes);
    std::fs::write(out, &file).expect("write the stream");
    println!(
        "stream: {} messages ({} notes, {} points) in {} pushes, {:.1} MiB, written to {out}",
        s.messages,
        s.notes,
        s.points,
        pushes,
        mib(file.len() as u64)
    );
}
