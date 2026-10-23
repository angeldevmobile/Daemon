use aya_ebpf::helpers::bpf_ktime_get_ns;
use aya_ebpf::macros::{map, tracepoint};
use aya_ebpf::maps::{HashMap, PerCpuArray};
use aya_ebpf::programs::TracePointContext;
use hpcd_ebpf::{BUCKETS, bucket};

// Tracepoint field offsets; userspace overrides them from the kernel's format files.
#[unsafe(no_mangle)]
static WAKEUP_PID: u32 = 24;
#[unsafe(no_mangle)]
static SWITCH_PREV_PID: u32 = 24;
#[unsafe(no_mangle)]
static SWITCH_PREV_STATE: u32 = 32;
#[unsafe(no_mangle)]
static SWITCH_NEXT_PID: u32 = 56;

#[map]
static TARGETS: HashMap<u32, u8> = HashMap::with_max_entries(8192, 0);
#[map]
static RUNNABLE: HashMap<u32, u64> = HashMap::with_max_entries(8192, 0);
#[map]
static HIST: PerCpuArray<u64> = PerCpuArray::with_max_entries(BUCKETS, 0);

#[inline(always)]
fn offset(field: &u32) -> usize {
    unsafe { core::ptr::read_volatile(field) as usize }
}

#[inline(always)]
fn mark_runnable(tid: u32) {
    if unsafe { TARGETS.get(&tid) }.is_some() {
        let _ = RUNNABLE.insert(&tid, &unsafe { bpf_ktime_get_ns() }, 0);
    }
}

#[inline(always)]
fn on_wakeup(ctx: &TracePointContext) -> u32 {
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
    // Low byte zero: still runnable (preempted reports TASK_REPORT_MAX, 0x100).
    if state & 0xff == 0 {
        mark_runnable(prev);
    }
    if let Some(&since) = unsafe { RUNNABLE.get(&next) } {
        let waited = unsafe { bpf_ktime_get_ns() }.saturating_sub(since);
        if let Some(slot) = HIST.get_ptr_mut(bucket(waited)) {
            unsafe { *slot += 1 };
        }
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
