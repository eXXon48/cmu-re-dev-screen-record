//! Kernel interfaces: fb0 geometry, the Freescale IPU task device (kernel 3.0.35 BSP,
//! `include/linux/ipu.h`), signals. No crates: libc symbols declared here.

use std::ffi::c_int;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};

extern "C" {
    fn ioctl(fd: c_int, req: c_int, ...) -> c_int;
    fn signal(sig: c_int, handler: usize) -> usize;
    fn dup(fd: c_int) -> c_int;
    fn dup2(fd: c_int, to: c_int) -> c_int;
}

/// Take stdout for the video and point fd 1 at stderr: libvpu prints its errors
/// with printf, which must not end up inside the H.264 stream.
pub fn steal_stdout() -> io::Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain fd syscalls; the dup'ed fd is owned only by the returned File.
    unsafe {
        let fd = dup(1);
        if fd < 0 || dup2(2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(fd))
    }
}

const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    dir << 30 | (size as u32) << 16 | (b'I' as u32) << 8 | nr
}
const W: u32 = 1;
const RW: u32 = 3;
pub const IPU_CHECK_TASK: u32 = ioc(RW, 1, std::mem::size_of::<IpuTask>());
pub const IPU_QUEUE_TASK: u32 = ioc(W, 2, std::mem::size_of::<IpuTask>());
pub const IPU_ALLOC: u32 = ioc(RW, 3, 4);
pub const IPU_FREE: u32 = ioc(W, 4, 4);
const IPU_CHECK_ERR_MIN: c_int = 4;
const FBIOGET_VSCREENINFO: u32 = 0x4600;
const FBIOGET_FSCREENINFO: u32 = 0x4602;

