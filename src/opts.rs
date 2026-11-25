//! Command line.

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
    // the VPU takes kbit/s, 1..=32767
    if !(1000..=32_767_000).contains(&o.bitrate) {
        return Err("bitrate must be 1000..32767000".into());
    }
    if let Some(p) = &o.out {
        let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(".".as_ref());
        if !dir.is_dir() {
            return Err(format!("no such directory: {}", dir.display()));
        }
    }
    Ok(o)
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
        assert!(parse(&a("-b 999")).is_err());
        assert!(parse(&a("-b 32768000")).is_err());
    }

    #[test]
    fn parse_rejects_missing_dir() {
        assert!(parse(&a("-o /nonexistent-dir-xyz/x.h264")).is_err());
    }

    #[test]
    fn parse_rejects_junk() {
        assert!(parse(&a("-h")).is_err());
        assert!(parse(&a("-t")).is_err());
        assert!(parse(&a("-t abc")).is_err());
        assert!(parse(&a("extra")).is_err());
    }

}
