use std::collections::HashMap;
use std::hint::black_box;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Instant;

const N: usize = 10_000;

#[derive(Clone)]
struct HeavySlot {
    name: String,
    metadata: [String; 3],
    shaped_glyphs: Vec<u32>,
    flags: u32,
}

#[derive(Clone, Copy, Default)]
struct RowSoa {
    name_id: u32,
    icon_id: u16,
    flags: u16,
    y: f32,
}

#[derive(Clone, Copy, Default)]
struct DrawInst {
    rect: [f32; 4],
    color: u32,
    resource: u32,
}

fn bench(mut f: impl FnMut() -> u64, iterations: usize) -> (f64, u64) {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        checksum ^= black_box(f());
    }
    let us = start.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64;
    (us, checksum)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn main() {
    let names: Vec<String> = (0..N)
        .map(|i| format!("file_{i:05}_representative_filename.rs"))
        .collect();
    let mut rows: Vec<RowSoa> = (0..N)
        .map(|i| RowSoa {
            name_id: i as u32,
            icon_id: (i & 1) as u16,
            flags: (i & 7) as u16,
            y: i as f32 * 24.0,
        })
        .collect();
    let mut by_id = HashMap::with_capacity(N);
    for (i, name) in names.iter().enumerate() {
        by_id.insert(fnv1a(name.as_bytes()), i as u32);
    }

    let heavy: Vec<Arc<HeavySlot>> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            Arc::new(HeavySlot {
                name: name.clone(),
                metadata: [
                    format!("{} KB", i * 4),
                    "2026-08-02 21:00".to_owned(),
                    "Rust source".to_owned(),
                ],
                shaped_glyphs: name.bytes().map(u32::from).collect(),
                flags: (i & 7) as u32,
            })
        })
        .collect();

    let (scan_us, scan_sum) = bench(
        || {
            let mut sum = 0_u64;
            for row in &mut rows {
                row.y += 0.25;
                row.flags ^= 1;
                sum = sum.wrapping_add(row.name_id as u64 + row.icon_id as u64 + row.flags as u64);
            }
            sum
        },
        5_001,
    );

    let mut commands = Vec::<DrawInst>::with_capacity(N * 3);
    let (commands_us, command_sum) = bench(
        || {
            commands.clear();
            for row in &rows {
                let y = row.y;
                commands.push(DrawInst {
                    rect: [0.0, y, 900.0, y + 24.0],
                    color: 0x101010ff,
                    resource: 0,
                });
                commands.push(DrawInst {
                    rect: [8.0, y + 4.0, 24.0, y + 20.0],
                    color: 0xffffffff,
                    resource: row.icon_id as u32,
                });
                commands.push(DrawInst {
                    rect: [30.0, y, 880.0, y + 24.0],
                    color: 0xffffffff,
                    resource: row.name_id,
                });
            }
            black_box(commands.as_slice());
            let last = commands.last().copied().unwrap_or_default();
            commands.len() as u64 ^ last.resource as u64 ^ last.color as u64 ^ last.rect[1] as u64
        },
        2_001,
    );

    let mut indexed_commands = vec![DrawInst::default(); N * 3];
    let (indexed_commands_us, indexed_command_sum) = bench(
        || {
            for (i, row) in rows.iter().enumerate() {
                let y = row.y;
                let at = i * 3;
                indexed_commands[at] = DrawInst {
                    rect: [0.0, y, 900.0, y + 24.0],
                    color: 0x101010ff,
                    resource: 0,
                };
                indexed_commands[at + 1] = DrawInst {
                    rect: [8.0, y + 4.0, 24.0, y + 20.0],
                    color: 0xffffffff,
                    resource: row.icon_id as u32,
                };
                indexed_commands[at + 2] = DrawInst {
                    rect: [30.0, y, 880.0, y + 24.0],
                    color: 0xffffffff,
                    resource: row.name_id,
                };
            }
            black_box(indexed_commands.as_slice());
            let last = indexed_commands.last().copied().unwrap_or_default();
            indexed_commands.len() as u64
                ^ last.resource as u64
                ^ last.color as u64
                ^ last.rect[1] as u64
        },
        2_001,
    );

    let (hash_us, hash_sum) = bench(
        || names.iter().map(|name| fnv1a(name.as_bytes())).fold(0, u64::wrapping_add),
        1_001,
    );

    let (map_us, map_sum) = bench(
        || {
            names
                .iter()
                .map(|name| by_id[&fnv1a(name.as_bytes())] as u64)
                .fold(0, u64::wrapping_add)
        },
        1_001,
    );

    let mut arc_snapshot = Vec::<Arc<HeavySlot>>::with_capacity(N);
    let (arc_collect_us, arc_sum) = bench(
        || {
            arc_snapshot.clear();
            arc_snapshot.extend(heavy.iter().cloned());
            arc_snapshot.iter().map(|slot| slot.flags as u64).sum()
        },
        1_001,
    );

    // Models Arc::make_mut while a paint snapshot still owns every slot.
    let (cow_us, cow_sum) = bench(
        || {
            let paint_snapshot: Vec<_> = heavy.iter().cloned().collect();
            let mut writable = heavy.clone();
            for slot in &mut writable {
                Arc::make_mut(slot).flags ^= 1;
            }
            paint_snapshot.len() as u64
                ^ writable.iter().map(|slot| slot.flags as u64).sum::<u64>()
        },
        101,
    );

    let mut rebuilt = Vec::<HeavySlot>::with_capacity(N);
    let (owned_rebuild_us, owned_sum) = bench(
        || {
            rebuilt.clear();
            rebuilt.extend(heavy.iter().map(|slot| HeavySlot {
                name: slot.name.clone(),
                metadata: slot.metadata.clone(),
                shaped_glyphs: slot.shaped_glyphs.clone(),
                flags: slot.flags,
            }));
            rebuilt.iter().map(|slot| slot.name.len() as u64).sum()
        },
        101,
    );

    let owned_bytes: usize = heavy
        .iter()
        .map(|slot| {
            size_of::<HeavySlot>()
                + slot.name.capacity()
                + slot.metadata.iter().map(String::capacity).sum::<usize>()
                + slot.shaped_glyphs.capacity() * size_of::<u32>()
        })
        .sum();

    println!("10,000-row isolated CPU lab (best interpreted as order-of-magnitude)");
    println!("contiguous scan/update       {scan_us:9.2} us   checksum={scan_sum}");
    println!("emit 30k flat draw records  {commands_us:9.2} us   checksum={command_sum}");
    println!("overwrite 30k draw records  {indexed_commands_us:9.2} us   checksum={indexed_command_sum}");
    println!("hash 10k names (FNV-1a)     {hash_us:9.2} us   checksum={hash_sum}");
    println!("hash + HashMap lookup       {map_us:9.2} us   checksum={map_sum}");
    println!("collect 10k Arc pointers    {arc_collect_us:9.2} us   checksum={arc_sum}");
    println!("Arc::make_mut while shared  {cow_us:9.2} us   checksum={cow_sum}");
    println!("deep rebuild owned payload  {owned_rebuild_us:9.2} us   checksum={owned_sum}");
    println!();
    println!("sizeof RowSoa={} DrawInst={} HeavySlot(header)={}", size_of::<RowSoa>(), size_of::<DrawInst>(), size_of::<HeavySlot>());
    println!("heavy payload estimate for 10k rows: {:.2} MiB", owned_bytes as f64 / 1_048_576.0);
}