const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}
/// fb0 bytes are B,G,R,X. ponytail: if the bench shows red/blue swapped, use fourcc(b"RGB4").
pub const FMT_FB: u32 = fourcc(b"BGR4");
pub const FMT_I420: u32 = fourcc(b"I420");

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Crop { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Deinterlace { enable: u8, motion: u8, field_fmt: u8 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IpuInput {
    pub width: u32, pub height: u32, pub format: u32, pub crop: Crop, pub paddr: u32,
    deinterlace: Deinterlace, paddr_n: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IpuOutput {
    pub width: u32, pub height: u32, pub format: u32, pub rotate: u8, pub crop: Crop, pub paddr: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Alpha { mode: u8, gvalue: u8, loc_alp_paddr: u32 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Colorkey { enable: u8, value: u32 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Overlay { width: u32, height: u32, format: u32, crop: Crop, alpha: Alpha, colorkey: Colorkey, paddr: u32 }

/// `struct ipu_task`, 136 bytes (stock mfw_ipucsc uses IPU_QUEUE_TASK = 0x40884902).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IpuTask {
    pub input: IpuInput,
    pub output: IpuOutput,
    overlay_en: u8,
    overlay: Overlay,
    priority: u8,
    task_id: u8,
    timeout: c_int,
}

impl IpuTask {
    /// Whole-frame colour conversion fb page -> I420, no scaling.
    pub fn csc(w: u32, h: u32, in_paddr: u32, out_paddr: u32) -> IpuTask {
        let crop = Crop { x: 0, y: 0, w, h };
        IpuTask {
            input: IpuInput { width: w, height: h, format: FMT_FB, crop, paddr: in_paddr, ..Default::default() },
            output: IpuOutput { width: w, height: h, format: FMT_I420, rotate: 0, crop, paddr: out_paddr },
            timeout: 1000,
            ..Default::default()
        }
    }
}

fn ctl<T>(f: &File, req: u32, arg: *mut T) -> io::Result<c_int> {
    // SAFETY: arg points at a live value of the type the request expects.
    let r = unsafe { ioctl(f.as_raw_fd(), req as c_int, arg) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

pub struct Fb {
    file: File,
    pub w: u32,
    pub h: u32,
    pub smem_start: u32,
    pub line_length: u32,
}

impl Fb {
    pub fn open(path: &str) -> io::Result<Fb> {
        let file = File::open(path)?;
        let mut var = [0u32; 40]; // fb_var_screeninfo, 160 bytes
        ctl(&file, FBIOGET_VSCREENINFO, var.as_mut_ptr())?;
        let mut fix = [0u32; 32]; // fb_fix_screeninfo (68 bytes on 32-bit ARM)
        ctl(&file, FBIOGET_FSCREENINFO, fix.as_mut_ptr())?;
        if var[6] != 32 {
            return Err(io::Error::other(format!("{path}: {} bpp, need 32", var[6])));
        }
        // 32-bit layout: smem_start @16, line_length @44
        Ok(Fb { file, w: var[0], h: var[1], smem_start: fix[4], line_length: fix[11] })
    }

    /// Physical address of the page on screen now (double-buffered: yoffset 0 or h).
    pub fn page_paddr(&self) -> io::Result<u32> {
        let mut var = [0u32; 40];
        ctl(&self.file, FBIOGET_VSCREENINFO, var.as_mut_ptr())?;
        Ok(self.smem_start + var[5] * self.line_length)
    }
}

pub struct Ipu(File);

impl Ipu {
    pub fn open() -> io::Result<Ipu> {
        Ok(Ipu(OpenOptions::new().read(true).write(true).open("/dev/mxc_ipu")?))
    }

    /// Physically contiguous DMA buffer for the IPU output (the VPU reads it directly).
    pub fn alloc(&self, len: usize) -> io::Result<IpuBuf<'_>> {
        let mut paddr = len as c_int;
        ctl(&self.0, IPU_ALLOC, &mut paddr)?;
        Ok(IpuBuf { ipu: self, paddr: paddr as u32 })
    }

    /// Validate a task once; warnings (misaligned offsets) are fine.
    pub fn check(&self, t: &IpuTask) -> io::Result<c_int> {
        let mut t = *t;
        let r = ctl(&self.0, IPU_CHECK_TASK, &mut t)?;
        if r > IPU_CHECK_ERR_MIN { Err(io::Error::other(format!("IPU_CHECK_TASK: {r}"))) } else { Ok(r) }
    }

    /// Run a task; blocks until the IPU is done.
    pub fn queue(&self, t: &IpuTask) -> io::Result<()> {
        let mut t = *t;
        ctl(&self.0, IPU_QUEUE_TASK, &mut t).map(|_| ())
    }
}

pub struct IpuBuf<'a> {
    ipu: &'a Ipu,
    pub paddr: u32,
}

impl Drop for IpuBuf<'_> {
    fn drop(&mut self) {
        let mut p = self.paddr as c_int;
        let _ = ctl(&self.ipu.0, IPU_FREE, &mut p);
    }
}

pub static STOP: AtomicBool = AtomicBool::new(false);

/// SIGHUP (ssh closed), SIGINT, SIGTERM end the recording.
pub fn stop_signals() -> [c_int; 3] {
    [1, 2, 15]
}

extern "C" fn on_stop(_: c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn install_signals() {
    for s in stop_signals() {
        // SAFETY: handler only stores an atomic.
        unsafe { signal(s, on_stop as extern "C" fn(c_int) as usize) };
    }
    // SIGPIPE ignored: a closed reader shows up as EPIPE from write.
    unsafe { signal(13, 1 /*SIG_IGN*/) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn ipu_task_layout() {
        assert_eq!(size_of::<IpuTask>(), 136);
        assert_eq!(offset_of!(IpuTask, input.paddr), 28);
        assert_eq!(offset_of!(IpuTask, input.paddr_n), 36);
        assert_eq!(offset_of!(IpuTask, output), 40);
        assert_eq!(offset_of!(IpuTask, output.crop), 56);
        assert_eq!(offset_of!(IpuTask, output.paddr), 72);
        assert_eq!(offset_of!(IpuTask, overlay_en), 76);
        assert_eq!(offset_of!(IpuTask, overlay), 80);
        assert_eq!(offset_of!(IpuTask, priority), 128);
        assert_eq!(offset_of!(IpuTask, timeout), 132);
    }

    #[test]
    fn ioctl_numbers() {
        assert_eq!(IPU_CHECK_TASK, 0xC088_4901);
        assert_eq!(IPU_QUEUE_TASK, 0x4088_4902);
        assert_eq!(IPU_ALLOC, 0xC004_4903);
        assert_eq!(IPU_FREE, 0x4004_4904);
    }

    #[test]
    fn formats() {
        assert_eq!(FMT_FB, 0x3452_4742);
        assert_eq!(FMT_I420, 0x3032_3449);
    }

    #[test]
    fn csc_task() {
        let t = IpuTask::csc(800, 480, 0x1000_0000, 0x2000_0000);
        assert_eq!((t.input.crop.w, t.input.crop.h, t.output.crop.w), (800, 480, 800));
        assert_eq!((t.input.paddr, t.output.paddr, t.timeout, t.overlay_en), (0x1000_0000, 0x2000_0000, 1000, 0));
    }

    #[test]
    fn stop_signals_include_hup() {
        assert_eq!(stop_signals(), [1, 2, 15]);
    }
}

/// glibc 2.11 on the unit has no getauxval (2.16+); Rust std only uses it for the
/// alternate signal stack size, where 0 means "use the default".
#[cfg(target_env = "gnu")]
#[no_mangle]
pub extern "C" fn getauxval(_type: std::ffi::c_ulong) -> std::ffi::c_ulong {
    0
}
