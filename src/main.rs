//! screen-record: CMU150 HMI screen to H.264. The IPU converts the fb0 page to I420 by DMA,
//! the VPU encodes it straight from that buffer; the CPU only paces frames and copies the bitstream.

mod opts;
mod pacer;
mod sys;
mod vpu;

use std::fs::File;
use std::io::{self, ErrorKind, Write};
use std::os::fd::AsFd;
use std::process::ExitCode;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match opts::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match run(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("screen-record: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(o: &opts::Opts) -> io::Result<()> {
    let fb = sys::Fb::open("/dev/fb0")?;
    let ipu = sys::Ipu::open()?;
    let buf = ipu.alloc((fb.w * fb.h * 3 / 2) as usize)?;
    let mut task = sys::IpuTask::csc(fb.w, fb.h, fb.page_paddr()?, buf.paddr);
    ipu.check(&task)?;
    let mut enc = vpu::Encoder::open(fb.w, fb.h, o.fps, o.bitrate / 1000)?;
    // Raw fd, not Stdout: Stdout is line-buffered and would flush on every 0x0a byte.
    let mut out = match &o.out {
        Some(p) => File::create(p)?,
        None => File::from(io::stdout().as_fd().try_clone_to_owned()?),
    };
    sys::install_signals();

    let mut result = enc.headers(&mut out);
    let start = Instant::now();
    let end = o.secs.map(|s| start + Duration::from_secs(s));
    let mut pacer = pacer::Pacer::new(o.fps, start);
    let (mut frames, mut dropped) = (0u64, 0u64);
    while result.is_ok() && !sys::STOP.load(Relaxed) {
        let (sleep, d) = pacer.next(Instant::now());
        dropped += d;
        std::thread::sleep(sleep);
        if end.is_some_and(|e| Instant::now() >= e) {
            break;
        }
        task.input.paddr = fb.page_paddr()?;
        match ipu.queue(&task).and_then(|()| enc.frame(buf.paddr, &mut out)) {
            Ok(()) => frames += 1,
            Err(e) if e.kind() == ErrorKind::BrokenPipe => break, // reader gone (ssh closed)
            Err(e) => result = Err(e),
        }
    }
    let secs = start.elapsed().as_secs_f64();
    eprintln!("frames={frames} dropped={dropped} fps={:.1}", frames as f64 / secs);
    result?;
    out.flush()
}
