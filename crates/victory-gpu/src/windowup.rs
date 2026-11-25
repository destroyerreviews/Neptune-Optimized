//! (Neptune) Pooled upload memory for transient windows (`VICTORY_CARD_WINDOW_UPLOAD=1`,
//! `crate::shadow::window_upload`): a window with a card side keeps no CPU side; the pages
//! that changed are copied into a [`Chunk`] here and sent from it to the card side.
//!
//! A chunk keeps its slab busy until it is dropped. A chunk is dropped when the copy sends it
//! and its plan is let go (`Host::card_done`, after the submission that copied it has run on
//! the card), or earlier if it was never sent (the copy was let go, or another refresh of the
//! same pages came after). A slab with no chunk left is reused from its start.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

const SLAB: u64 = 16 << 20;
const ALIGN: u64 = 256;

fn most() -> u64 {
    static MOST: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MOST.get_or_init(|| {
        std::env::var("VICTORY_CARD_WINDOW_UPLOAD_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(512) << 20
    })
}

struct Slab {
    buffer: ID3D12Resource,
    base: *mut u8,
    size: u64,
    /// Chunks of the slab not dropped yet. Raised only under the pool's lock.
    live: AtomicU64,
    /// Made beyond the cap for one chunk: not kept by the pool.
    overflow: bool,
}

// SAFETY: a D3D12 resource and its persistent mapping; each chunk's bytes are written by one
// thread (its allocator) before the chunk is shared.
unsafe impl Send for Slab {}
// SAFETY: as above.
unsafe impl Sync for Slab {}

impl Drop for Slab {
    fn drop(&mut self) {
        if self.overflow {
            OVERFLOW_LIVE.fetch_sub(self.size, Ordering::Relaxed);
        }
    }
}

/// A piece of a slab, at `offset`.
pub(crate) struct Chunk {
    slab: Arc<Slab>,
    pub(crate) offset: u64,
}

impl Chunk {
    pub(crate) fn ptr(&self) -> *mut u8 {
        // SAFETY: inside the slab's mapping (`alloc`).
        unsafe { self.slab.base.add(self.offset as usize) }
    }

    pub(crate) fn buffer(&self) -> &ID3D12Resource {
        &self.slab.buffer
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        self.slab.live.fetch_sub(1, Ordering::Release);
    }
}

struct Entry {
    slab: Arc<Slab>,
    // Never reuse bytes within a live slab; append only.
    at: u64,
    idle_since: Option<u64>,
}

struct Pool {
    slabs: Vec<Entry>,
    cur: usize,
}

static POOL: Mutex<Pool> = Mutex::new(Pool { slabs: Vec::new(), cur: 0 });

/// Opt-in V3. No changes to the lifetime of a submitted chunk.
fn adaptive() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_ADAPTIVE_MEMORY").is_some_and(|v| v == "1");
        if on {
            tracing::info!("adaptive memory v3: upload slab reuse and idle RAM cache trimming enabled");
        }
        on
    })
}

/// Idle cache allowance (not an allocation limit) and minimum idle time.
/// Pressure uses the machine's physical and commit headroom. Active chunks
/// are never included in this allowance, even when memory is very low.
fn idle_policy(total: u64, available: u64, commit: u64) -> (u64, u64) {
    const MIB: u64 = 1 << 20;
    let critical = (total / 16).max(1024 * MIB).min(4096 * MIB);
    let moderate = (total / 8).max(2048 * MIB).min(8192 * MIB);
    if available < critical || commit < 1024 * MIB {
        (16 * MIB, 1000)
    } else if available < moderate || commit < 2048 * MIB {
        (64 * MIB, 3000)
    } else {
        (128 * MIB, 8000)
    }
}

fn fitting_tail(remaining: impl Iterator<Item = (usize, u64)>, n: u64) -> Option<usize> {
    remaining.filter(|&(_, bytes)| bytes >= n).min_by_key(|&(i, bytes)| (bytes, i)).map(|(i, _)| i)
}

/// Called from the existing once-a-second sweep. A slab with a live chunk
/// (including every submitted chunk) is never trimmed. Dropping the COM
/// objects outside the pool lock avoids blocking allocation on destruction.
pub(crate) fn trim_idle() {
    if !adaptive() {
        return;
    }
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: a correctly sized, writable MEMORYSTATUSEX.
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return;
    }
    let (keep, age) = idle_policy(m.ullTotalPhys, m.ullAvailPhys, m.ullAvailPageFile);
    let now = crate::host::now_ms();
    let mut dropped = Vec::new();
    let mut reclaimed = 0u64;
    {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut idle_bytes = 0u64;
        for entry in &mut pool.slabs {
            if entry.slab.live.load(Ordering::Acquire) == 0 && Arc::strong_count(&entry.slab) == 1 {
                idle_bytes += entry.slab.size;
                entry.idle_since.get_or_insert(now);
            } else {
                entry.idle_since = None;
            }
        }
        // Oldest first. Retain recent idle slabs for reuse.
        let mut eligible: Vec<_> = pool.slabs.iter().enumerate().filter_map(|(i, entry)| {
            let since = entry.idle_since?;
            (now.saturating_sub(since) >= age).then_some((since, i))
        }).collect();
        eligible.sort_unstable();
        let mut remove = vec![false; pool.slabs.len()];
        for (_, i) in eligible {
            if idle_bytes <= keep {
                break;
            }
            let size = pool.slabs[i].slab.size;
            remove[i] = true;
            idle_bytes = idle_bytes.saturating_sub(size);
            reclaimed += size;
        }
        // Allocation and increasing live are protected by POOL. A chunk
        // can only disappear in parallel, never appear in a selected slab.
        if reclaimed != 0 {
            let entries = std::mem::take(&mut pool.slabs);
            for (i, entry) in entries.into_iter().enumerate() {
                if remove[i] {
                    dropped.push(entry);
                } else {
                    pool.slabs.push(entry);
                }
            }
            pool.cur = usize::MAX;
            POOLED.fetch_sub(reclaimed, Ordering::Relaxed);
        }
    }
    drop(dropped);
    if reclaimed != 0 {
        tracing::info!(released_mb = reclaimed >> 20, pooled_mb = POOLED.load(Ordering::Relaxed) >> 20,
            idle_target_mb = keep >> 20, "adaptive memory v3: idle upload slabs released");
    }
}
/// Bytes of slabs in the pool (peak), of overflow slabs alive (now), allocations that needed one.
static POOLED: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_LIVE: AtomicU64 = AtomicU64::new(0);
static OVERFLOWS: AtomicU64 = AtomicU64::new(0);
/// Bytes sent from chunks since the last [`stats`].
pub(crate) static SENT: AtomicU64 = AtomicU64::new(0);

