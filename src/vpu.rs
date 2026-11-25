//! H.264 encoder on the i.MX6 VPU through the unit's own `libvpu.so.4` (imx-lib vpulib 5.4.10),
//! loaded at run time. The VPU reads the I420 source frame by DMA from a physical address,
//! so the CPU never touches pixels; it only copies the small encoded bitstream out.
//!
//! Struct layouts follow imx-lib `vpu/vpu_lib.h` and `vpu/vpu_io.h` at VPU_LIB_VERSION(5, 4, 10).
//! Pointer fields are kept as u32: the unit is 32-bit ARM, and the layout tests then hold on any host.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::io::{self, Write};

const STD_AVC: c_int = 2;
const ENC_PUT_AVC_HEADER: c_int = 15;
const SPS_RBSP: c_int = 0;
const PPS_RBSP: c_int = 1;
const BS_SIZE: u32 = 0x20_0000; // 2 MiB, same as imx-test
const BUSY_TRIES: u32 = 20; // x 200 ms, then give up (imx-test does the same)

#[repr(C)]
#[derive(Default)]
pub struct MemDesc { size: c_int, phy_addr: u32, cpu_addr: u32, virt_uaddr: u32 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FrameBuffer {
    stride_y: u32, stride_c: u32, my_index: c_int,
    buf_y: u32, buf_cb: u32, buf_cr: u32, buf_mv_col: u32,
}

impl FrameBuffer {
    /// Planar I420 at `paddr`, `w` x `h`.
    pub fn i420(paddr: u32, w: u32, h: u32, index: c_int) -> FrameBuffer {
        let y = w * h;
        FrameBuffer {
            stride_y: w, stride_c: w / 2, my_index: index,
            buf_y: paddr, buf_cb: paddr + y, buf_cr: paddr + y + y / 4, buf_mv_col: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SliceMode { slice_mode: c_int, slice_size_mode: c_int, slice_size: c_int }

/// `EncAvcParam`, the AVC member of the `EncStdParam` union.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AvcParam {
    constrained_intra_pred: c_int, disable_deblk: c_int, deblk_alpha: c_int, deblk_beta: c_int,
    chroma_qp_offset: c_int, aud_enable: c_int, fmo_enable: c_int, fmo_slice_num: c_int,
    fmo_type: c_int, fmo_slice_save_buf_size: c_int, frame_cropping: c_int,
    crop_left: c_int, crop_right: c_int, crop_top: c_int, crop_bottom: c_int,
    mvc_extension: c_int, interview_en: c_int, paraset_refresh_en: c_int, prefix_nal_en: c_int,
}

/// The union is as big as its largest member, `EncMjpgParam` (1980 bytes).
const STD_PARAM_WORDS: usize = 1980 / 4;

#[repr(C)]
pub struct OpenParam {
    bitstream_buffer: u32, bitstream_buffer_size: u32, bitstream_format: c_int,
    pic_width: c_int, pic_height: c_int, frame_rate_info: u32, bit_rate: c_int,
    initial_delay: c_int, vbv_buffer_size: c_int, enable_auto_skip: c_int, gop_size: c_int,
    linear2tiled: c_int, map_type: c_int, slicemode: SliceMode, intra_refresh: c_int,
    slice_report: c_int, mb_report: c_int, mb_qp_report: c_int, rc_intra_qp: c_int,
    chroma_interleave: c_int, dynamic_alloc: c_int, ring_buffer_enable: c_int,
    std_param: [u32; STD_PARAM_WORDS],
    user_qp_min: c_int, user_qp_max: c_int, user_qp_min_enable: c_int, user_qp_max_enable: c_int,
    user_gamma: u32, rc_interval_mode: c_int, mb_interval: c_int, avc_intra16x16_only: c_int,
    me_search_range: c_int, me_use_zero_pmv: c_int, intra_cost_weight: c_int,
}

impl OpenParam {
    /// H.264, linear frames, line-buffer bitstream mode, constant bitrate in kbit/s.
    pub fn avc(bs_paddr: u32, w: u32, h: u32, fps: u32, kbps: u32) -> OpenParam {
        let mut p = OpenParam {
            bitstream_buffer: bs_paddr, bitstream_buffer_size: BS_SIZE, bitstream_format: STD_AVC,
            pic_width: w as c_int, pic_height: h as c_int, frame_rate_info: fps, bit_rate: kbps as c_int,
            initial_delay: 0, vbv_buffer_size: 0,
            enable_auto_skip: 0, // keep every frame: steady fps beats an exact bitrate
            gop_size: fps as c_int, // one IDR a second, so a cut clip still decodes quickly
            linear2tiled: 0, map_type: 0, slicemode: SliceMode::default(), intra_refresh: 0,
            slice_report: 0, mb_report: 0, mb_qp_report: 0, rc_intra_qp: -1,
            chroma_interleave: 0, dynamic_alloc: 0, ring_buffer_enable: 0,
            std_param: [0; STD_PARAM_WORDS],
            user_qp_min: 0, user_qp_max: 0, user_qp_min_enable: 0, user_qp_max_enable: 0,
            user_gamma: 24576, // 0.75 * 32768, imx-test default
            rc_interval_mode: 1, mb_interval: 0, avc_intra16x16_only: 0,
            me_search_range: 3, me_use_zero_pmv: 0, intra_cost_weight: 0,
        };
        let avc = AvcParam::default(); // deblocking on, no crop, no MVC
        // SAFETY: AvcParam is plain ints and smaller than the union storage.
        unsafe { std::ptr::write(p.std_param.as_mut_ptr() as *mut AvcParam, avc) };
        p
    }
}

#[repr(C)]
#[derive(Default)]
struct InitialInfo { min_frame_buffer_count: c_int, report_buf_size: [c_int; 3] }

#[repr(C)]
#[derive(Default)]
struct ExtBufInfo { sub_samp_a_mvc: u32, sub_samp_b_mvc: u32, scratch_base: u32, scratch_size: c_int }

#[repr(C)]
#[derive(Default)]
pub struct EncParam {
    source_frame: u32, top_offset: c_int, left_offset: c_int, force_i_picture: c_int,
    skip_picture: c_int, quant_param: c_int, pic_stream_buffer_addr: u32,
    pic_stream_buffer_size: c_int, enable_auto_skip: c_int,
}

#[repr(C)]
#[derive(Default)]
struct ReportInfo { enable: c_int, kind: c_int, size: c_int, addr: u32 }

#[repr(C)]
#[derive(Default)]
pub struct OutputInfo {
    bitstream_buffer: u32, bitstream_size: u32, wrap_around: c_int, skip_encoded: c_int,
    pic_type: c_int, num_of_slices: c_int, recon_frame_index: c_int,
    p_slice_info: u32, p_mb_info: u32, p_mb_qp_info: u32,
    mb_info: ReportInfo, mv_info: ReportInfo, slice_info: ReportInfo,
}

#[repr(C)]
#[derive(Default)]
pub struct HeaderParam { buf: u32, size: c_int, header_type: c_int, user_profile_level_enable: c_int, user_profile_level: c_int }

type Handle = *mut c_void;

#[link(name = "dl")]
extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

macro_rules! api {
    ($($f:ident : fn($($a:ty),*) -> $r:ty;)*) => {
        #[allow(non_snake_case)]
        struct Api { $($f: unsafe extern "C" fn($($a),*) -> $r,)* }
        impl Api {
            fn load() -> io::Result<Api> {
                // SAFETY: dlopen/dlsym with NUL-terminated names; each symbol has the C type above.
                unsafe {
                    let lib = dlopen(c"libvpu.so.4".as_ptr(), 2 /*RTLD_NOW*/);
                    if lib.is_null() {
                        return Err(io::Error::other(format!("dlopen libvpu.so.4: {}", CStr::from_ptr(dlerror()).to_string_lossy())));
                    }
                    Ok(Api { $($f: {
                        let p = dlsym(lib, concat!(stringify!($f), "\0").as_ptr() as *const c_char);
                        if p.is_null() { return Err(io::Error::other(concat!("libvpu: no ", stringify!($f)))); }
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($a),*) -> $r>(p)
                    },)* })
                }
            }
        }
    };
}

api! {
    vpu_Init: fn(*mut c_void) -> c_int;
    vpu_UnInit: fn() -> ();
    vpu_EncOpen: fn(*mut Handle, *mut OpenParam) -> c_int;
    vpu_EncClose: fn(Handle) -> c_int;
    vpu_EncGetInitialInfo: fn(Handle, *mut InitialInfo) -> c_int;
    vpu_EncRegisterFrameBuffer: fn(Handle, *mut FrameBuffer, c_int, c_int, c_int, u32, u32, *mut ExtBufInfo) -> c_int;
    vpu_EncStartOneFrame: fn(Handle, *mut EncParam) -> c_int;
    vpu_EncGetOutputInfo: fn(Handle, *mut OutputInfo) -> c_int;
    vpu_EncGiveCommand: fn(Handle, c_int, *mut c_void) -> c_int;
    vpu_IsBusy: fn() -> c_int;
    vpu_WaitForInt: fn(c_int) -> c_int;
    IOGetPhyMem: fn(*mut MemDesc) -> c_int;
    IOFreePhyMem: fn(*mut MemDesc) -> c_int;
    IOGetVirtMem: fn(*mut MemDesc) -> c_int;
    IOFreeVirtMem: fn(*mut MemDesc) -> c_int;
}

fn rc(what: &str, r: c_int) -> io::Result<()> {
    if r == 0 { Ok(()) } else { Err(io::Error::other(format!("{what}: RetCode {r}"))) }
}

pub struct Encoder {
    api: Api,
    handle: Handle,
    bs: MemDesc,
    frames: Vec<MemDesc>, // reconstruction + 2 subsample buffers, owned by the VPU
    w: u32,
    h: u32,
}

impl Encoder {
    pub fn open(w: u32, h: u32, fps: u32, kbps: u32) -> io::Result<Encoder> {
        if !w.is_multiple_of(16) || !h.is_multiple_of(16) {
            return Err(io::Error::other(format!("{w}x{h}: VPU needs a multiple of 16")));
        }
        let api = Api::load()?;
        // SAFETY (whole block): calls follow the imx-test mxc_vpu_test encoder sequence
        // with structs laid out as in vpu_lib.h 5.4.10.
        unsafe {
            rc("vpu_Init", (api.vpu_Init)(std::ptr::null_mut()))?;
            let mut e = Encoder { api, handle: std::ptr::null_mut(), bs: MemDesc::default(), frames: Vec::new(), w, h };
            e.bs = e.phys(BS_SIZE)?;
            let mut op = OpenParam::avc(e.bs.phy_addr, w, h, fps, kbps);
            rc("vpu_EncOpen", (e.api.vpu_EncOpen)(&mut e.handle, &mut op))?;
            let mut info = InitialInfo::default();
            rc("vpu_EncGetInitialInfo", (e.api.vpu_EncGetInitialInfo)(e.handle, &mut info))?;
            let n = info.min_frame_buffer_count.max(0) as usize;
            let mut fbs = Vec::new();
            for i in 0..n + 2 {
                let m = e.phys(w * h * 3 / 2)?;
                fbs.push(FrameBuffer::i420(m.phy_addr, w, h, i as c_int));
                e.frames.push(m);
            }
            let mut ext = ExtBufInfo::default();
            rc("vpu_EncRegisterFrameBuffer", (e.api.vpu_EncRegisterFrameBuffer)(
                e.handle, fbs.as_mut_ptr(), n as c_int, w as c_int, w as c_int,
                fbs[n].buf_y, fbs[n + 1].buf_y, &mut ext))?;
            Ok(e)
        }
    }

