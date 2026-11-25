//! screen-record: CMU150 HMI screen to H.264. The IPU converts the fb0 page to I420 by DMA,
//! the hardware encoder (gst-launch-0.10 vpuenc) packs it; the CPU only moves the I420 bytes.

mod opts;
mod pacer;
mod sys;

use std::io::{self, ErrorKind, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};
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
        Ok(c) => c,
        Err(e) => {
            eprintln!("screen-record: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(o: &opts::Opts) -> io::Result<ExitCode> {
    let fb = sys::Fb::open("/dev/fb0")?;
    let ipu = sys::Ipu::open()?;
    let buf = ipu.alloc((fb.w * fb.h * 3 / 2) as usize)?;
    let mut task = sys::IpuTask::csc(fb.w, fb.h, fb.page_paddr()?, buf.paddr);
    ipu.check(&task)?;
    sys::install_signals();

    // Own process group: a terminal Ctrl-C reaches only us, the encoder gets a clean EOF.
    let mut child = Command::new("gst-launch-0.10")
        .args(opts::gst_argv(o, fb.w, fb.h))
        .stdin(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let mut pipe = child.stdin.take().expect("piped stdin");
    let mut grab = || {
        task.input.paddr = fb.page_paddr()?;
        ipu.queue(&task)?;
        pipe.write_all(buf.bytes())
    };

    // The first write blocks until gst-launch has loaded its plugins (can take seconds);
    // the clock starts after it, so start-up does not count as dropped frames.
    let mut result = grab();
    let start = Instant::now();
    let end = o.secs.map(|s| start + Duration::from_secs(s));
    let mut pacer = pacer::Pacer::new(o.fps, start);
    pacer.next(start);
    let (mut frames, mut dropped) = (1u64, 0u64);
    while result.is_ok() && !sys::STOP.load(Relaxed) {
        let (sleep, d) = pacer.next(Instant::now());
        dropped += d;
        std::thread::sleep(sleep);
        if end.is_some_and(|e| Instant::now() >= e) {
            break;
        }
        match grab() {
            Ok(()) => frames += 1,
            Err(e) if e.kind() == ErrorKind::BrokenPipe => break, // reader gone (ssh closed)
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    drop(pipe); // EOF -> encoder flushes and exits
    let st = child.wait()?;
    eprintln!("frames={frames} dropped={dropped} fps={:.1}", frames as f64 / secs);
    result?;
    Ok(if st.success() { ExitCode::SUCCESS } else { ExitCode::from(st.code().unwrap_or(1) as u8) })
}