fn make(size: u64, overflow: bool) -> Option<Arc<Slab>> {
    let (buffer, base) = crate::host::host()?.upload_buffer(size)?;
    Some(Arc::new(Slab { buffer, base, size, live: AtomicU64::new(0), overflow }))
}

/// `n` bytes of upload memory. Past the cap a slab of its own is made (counted and logged:
/// nothing blocks, and nothing is overwritten while the card may read it); `None` only if
/// the device gives no memory.
pub(crate) fn alloc(n: u64) -> Option<Chunk> {
    let n = n.next_multiple_of(ALIGN);
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let pool = &mut *pool;
    if n <= SLAB {
        // The slab in hand (a slab with no chunk left starts again), then any idle one, then a new one.
        if let Some(entry) = pool.slabs.get_mut(pool.cur) {
            if entry.slab.live.load(Ordering::Acquire) == 0 {
                entry.at = 0;
            }
        }
        let fits = |pool: &Pool| pool.slabs.get(pool.cur).is_some_and(|e| n <= e.slab.size.saturating_sub(e.at));
        if !fits(pool) {
            // V3 fills unused tails of existing slabs before opening another
            // one. The cursor is per slab, so no live bytes are overwritten.
            let tail = if adaptive() {
                fitting_tail(pool.slabs.iter().enumerate().filter_map(|(i, e)| {
                    (e.slab.live.load(Ordering::Acquire) != 0)
                        .then_some((i, e.slab.size.saturating_sub(e.at)))
                }), n)
            } else {
                None
            };
            if let Some(i) = tail {
                pool.cur = i;
            } else if let Some(i) = pool.slabs.iter().position(|e| e.slab.live.load(Ordering::Acquire) == 0) {
                pool.cur = i;
                pool.slabs[i].at = 0;
            } else if POOLED.load(Ordering::Relaxed) + SLAB <= most() {
                if let Some(slab) = make(SLAB, false) {
                    let total = POOLED.fetch_add(SLAB, Ordering::Relaxed) + SLAB;
                    PEAK.fetch_max(total, Ordering::Relaxed);
                    pool.slabs.push(Entry { slab, at: 0, idle_since: None });
                    pool.cur = pool.slabs.len() - 1;
                }
            }
        }
        if fits(pool) {
            let entry = &mut pool.slabs[pool.cur];
            let slab = entry.slab.clone();
            slab.live.fetch_add(1, Ordering::Relaxed);
            let offset = entry.at;
            entry.at += n;
            entry.idle_since = None;
            return Some(Chunk { slab, offset });
        }
    }
    let said = OVERFLOWS.fetch_add(1, Ordering::Relaxed) + 1;
    if said <= 16 || said.is_power_of_two() {
        tracing::warn!(
            said,
            bytes = n,
            pooled_mb = POOLED.load(Ordering::Relaxed) >> 20,
            "window upload pool is full: a slab of its own (VICTORY_CARD_WINDOW_UPLOAD)"
        );
    }
    let slab = make(n, true)?;
    OVERFLOW_LIVE.fetch_add(n, Ordering::Relaxed);
    slab.live.fetch_add(1, Ordering::Relaxed);
    Some(Chunk { slab, offset: 0 })
}

/// For the ram line: pool bytes now, peak, overflow slabs alive (bytes), overflow allocations so far.
pub(crate) fn stats() -> [u64; 4] {
    [
        POOLED.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        OVERFLOW_LIVE.load(Ordering::Relaxed),
        OVERFLOWS.load(Ordering::Relaxed),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_keeps_less_idle_ram_under_pressure() {
        let gib = 1u64 << 30;
        let mib = 1u64 << 20;
        assert_eq!(idle_policy(32*gib, 8*gib, 8*gib), (128*mib, 8000));
        assert_eq!(idle_policy(32*gib, 3*gib, 8*gib), (64*mib, 3000));
        assert_eq!(idle_policy(32*gib, gib, 8*gib), (16*mib, 1000));
        assert_eq!(idle_policy(64*gib, 16*gib, 512*mib), (16*mib, 1000));
    }

    #[test]
    fn reuses_smallest_fitting_unused_tail() {
        assert_eq!(fitting_tail([(0, 8), (1, 4), (2, 6)].into_iter(), 5), Some(2));
        assert_eq!(fitting_tail([(0, 1), (1, 2)].into_iter(), 3), None);
        assert_eq!(fitting_tail([(3, 4), (1, 4)].into_iter(), 4), Some(1));
    }
}