    fn phys(&self, size: u32) -> io::Result<MemDesc> {
        let mut m = MemDesc { size: size as c_int, ..Default::default() };
        // SAFETY: m is a valid vpu_mem_desc.
        unsafe {
            if (self.api.IOGetPhyMem)(&mut m) != 0 {
                return Err(io::Error::other(format!("IOGetPhyMem({size}) failed")));
            }
            if (self.api.IOGetVirtMem)(&mut m) <= 0 {
                (self.api.IOFreePhyMem)(&mut m);
                return Err(io::Error::other("IOGetVirtMem failed"));
            }
        }
        Ok(m)
    }

    /// Copy `len` encoded bytes at physical `paddr` (inside the bitstream buffer) to `out`.
    fn emit(&self, paddr: u32, len: u32, out: &mut impl Write) -> io::Result<()> {
        let off = paddr.wrapping_sub(self.bs.phy_addr);
        if off.checked_add(len).is_none_or(|end| end > BS_SIZE) {
            return Err(io::Error::other(format!("bitstream out of range: {paddr:#x}+{len}")));
        }
        // SAFETY: range checked against the mapped bitstream buffer.
        out.write_all(unsafe { std::slice::from_raw_parts((self.bs.virt_uaddr + off) as *const u8, len as usize) })
    }

