//! Vita system calls behind the platform layer: pad and touch polling, power clocks,
//! thread placement and the idle timers.
use crate::controls::{self, Area, Pad, Settings, Touch};
use skate_core::input::xbox::XboxState;
use vitasdk_sys::*;

/// Most touches either panel reports at once (`SCE_TOUCH_MAX_REPORT`).
pub const MAX_TOUCHES: usize = 8;

/// One input sample: the pad and the touches on both panels.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub pad: Pad,
    pub front: [Touch; MAX_TOUCHES],
    pub front_count: usize,
    pub rear: [Touch; MAX_TOUCHES],
    pub rear_count: usize,
}
impl Sample {
    pub fn front(&self) -> &[Touch] {
        &self.front[..self.front_count]
    }
    pub fn rear(&self) -> &[Touch] {
        &self.rear[..self.rear_count]
    }
}

/// Pad and touch input, set up for the controls mapping (analog wide sampling, both
/// touch panels sampling).
pub struct Input {
    pub front_area: Area,
    pub rear_area: Area,
}

impl Input {
    pub fn new() -> Self {
        unsafe {
            sceCtrlSetSamplingMode(SCE_CTRL_MODE_ANALOG_WIDE);
            sceTouchSetSamplingState(SCE_TOUCH_PORT_FRONT, SCE_TOUCH_SAMPLING_STATE_START);
            sceTouchSetSamplingState(SCE_TOUCH_PORT_BACK, SCE_TOUCH_SAMPLING_STATE_START);
        }
        Self {
            front_area: panel_area(SCE_TOUCH_PORT_FRONT),
            rear_area: panel_area(SCE_TOUCH_PORT_BACK),
        }
    }

    /// The latest pad and touch state (non-blocking; call once per sim tick).
    pub fn poll(&self) -> Sample {
        let mut ctrl: SceCtrlData = unsafe { core::mem::zeroed() };
        unsafe { sceCtrlPeekBufferPositive(0, &mut ctrl, 1) };
        let (front, front_count) = touches(SCE_TOUCH_PORT_FRONT);
        let (rear, rear_count) = touches(SCE_TOUCH_PORT_BACK);
        Sample {
            pad: Pad {
                buttons: ctrl.buttons,
                lx: ctrl.lx,
                ly: ctrl.ly,
                rx: ctrl.rx,
                ry: ctrl.ry,
            },
            front,
            front_count,
            rear,
            rear_count,
        }
    }

