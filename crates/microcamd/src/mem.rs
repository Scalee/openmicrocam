//! `microcam mem`: the memory pools read live — the media heap by owner from the heap's own listing, microcamd's
//! share of it by use, Linux memory, and the peaks of microcamd's budgets since the last reset.

use std::collections::BTreeMap;

use microcam_draw::render::{layer_peak, reset_layer_peak, LAYER_BYTES};
use microcam_core::protocol::Account;
use mi::layer::{reset_peaks, usage, Use};


const HEAP: &str = "/proc/mi_modules/mi_sys_mma/mma_heap_name0";
/// the owner the heap's listing gives every piece a process allocates, microcamd's included
const PROCESS: &str = "app-mmaAlloc";

fn mib(b: u64) -> String {
    format!("{:.1}", b as f64 / (1 << 20) as f64)
}

/// The account: the video's owners `priced` beside the heap's listing, microcamd's share and its use.
pub fn account(priced: Vec<(String, u64)>) -> Account {
    let heap = std::fs::read_to_string(HEAP).map(|text| Heap::parse(&text)).unwrap_or(Heap { size: 0, free: 0, owners: BTreeMap::new() });
    let mut held: Vec<_> = heap.owners.into_iter().map(|(name, (bytes, _))| (name, bytes)).collect();
    held.sort_by(|a, b| b.1.cmp(&a.1));
    Account { priced, held, heap: heap.size, free: heap.free, share: mi::layer::share(), used: mi::layer::held() }
}

/// The report, one line each (`brief`: one line in all); `reset` starts the peaks again from what is held now. The
/// video's owners as `priced` follow the heap's listing.
pub fn report(reset: bool, brief: bool, priced: Option<Vec<(String, u64)>>) -> Vec<String> {
    if reset {
        reset_peaks();
        reset_layer_peak();
    }
    let mut out = Vec::new();
    let mut process = 0;
    let heap = std::fs::read_to_string(HEAP).map(|text| Heap::parse(&text));
    let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |text: &str, key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.split_whitespace().next()?.parse::<u64>().ok()).map_or("?".into(), |k| mib(k * 1024));
    if brief {
        let menu = usage().into_iter().find(|u| u.0 == Use::Menu).map_or(0, |u| u.1);
        let heap = match &heap {
            Ok(h) => {
                let (bytes, n) = h.owners.get("microcam_canvas").copied().unwrap_or_default();
                format!("heap free {} of {} MiB, canvases {} ×{n}", mib(h.free), mib(h.size), mib(bytes))
            }
            Err(e) => format!("heap: {e}"),
        };
        return vec![format!("{heap}, menu {} of {} (microcamd {}), Linux available {}", mib(menu), mib(mi::layer::share()), mib(mi::layer::held()), kb(&info, "MemAvailable:"))];
    }
    match heap {
        Ok(heap) => {
            out.push(format!("media heap {} MiB: free {}, in use {}", mib(heap.size), mib(heap.free), mib(heap.used())));
            let mut owners: Vec<_> = heap.owners.iter().collect();
            owners.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (name, (bytes, n)) in owners {
                out.push(format!("  {name:<20} {:>6}  ×{n}", mib(*bytes)));
            }
            process = heap.owners.get(PROCESS).map_or(0, |o| o.0);
        }
        Err(e) => out.push(format!("media heap: {HEAP}: {e}")),
    }
    if let Some(priced) = priced {
        let total: u64 = priced.iter().map(|o| o.1).sum();
        let owners: Vec<String> = priced.iter().map(|(n, b)| format!("{n} {}", mib(*b))).collect();
        out.push(format!("video priced {}: {}", mib(total), owners.join(", ")));
    }
    let uses = usage();
    let ours: u64 = uses.iter().map(|u| u.1).sum();
    let mut line = format!("microcamd's {PROCESS}:");
    for &(u, held, peak) in &uses {
        line += &format!(" {} {} (peak {}),", u.name(), mib(held), mib(peak));
    }
    // the RGN canvases are mi_rgn's allocations, listed under their own owner, not the process's
    let in_process: u64 = uses.iter().filter(|u| u.0 != Use::Rgn).map(|u| u.1).sum();
    line += &format!(" — {} of its share {}; not microcamd's {}", mib(ours), mib(mi::layer::share()), mib(process.saturating_sub(in_process)));
    out.push(line);
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    out.push(format!(
        "Linux {} MiB: available {}, free {}; microcamd resident {}; a tile's drawing at most {} of {} (peak)",
        kb(&info, "MemTotal:"),
        kb(&info, "MemAvailable:"),
        kb(&info, "MemFree:"),
        kb(&status, "VmRSS:"),
        mib(layer_peak()),
        mib(LAYER_BYTES)
    ));
    out
}

/// The heap's listing: its size, its free memory (`chunk_mgr_avail` of the header — the chunk manager's own «avail» is
/// address space, the heap runs through an MMU) and its pieces in use summed by owner.
struct Heap {
    size: u64,
    free: u64,
    owners: BTreeMap<String, (u64, u32)>,
}

impl Heap {
    fn parse(text: &str) -> Heap {
        let hex = |s: &str| u64::from_str_radix(s, 16).unwrap_or(0);
        let mut heap = Heap { size: 0, free: 0, owners: BTreeMap::new() };
        let mut chunks = false;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if line.starts_with("each chunk info") {
                chunks = true;
            } else if !chunks && f.len() >= 4 && f[0] == "mma_heap_name0" {
                heap.size = hex(f[2]);
                heap.free = hex(f[3]);
            } else if chunks && f.len() >= 4 && f[2] == "1" {
                // one canvas per piece, named by region handle and canvas: summed as one owner
                let owner = if f[3].starts_with("microcam-rgn") { "microcam-rgn canvases" } else { f[3] };
                let o = heap.owners.entry(owner.to_string()).or_default();
                o.0 += hex(f[1]);
                o.1 += 1;
            }
        }
        heap
    }

    fn used(&self) -> u64 {
        self.size.saturating_sub(self.free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_listing() {
        let text = "       mma heap nameheap_base_cpu_bus_addr              length     chunk_mgr_avail           mmuEnable
      mma_heap_name0            24700000             b000000             6534000                   1
chunk_mgr info:
              offset              length               avail                used            HighPeak             LowPeak
                   0            10000000             b534000             4acc000             5c67000             175f000

each chunk info:
              offset              length           used_flag           task_name                 pid              Module
                   0              300000                   1              CMDMEM                  -1              mi_sys
             2f1b000              bde000                   1         scl0-out0-2                 958              mi_scl
             3af9000              bde000                   1         scl0-out0-2                 958              mi_scl
             2633000              7e9000                   0                  NA                  -1              mi_sys
             46d7000               10000                   1          microcam-rgn3_0                  -1              mi_sys
             46e7000               10000                   1          microcam-rgn3_1                  -1              mi_sys
";
        let h = Heap::parse(text);
        assert_eq!(h.size, 0xb000000);
        assert_eq!(h.free, 0x6534000);
        assert_eq!(h.owners["scl0-out0-2"], (2 * 0xbde000, 2));
        assert_eq!(h.owners["microcam-rgn canvases"], (0x20000, 2));
        assert_eq!(h.owners["CMDMEM"], (0x300000, 1));
        assert!(!h.owners.contains_key("NA"));
    }
}