    /// SPS + PPS; write once before the first frame.
    pub fn headers(&mut self, out: &mut impl Write) -> io::Result<()> {
        for t in [SPS_RBSP, PPS_RBSP] {
            let mut hp = HeaderParam { buf: self.bs.phy_addr, header_type: t, ..Default::default() };
            // SAFETY: hp is a valid EncHeaderParam.
            rc("ENC_PUT_AVC_HEADER", unsafe { (self.api.vpu_EncGiveCommand)(self.handle, ENC_PUT_AVC_HEADER, &mut hp as *mut _ as *mut c_void) })?;
            self.emit(hp.buf, hp.size as u32, out)?;
        }
        Ok(())
    }

    /// Encode the I420 frame at physical `src` and write its NAL units to `out`.
    pub fn frame(&mut self, src: u32, out: &mut impl Write) -> io::Result<()> {
        let mut fb = FrameBuffer::i420(src, self.w, self.h, self.frames.len() as c_int);
        let mut p = EncParam { source_frame: &mut fb as *mut FrameBuffer as usize as u32, quant_param: 23, ..Default::default() };
        let mut o = OutputInfo::default();
        // SAFETY: fb outlives the encode (we wait for the VPU before returning).
        unsafe {
            rc("vpu_EncStartOneFrame", (self.api.vpu_EncStartOneFrame)(self.handle, &mut p))?;
            let mut tries = 0;
            while (self.api.vpu_IsBusy)() != 0 {
                (self.api.vpu_WaitForInt)(200);
                tries += 1;
                if tries > BUSY_TRIES {
                    return Err(io::Error::other("VPU timeout"));
                }
            }
            rc("vpu_EncGetOutputInfo", (self.api.vpu_EncGetOutputInfo)(self.handle, &mut o))?;
        }
        self.emit(o.bitstream_buffer, o.bitstream_size, out)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: releasing what open() acquired, in reverse order.
        unsafe {
            if !self.handle.is_null() {
                (self.api.vpu_EncClose)(self.handle);
            }
            for m in self.frames.iter_mut().chain(std::iter::once(&mut self.bs)) {
                if m.phy_addr != 0 {
                    (self.api.IOFreeVirtMem)(m);
                    (self.api.IOFreePhyMem)(m);
                }
            }
            (self.api.vpu_UnInit)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_match_vpu_lib_5_4_10() {
        assert_eq!(size_of::<MemDesc>(), 16);
        assert_eq!(size_of::<FrameBuffer>(), 28);
        assert_eq!(size_of::<AvcParam>(), 76);
        assert_eq!(offset_of!(OpenParam, slicemode), 52);
        assert_eq!(offset_of!(OpenParam, ring_buffer_enable), 92);
        assert_eq!(offset_of!(OpenParam, std_param), 96);
        assert_eq!(offset_of!(OpenParam, user_qp_min), 2076);
        assert_eq!(offset_of!(OpenParam, me_search_range), 2108);
        assert_eq!(size_of::<OpenParam>(), 2120);
        assert_eq!(size_of::<InitialInfo>(), 16);
        assert_eq!(size_of::<ExtBufInfo>(), 16);
        assert_eq!(size_of::<EncParam>(), 36);
        assert_eq!(offset_of!(OutputInfo, mb_info), 40);
        assert_eq!(size_of::<OutputInfo>(), 88);
        assert_eq!(size_of::<HeaderParam>(), 20);
    }

    #[test]
    fn i420_planes() {
        let f = FrameBuffer::i420(0x1000_0000, 800, 480, 3);
        assert_eq!((f.buf_y, f.buf_cb, f.buf_cr), (0x1000_0000, 0x1000_0000 + 384_000, 0x1000_0000 + 480_000));
        assert_eq!((f.stride_y, f.stride_c, f.my_index), (800, 400, 3));
    }

    #[test]
    fn avc_open_param() {
        let p = OpenParam::avc(0x2000_0000, 800, 480, 24, 4000);
        assert_eq!((p.bitstream_format, p.pic_width, p.pic_height), (STD_AVC, 800, 480));
        assert_eq!((p.frame_rate_info, p.bit_rate, p.gop_size, p.enable_auto_skip), (24, 4000, 24, 0));
        assert_eq!(p.std_param.iter().copied().max(), Some(0)); // deblocking on, everything else off
    }
}