    /// The controls mapping of a sample.
    pub fn xbox(&self, sample: &Sample, settings: &Settings) -> XboxState {
        controls::map(
            sample.pad,
            sample.front(),
            self.front_area,
            sample.rear(),
            self.rear_area,
            settings,
        )
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

fn panel_area(port: SceUInt32) -> Area {
    let mut info: SceTouchPanelInfo = unsafe { core::mem::zeroed() };
    unsafe { sceTouchGetPanelInfo(port, &mut info) };
    Area {
        min_x: info.minAaX,
        min_y: info.minAaY,
        max_x: info.maxAaX,
        max_y: info.maxAaY,
    }
}

fn touches(port: SceUInt32) -> ([Touch; MAX_TOUCHES], usize) {
    let mut data: SceTouchData = unsafe { core::mem::zeroed() };
    let mut out = [Touch { x: 0, y: 0 }; MAX_TOUCHES];
    if unsafe { sceTouchPeek(port, &mut data, 1) } < 1 {
        return (out, 0);
    }
    let count = (data.reportNum as usize).min(MAX_TOUCHES);
    for (slot, report) in out.iter_mut().zip(&data.report[..count]) {
        *slot = Touch {
            x: report.x,
            y: report.y,
        };
    }
    (out, count)
}

/// The benchmark clocks: ARM 444, bus 222, GPU 222, crossbar 166 MHz. Returns the
/// clocks the system reports afterwards.
pub fn set_clocks() -> [i32; 4] {
    unsafe {
        scePowerSetArmClockFrequency(444);
        scePowerSetBusClockFrequency(222);
        scePowerSetGpuClockFrequency(222);
        scePowerSetGpuXbarClockFrequency(166);
        [
            scePowerGetArmClockFrequency(),
            scePowerGetBusClockFrequency(),
            scePowerGetGpuClockFrequency(),
            scePowerGetGpuXbarClockFrequency(),
        ]
    }
}

/// Pins the calling thread to one user core (0–2). Returns the system's result code.
pub fn pin_current_thread(core: u32) -> i32 {
    let mask = match core {
        0 => SCE_KERNEL_CPU_MASK_USER_0,
        1 => SCE_KERNEL_CPU_MASK_USER_1,
        _ => SCE_KERNEL_CPU_MASK_USER_2,
    };
    unsafe { sceKernelChangeThreadCpuAffinityMask(sceKernelGetThreadId(), mask as i32) }
}

/// Lets the calling thread run on any of `cores` (user cores 0–2): the scheduler puts it on
/// whichever is idle. Returns the system's result code.
pub fn allow_current_thread_on(cores: &[u32]) -> i32 {
    let mask = cores.iter().fold(0, |m, &c| {
        m | match c {
            0 => SCE_KERNEL_CPU_MASK_USER_0,
            1 => SCE_KERNEL_CPU_MASK_USER_1,
            _ => SCE_KERNEL_CPU_MASK_USER_2,
        }
    });
    unsafe { sceKernelChangeThreadCpuAffinityMask(sceKernelGetThreadId(), mask as i32) }
}

/// Changes the calling thread's priority (lower is more urgent; user threads use
/// 64–191, the default is 160). Returns the system's result code.
pub fn set_current_thread_priority(priority: i32) -> i32 {
    unsafe { sceKernelChangeThreadPriority(sceKernelGetThreadId(), priority) }
}

/// Resets the idle timers, so the screen does not dim and the Vita does not sleep.
/// Gameplay with only stick input does not count as activity otherwise.
pub fn keep_awake() {
    unsafe { sceKernelPowerTick(SCE_KERNEL_POWER_TICK_DEFAULT) };
}

/// The calling thread's FPSCR (0 = IEEE mode, see tools/vita-fpu).
pub fn fpscr() -> u32 {
    let value: u32;
    unsafe { core::arch::asm!("vmrs {}, fpscr", out(reg) value) };
    value
}

/// Audio output frames per buffer (5.3 ms at 48 kHz).
pub const AUDIO_FRAMES: usize = 256;
pub const AUDIO_RATE: u32 = 48_000;

/// Starts the audio thread: one SceAudio main port, 48 kHz stereo 16-bit.
/// `fill` writes `AUDIO_FRAMES` interleaved stereo frames per call; output blocks on
/// the hardware, so `fill` runs every 5.3 ms. Returns false if the port did not open.
pub fn start_audio(mut fill: impl FnMut(&mut [i16]) + Send + 'static) -> bool {
    let port = unsafe {
        sceAudioOutOpenPort(
            SCE_AUDIO_OUT_PORT_TYPE_MAIN,
            AUDIO_FRAMES as i32,
            AUDIO_RATE as i32,
            SCE_AUDIO_OUT_MODE_STEREO,
        )
    };
    if port < 0 {
        println!("sceAudioOutOpenPort failed: {port:#x}");
        return false;
    }
    std::thread::Builder::new()
        .name("audio".into())
        // The retail audio host runs here too (the reference ran it on an 8 MB main thread).
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            pin_current_thread(2);
            set_current_thread_priority(64);
            let mut buffers = [[0i16; AUDIO_FRAMES * 2]; 2];
            let mut which = 0;
            let mut errors = 0u32;
            loop {
                fill(&mut buffers[which]);
                let started = std::time::Instant::now();
                let result = unsafe { sceAudioOutOutput(port, buffers[which].as_ptr().cast()) };
                // A failing port must not spin this core, and a suspend shows up as a long call.
                if result < 0 {
                    errors += 1;
                    if errors == 1 {
                        println!("audio: sceAudioOutOutput failed: {result:#x}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                } else if errors > 0 {
                    println!("audio: output recovered after {errors} failed calls");
                    errors = 0;
                }
                if started.elapsed() > std::time::Duration::from_millis(250) {
                    println!("SUSPEND audio thread: output call took {} ms", started.elapsed().as_millis());
                }
                which ^= 1;
            }
        })
        .is_ok()
}

/// newlib's heap statistics (`mallinfo`): (bytes in use, arena size). Every Rust and C
/// allocation on the Vita goes through this malloc.
pub fn heap() -> (usize, usize) {
    let (used, arena, _, _) = heap_detail();
    (used, arena)
}

/// `mallinfo` with the free side: (bytes in use, arena size, bytes free, free chunks).
pub fn heap_detail() -> (usize, usize, usize, usize) {
    #[repr(C)]
    struct Mallinfo {
        arena: usize,
        ordblks: usize,
        smblks: usize,
        hblks: usize,
        hblkhd: usize,
        usmblks: usize,
        fsmblks: usize,
        uordblks: usize,
        fordblks: usize,
        keepcost: usize,
    }
    unsafe extern "C" {
        fn mallinfo() -> Mallinfo;
    }
    let m = unsafe { mallinfo() };
    (m.uordblks, m.arena, m.fordblks, m.ordblks)
}

/// The pad alone (menus on the main thread; `Input::poll` stays the gameplay path).
pub fn pad() -> Pad {
    let mut ctrl: SceCtrlData = unsafe { core::mem::zeroed() };
    unsafe { sceCtrlPeekBufferPositive(0, &mut ctrl, 1) };
    Pad {
        buttons: ctrl.buttons,
        lx: ctrl.lx,
        ly: ctrl.ly,
        rx: ctrl.rx,
        ry: ctrl.ry,
    }
}
