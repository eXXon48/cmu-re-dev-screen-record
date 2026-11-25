//! Command line and the gst-launch pipeline built from it.

use std::path::PathBuf;

pub const USAGE: &str = "usage: screen-record [-t SECS] [-o FILE] [-b BITRATE] [-r FPS]
  records the HMI screen to H.264 (raw stream); default: stdout, until Ctrl-C
  -t SECS     stop after SECS seconds
  -o FILE     write to FILE on the unit instead of stdout
  -b BITRATE  encoder bitrate in bit/s (default 4000000)
  -r FPS      frames per second (default 24)";

#[derive(Debug, PartialEq)]
pub struct Opts {
    pub secs: Option<u64>,
    pub out: Option<PathBuf>,
    pub bitrate: u32,
    pub fps: u32,
}

pub fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { secs: None, out: None, bitrate: 4_000_000, fps: 24 };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let val = it.next().ok_or_else(|| USAGE.to_string());
        let num = |v: &String| v.parse::<u64>().map_err(|_| format!("bad number for {flag}: {v}"));
        match flag.as_str() {
            "-t" => o.secs = Some(num(val?)?),
            "-o" => o.out = Some(PathBuf::from(val?)),
            "-b" => o.bitrate = num(val?)?.try_into().map_err(|_| "bitrate too big".to_string())?,
            "-r" => o.fps = num(val?)?.try_into().map_err(|_| "fps too big".to_string())?,
            _ => return Err(USAGE.to_string()),
        }
    }
    if !(1..=60).contains(&o.fps) {
        return Err("fps must be 1..60".into());
    }
    if o.bitrate == 0 {
        return Err("bitrate must be > 0".into());
    }
    if let Some(p) = &o.out {
        let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(".".as_ref());
        if p.to_string_lossy().contains(char::is_whitespace) {
            return Err("-o path must not contain spaces".into());
        }
        if !dir.is_dir() {
            return Err(format!("no such directory: {}", dir.display()));
        }
    }
    Ok(o)
}

/// Arguments for `gst-launch-0.10`: I420 frames on stdin -> vpuenc H.264 -> stdout or file.
pub fn gst_argv(o: &Opts, w: u32, h: u32) -> Vec<String> {
    let sink = match &o.out {
        Some(p) => format!("filesink location={}", p.display()),
        None => "fdsink fd=1".into(),
    };
    let s = format!(
        "-q fdsrc blocksize={} ! video/x-raw-yuv,format=(fourcc)I420,width={w},height={h},framerate={}/1 ! vpuenc codec=6 bitrate={} ! {sink}",
        w * h * 3 / 2,
        o.fps,
        o.bitrate
    );
    // ponytail: split on spaces; parse() rejects a -o path with spaces
    s.split(' ').map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parse_defaults() {
        assert_eq!(parse(&[]).unwrap(), Opts { secs: None, out: None, bitrate: 4_000_000, fps: 24 });
    }

    #[test]
    fn parse_all_flags() {
        let o = parse(&a("-t 20 -o /tmp/x.h264 -b 2000000 -r 30")).unwrap();
        assert_eq!(o, Opts { secs: Some(20), out: Some("/tmp/x.h264".into()), bitrate: 2_000_000, fps: 30 });
    }

    #[test]
    fn parse_rejects_zero_fps() {
        assert!(parse(&a("-r 0")).is_err());
        assert!(parse(&a("-r 61")).is_err());
        assert!(parse(&a("-b 0")).is_err());
    }

    #[test]
    fn parse_rejects_missing_dir() {
        assert!(parse(&a("-o /nonexistent-dir-xyz/x.h264")).is_err());
        assert!(parse(&["-o".into(), "/tmp/a b.h264".into()]).is_err());
    }

    #[test]
    fn parse_rejects_junk() {
        assert!(parse(&a("-h")).is_err());
        assert!(parse(&a("-t")).is_err());
        assert!(parse(&a("-t abc")).is_err());
        assert!(parse(&a("extra")).is_err());
    }

    #[test]
    fn gst_argv_stdout() {
        let o = parse(&[]).unwrap();
        assert_eq!(
            gst_argv(&o, 800, 480).join(" "),
            "-q fdsrc blocksize=576000 ! video/x-raw-yuv,format=(fourcc)I420,width=800,height=480,framerate=24/1 ! vpuenc codec=6 bitrate=4000000 ! fdsink fd=1"
        );
    }

    #[test]
    fn gst_argv_file() {
        let o = parse(&a("-o /tmp/x.h264 -r 30 -b 1000000")).unwrap();
        assert_eq!(
            gst_argv(&o, 800, 480).join(" "),
            "-q fdsrc blocksize=576000 ! video/x-raw-yuv,format=(fourcc)I420,width=800,height=480,framerate=30/1 ! vpuenc codec=6 bitrate=1000000 ! filesink location=/tmp/x.h264"
        );
    }
}
