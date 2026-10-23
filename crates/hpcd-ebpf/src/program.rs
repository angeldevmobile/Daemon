use aya_ebpf::bindings::bpf_pidns_info;
use aya_ebpf::helpers::{bpf_get_ns_current_pid_tgid, bpf_ktime_get_ns};
use aya_ebpf::macros::{map, tracepoint};
use aya_ebpf::maps::{HashMap, PerCpuArray};
use aya_ebpf::programs::TracePointContext;
use hpcd_ebpf::{BUCKETS, STAT_RECORDED, STAT_RUNNABLE, STAT_SWITCH, STAT_WAKEUP, STATS, bucket};

// Tracepoint field offsets; userspace overrides them from the kernel's format files.
#[unsafe(no_mangle)]
static WAKEUP_PID: u32 = 24;
#[unsafe(no_mangle)]
static SWITCH_PREV_PID: u32 = 24;
#[unsafe(no_mangle)]
static SWITCH_PREV_STATE: u32 = 32;
#[unsafe(no_mangle)]
static SWITCH_NEXT_PID: u32 = 56;
// Our PID namespace; tracepoints report global pids, userspace knows namespaced ones.
#[unsafe(no_mangle)]
static PIDNS_DEV: u64 = 0;
#[unsafe(no_mangle)]
static PIDNS_INO: u64 = 0;

#[map]
static TARGETS: HashMap<u32, u8> = HashMap::with_max_entries(8192, 0);
#[map]
static GLOBAL: HashMap<u32, u8> = HashMap::with_max_entries(8192, 0);
#[map]
static RUNNABLE: HashMap<u32, u64> = HashMap::with_max_entries(8192, 0);
#[map]
static HIST: PerCpuArray<u64> = PerCpuArray::with_max_entries(BUCKETS, 0);
#[map]
static COUNTERS: PerCpuArray<u64> = PerCpuArray::with_max_entries(STATS, 0);

#[inline(always)]
fn count(stat: u32) {
    if let Some(slot) = COUNTERS.get_ptr_mut(stat) {
        unsafe { *slot += 1 };
    }
}

#[inline(always)]
fn offset(field: &u32) -> usize {
    unsafe { core::ptr::read_volatile(field) as usize }
}

#[inline(always)]
fn learn_global(prev: u32) {
    if unsafe { GLOBAL.get(&prev) }.is_some() {
        return;
    }
    let ino = unsafe { core::ptr::read_volatile(&PIDNS_INO) };
    let tid = if ino == 0 {
        prev
    } else {
        let dev = unsafe { core::ptr::read_volatile(&PIDNS_DEV) };
        let mut info = bpf_pidns_info { pid: 0, tgid: 0 };
        let size = core::mem::size_of::<bpf_pidns_info>() as u32;
        if unsafe { bpf_get_ns_current_pid_tgid(dev, ino, &mut info, size) } != 0 {
            return;
        }
        info.pid
    };
    if unsafe { TARGETS.get(&tid) }.is_some() {
        let _ = GLOBAL.insert(&prev, &1, 0);
    }
}

#[inline(always)]
fn mark_runnable(tid: u32) {
    if unsafe { GLOBAL.get(&tid) }.is_some() {
        count(STAT_RUNNABLE);
        let _ = RUNNABLE.insert(&tid, &unsafe { bpf_ktime_get_ns() }, 0);
    }
}

#[inline(always)]
fn on_wakeup(ctx: &TracePointContext) -> u32 {
    count(STAT_WAKEUP);
    if let Ok(tid) = unsafe { ctx.read_at::<u32>(offset(&WAKEUP_PID)) } {
        mark_runnable(tid);
    }
    0
}

#[tracepoint]
pub fn sched_wakeup(ctx: TracePointContext) -> u32 {
    on_wakeup(&ctx)
}

#[tracepoint]
pub fn sched_wakeup_new(ctx: TracePointContext) -> u32 {
    on_wakeup(&ctx)
}

#[tracepoint]
pub fn sched_switch(ctx: TracePointContext) -> u32 {
    count(STAT_SWITCH);
    let fields = unsafe {
        (
            ctx.read_at::<u32>(offset(&SWITCH_PREV_PID)),
            ctx.read_at::<u32>(offset(&SWITCH_PREV_STATE)),
            ctx.read_at::<u32>(offset(&SWITCH_NEXT_PID)),
        )
    };
    let (Ok(prev), Ok(state), Ok(next)) = fields else {
        return 0;
    };
    learn_global(prev);
    // Low byte zero: still runnable (preempted reports TASK_REPORT_MAX, 0x100).
    if state & 0xff == 0 {
        mark_runnable(prev);
    }
    if let Some(&since) = unsafe { RUNNABLE.get(&next) } {
        let waited = unsafe { bpf_ktime_get_ns() }.saturating_sub(since);
        if let Some(slot) = HIST.get_ptr_mut(bucket(waited)) {
            unsafe { *slot += 1 };
        }
        count(STAT_RECORDED);
        let _ = RUNNABLE.remove(&next);
    }
    0
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
